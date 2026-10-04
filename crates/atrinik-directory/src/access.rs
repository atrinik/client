//! Ephemeral access-code handling for private discovery and game admission.

use atrinik_protocol::metaserver::access::{
    AccessEndpoint, AccessProfile, is_access_unavailable, marshal_access_resolve_request,
    parse_access_resolved,
};
use atrinik_protocol_adapter::trust::{CertificateIdentities, verify_certificate_identity};
use sha2::{Digest, Sha256};
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::io::Read;
use std::time::Duration;
use ureq::Agent;
use zeroize::Zeroize;

pub const ACCESS_CODE_BYTES: usize = 16;
pub const ACCESS_RESOLVE_REQUEST_BYTES: usize = 204;
pub const ACCESS_RESOLVE_RESPONSE_BYTES_LIMIT: usize = 8 * 1024;
pub const ACCESS_RESOLVE_URL: &str = "https://rendezvous.meta.atrinik.org/v1/access/resolve";
const ACCESS_MEDIA_TYPE: &str = "application/json; charset=utf-8";
const MAXIMUM_RESPONSE_HEADER_BYTES: usize = 8 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const ACCESS_ALPHABET: &[u8] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const ROUTE_DOMAIN: &[u8] = b"atrinik-access-route-v1\0";

/// A canonical access code retained only for the lifetime of one connection attempt.
///
/// Deliberately does not implement `Debug`, `Display`, `Clone`, or serialization.
pub struct AccessCode([u8; ACCESS_CODE_BYTES]);

impl AccessCode {
    pub fn parse_user_input(input: &str) -> Result<Self, AccessCodeError> {
        let trimmed = input.trim_matches(|value: char| value.is_ascii_whitespace());
        if !trimmed.is_ascii() || trimmed.len() != ACCESS_CODE_BYTES {
            return Err(AccessCodeError::Invalid);
        }
        let mut canonical = [0u8; ACCESS_CODE_BYTES];
        for (output, input) in canonical.iter_mut().zip(trimmed.bytes()) {
            let value = input.to_ascii_uppercase();
            if !ACCESS_ALPHABET.contains(&value) {
                canonical.zeroize();
                return Err(AccessCodeError::Invalid);
            }
            *output = value;
        }
        Ok(Self(canonical))
    }

    #[must_use]
    pub fn route_capability(&self) -> RouteCapability {
        let mut hasher = Sha256::new();
        hasher.update(ROUTE_DOMAIN);
        hasher.update(self.0);
        RouteCapability(hasher.finalize().into())
    }

    /// Exposes the canonical bytes only to the encrypted GP1 access adapter.
    pub fn with_canonical_bytes<T>(&self, use_bytes: impl FnOnce(&[u8; 16]) -> T) -> T {
        use_bytes(&self.0)
    }
}

impl Drop for AccessCode {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccessCodeError {
    Invalid,
}

impl Display for AccessCodeError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("access code is invalid")
    }
}

impl Error for AccessCodeError {}

/// A discovery capability derived from an access code.
///
/// This value remains private even though it cannot authenticate a game connection.
pub struct RouteCapability([u8; 32]);

impl Drop for RouteCapability {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

/// A fixed-shape access resolve request. It deliberately has no logging traits.
pub struct AccessResolveRequest {
    body: Vec<u8>,
    client_nonce: [u8; 32],
}

impl AccessResolveRequest {
    #[must_use]
    pub fn new(route: &RouteCapability, client_nonce: [u8; 32]) -> Self {
        let body = marshal_access_resolve_request(&route.0, &client_nonce);
        debug_assert_eq!(body.len(), ACCESS_RESOLVE_REQUEST_BYTES);
        Self { body, client_nonce }
    }

    pub(crate) fn body(&self) -> &[u8] {
        &self.body
    }

    fn client_nonce(&self) -> &[u8; 32] {
        &self.client_nonce
    }
}

impl Drop for AccessResolveRequest {
    fn drop(&mut self) {
        self.body.zeroize();
        self.client_nonce.zeroize();
    }
}

/// A bounded access response. Its grant and private routing data must not be logged.
pub struct AccessResolveResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl AccessResolveResponse {
    #[must_use]
    pub fn header_values(&self, name: &str) -> Vec<&str> {
        self.headers
            .iter()
            .filter(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
            .collect()
    }
}

impl Drop for AccessResolveResponse {
    fn drop(&mut self) {
        self.body.zeroize();
    }
}

/// A successful private-discovery result whose one-use grant is never printable.
pub struct ResolvedAccess {
    pub server_id: [u8; 32],
    pub certificate_der: Vec<u8>,
    pub certificate_identities: CertificateIdentities,
    pub name: String,
    pub generation: [u8; 32],
    pub expires_at: u64,
    pub endpoint: Option<AccessEndpoint>,
    grant: [u8; 32],
}

impl ResolvedAccess {
    /// Exposes the grant only to the existing rendezvous adapter for one attempt.
    pub fn with_grant<T>(&self, use_grant: impl FnOnce(&[u8; 32]) -> T) -> T {
        use_grant(&self.grant)
    }
}

impl Drop for ResolvedAccess {
    fn drop(&mut self) {
        self.certificate_der.zeroize();
        self.generation.zeroize();
        self.grant.zeroize();
    }
}

pub enum AccessResolution {
    Resolved(Box<ResolvedAccess>),
    Unavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccessResponseError {
    Protocol,
    Identity,
}

impl Display for AccessResponseError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Protocol => "access service response is invalid",
            Self::Identity => "access service returned an untrusted server identity",
        })
    }
}

impl Error for AccessResponseError {}

/// Validates one uncached response and binds its certificate and grant to this request.
pub fn validate_access_response(
    response: &AccessResolveResponse,
    request: &AccessResolveRequest,
    now: u64,
    expected_server_id: Option<&[u8; 32]>,
) -> Result<AccessResolution, AccessResponseError> {
    validate_response_headers(response)?;
    if response.status == 404 {
        return if is_access_unavailable(&response.body) {
            Ok(AccessResolution::Unavailable)
        } else {
            Err(AccessResponseError::Protocol)
        };
    }
    if response.status != 200 {
        return Err(AccessResponseError::Protocol);
    }

    let mut resolved = parse_access_resolved(&response.body, request.client_nonce(), now)
        .map_err(|_| AccessResponseError::Protocol)?;
    if resolved.profile != AccessProfile::Game
        || expected_server_id.is_some_and(|expected| expected != &resolved.server_id)
    {
        clear_resolved_secrets(&mut resolved);
        return Err(AccessResponseError::Identity);
    }
    let Ok(certificate_identities) =
        verify_certificate_identity(&resolved.certificate_der, &resolved.server_id)
    else {
        clear_resolved_secrets(&mut resolved);
        return Err(AccessResponseError::Identity);
    };

    let output = ResolvedAccess {
        server_id: resolved.server_id,
        certificate_der: std::mem::take(&mut resolved.certificate_der),
        certificate_identities,
        name: std::mem::take(&mut resolved.name),
        generation: std::mem::take(&mut resolved.generation),
        expires_at: resolved.expires_at,
        endpoint: resolved.endpoint.take(),
        grant: std::mem::take(&mut resolved.grant),
    };
    clear_resolved_secrets(&mut resolved);
    Ok(AccessResolution::Resolved(Box::new(output)))
}

fn validate_response_headers(response: &AccessResolveResponse) -> Result<(), AccessResponseError> {
    if response.header_values("cache-control") != ["no-store"]
        || response.header_values("content-type") != [ACCESS_MEDIA_TYPE]
    {
        return Err(AccessResponseError::Protocol);
    }
    let content_lengths = response.header_values("content-length");
    if content_lengths.len() > 1
        || content_lengths
            .first()
            .is_some_and(|value| value.parse::<usize>().ok() != Some(response.body.len()))
    {
        return Err(AccessResponseError::Protocol);
    }
    Ok(())
}

fn clear_resolved_secrets(resolved: &mut atrinik_protocol::metaserver::access::AccessResolved) {
    resolved.certificate_der.zeroize();
    resolved.generation.zeroize();
    resolved.client_nonce.zeroize();
    resolved.grant.zeroize();
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccessTransportError {
    Offline,
    Timeout,
    Tls,
    Protocol,
    BodyTooLarge,
}

impl Display for AccessTransportError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Offline => "access service is offline",
            Self::Timeout => "access request timed out",
            Self::Tls => "access service TLS validation failed",
            Self::Protocol => "access service response is invalid",
            Self::BodyTooLarge => "access service response exceeds its byte limit",
        })
    }
}

impl Error for AccessTransportError {}

pub trait AccessTransport {
    fn resolve(
        &mut self,
        request: &AccessResolveRequest,
    ) -> Result<AccessResolveResponse, AccessTransportError>;
}

#[derive(Clone)]
pub struct UreqAccessTransport {
    agent: Agent,
}

impl Default for UreqAccessTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl UreqAccessTransport {
    #[must_use]
    pub fn new() -> Self {
        let config = Agent::config_builder()
            .https_only(true)
            .http_status_as_error(false)
            .max_redirects(0)
            .max_response_header_size(MAXIMUM_RESPONSE_HEADER_BYTES)
            .timeout_global(Some(REQUEST_TIMEOUT))
            .timeout_connect(Some(CONNECT_TIMEOUT))
            .user_agent(concat!("atrinik-client/", env!("CARGO_PKG_VERSION")))
            .build();
        Self {
            agent: config.new_agent(),
        }
    }
}

impl AccessTransport for UreqAccessTransport {
    fn resolve(
        &mut self,
        request: &AccessResolveRequest,
    ) -> Result<AccessResolveResponse, AccessTransportError> {
        if request.body().len() != ACCESS_RESOLVE_REQUEST_BYTES {
            return Err(AccessTransportError::Protocol);
        }
        let mut response = self
            .agent
            .post(ACCESS_RESOLVE_URL)
            .header("Accept", ACCESS_MEDIA_TYPE)
            .header("Content-Type", ACCESS_MEDIA_TYPE)
            .header("Cache-Control", "no-store")
            .send(request.body())
            .map_err(|error| classify_ureq_error(&error))?;
        let status = response.status().as_u16();
        let headers = selected_headers(response.headers())?;
        let body = read_bounded_body(&mut response)?;
        Ok(AccessResolveResponse {
            status,
            headers,
            body,
        })
    }
}

fn selected_headers(
    headers: &ureq::http::HeaderMap,
) -> Result<Vec<(String, String)>, AccessTransportError> {
    const SELECTED: &[&str] = &["cache-control", "content-length", "content-type"];
    let mut output = Vec::new();
    for (name, value) in headers {
        if matches!(name.as_str(), "location" | "set-cookie") {
            return Err(AccessTransportError::Protocol);
        }
        if SELECTED.contains(&name.as_str()) {
            output.push((
                name.as_str().to_owned(),
                value
                    .to_str()
                    .map_err(|_| AccessTransportError::Protocol)?
                    .to_owned(),
            ));
        }
    }
    Ok(output)
}

fn read_bounded_body(
    response: &mut ureq::http::Response<ureq::Body>,
) -> Result<Vec<u8>, AccessTransportError> {
    let mut reader = response
        .body_mut()
        .with_config()
        .limit((ACCESS_RESOLVE_RESPONSE_BYTES_LIMIT + 1) as u64)
        .reader();
    let mut output = Vec::new();
    let mut buffer = [0u8; 8 * 1024];
    loop {
        let read = match reader.read(&mut buffer) {
            Ok(read) => read,
            Err(error) => {
                output.zeroize();
                buffer.zeroize();
                return Err(classify_body_error(&error));
            }
        };
        if read == 0 {
            buffer.zeroize();
            return Ok(output);
        }
        let next = output
            .len()
            .checked_add(read)
            .ok_or(AccessTransportError::BodyTooLarge)?;
        if next > ACCESS_RESOLVE_RESPONSE_BYTES_LIMIT {
            output.zeroize();
            buffer.zeroize();
            return Err(AccessTransportError::BodyTooLarge);
        }
        output.extend_from_slice(&buffer[..read]);
    }
}

fn classify_body_error(error: &std::io::Error) -> AccessTransportError {
    if error.kind() == std::io::ErrorKind::TimedOut {
        return AccessTransportError::Timeout;
    }
    error
        .get_ref()
        .and_then(|source| source.downcast_ref::<ureq::Error>())
        .map_or(AccessTransportError::Offline, classify_ureq_error)
}

fn classify_ureq_error(error: &ureq::Error) -> AccessTransportError {
    match error {
        ureq::Error::Timeout(_) => AccessTransportError::Timeout,
        ureq::Error::HostNotFound | ureq::Error::ConnectionFailed | ureq::Error::Io(_) => {
            AccessTransportError::Offline
        }
        ureq::Error::Tls(_) | ureq::Error::Rustls(_) => AccessTransportError::Tls,
        ureq::Error::BodyExceedsLimit(_) | ureq::Error::LargeResponseHeader(_, _) => {
            AccessTransportError::BodyTooLarge
        }
        _ => AccessTransportError::Protocol,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ureq::Body;
    use ureq::http::{HeaderMap, HeaderValue, Response};

    const CANONICAL_RESOLVED: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/access-resolve-v1/canonical.json"
    ));

    #[test]
    fn user_input_normalizes_only_outer_ascii_space_and_case() {
        let code = AccessCode::parse_user_input(" \t01abcdefghjkmnpq\r\n").expect("valid code");
        assert_eq!(
            code.with_canonical_bytes(|value| *value),
            *b"01ABCDEFGHJKMNPQ"
        );
        for invalid in [
            "01ABCDEFGHJKMNP",
            "01ABCDEFGHJKMNPQR",
            "01ABC-DEFGHJKMNP",
            "01AB CDEFGHJKMNP",
            "01ABCDEFGHJKLMNO",
            "01ABCDEFGHJKLMNI",
            "01ABCDEFGHJKLＭＮ",
            "\u{2003}01ABCDEFGHJKMNPQ",
        ] {
            assert!(
                AccessCode::parse_user_input(invalid).is_err(),
                "{invalid:?}"
            );
        }
    }

    #[test]
    fn route_hash_and_request_match_the_language_neutral_formula() {
        assert_eq!(
            ACCESS_RESOLVE_URL,
            "https://rendezvous.meta.atrinik.org/v1/access/resolve"
        );
        let code = AccessCode::parse_user_input("0123456789ABCDEF").expect("valid code");
        let route = code.route_capability();
        let nonce = [0x5a; 32];
        let request = AccessResolveRequest::new(&route, nonce);
        assert_eq!(request.body().len(), ACCESS_RESOLVE_REQUEST_BYTES);
        assert_eq!(
            std::str::from_utf8(request.body()).expect("JSON is ASCII"),
            "{\"schema\":\"atrinik-access-resolve-v1\",\"routeCapability\":\"3cc820e9a4e884e9515c8f8b211d1fdfaeb75afa1319fd1a1728a050ebe1c60e\",\"clientNonce\":\"5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a\"}"
        );
    }

    #[test]
    fn response_metadata_rejects_redirect_and_cookie_channels() {
        let mut headers = HeaderMap::new();
        headers.insert("cache-control", HeaderValue::from_static("no-store"));
        headers.insert("content-type", HeaderValue::from_static(ACCESS_MEDIA_TYPE));
        assert_eq!(
            selected_headers(&headers).expect("selected"),
            vec![
                ("cache-control".to_owned(), "no-store".to_owned()),
                ("content-type".to_owned(), ACCESS_MEDIA_TYPE.to_owned()),
            ]
        );
        for name in ["location", "set-cookie"] {
            let mut invalid = headers.clone();
            invalid.insert(name, HeaderValue::from_static("forbidden"));
            assert_eq!(
                selected_headers(&invalid),
                Err(AccessTransportError::Protocol)
            );
        }
    }

    #[test]
    fn response_body_limit_is_enforced_before_parsing_private_data() {
        let exact = vec![b'a'; ACCESS_RESOLVE_RESPONSE_BYTES_LIMIT];
        let mut response = Response::builder()
            .status(200)
            .body(Body::builder().data(exact.clone()))
            .expect("response");
        assert_eq!(read_bounded_body(&mut response), Ok(exact));

        let oversized = vec![b'a'; ACCESS_RESOLVE_RESPONSE_BYTES_LIMIT + 1];
        let mut response = Response::builder()
            .status(200)
            .body(Body::builder().data(oversized))
            .expect("response");
        assert_eq!(
            read_bounded_body(&mut response),
            Err(AccessTransportError::BodyTooLarge)
        );
    }

    #[test]
    fn unavailable_response_has_one_exact_uncached_shape() {
        let code = AccessCode::parse_user_input("0123456789ABCDEF").expect("code");
        let request = AccessResolveRequest::new(&code.route_capability(), [0x5a; 32]);
        let response = AccessResolveResponse {
            status: 404,
            headers: vec![
                ("cache-control".to_owned(), "no-store".to_owned()),
                ("content-type".to_owned(), ACCESS_MEDIA_TYPE.to_owned()),
            ],
            body: br#"{"error":{"code":"access_unavailable"}}"#.to_vec(),
        };
        assert!(matches!(
            validate_access_response(&response, &request, 1_000, None),
            Ok(AccessResolution::Unavailable)
        ));

        for invalid in [
            AccessResolveResponse {
                status: 404,
                headers: response.headers.clone(),
                body: br#"{"error":{"code":"access_expired"}}"#.to_vec(),
            },
            AccessResolveResponse {
                status: 404,
                headers: vec![("content-type".to_owned(), ACCESS_MEDIA_TYPE.to_owned())],
                body: response.body.clone(),
            },
        ] {
            assert!(matches!(
                validate_access_response(&invalid, &request, 1_000, None),
                Err(AccessResponseError::Protocol)
            ));
        }
    }

    #[test]
    fn resolved_response_binds_nonce_leaf_identity_and_gp1_spki_pin() {
        let code = AccessCode::parse_user_input("0123456789ABCDEF").expect("code");
        let request = AccessResolveRequest::new(&code.route_capability(), [0x22; 32]);
        let response = AccessResolveResponse {
            status: 200,
            headers: vec![
                ("cache-control".to_owned(), "no-store".to_owned()),
                ("content-type".to_owned(), ACCESS_MEDIA_TYPE.to_owned()),
                (
                    "content-length".to_owned(),
                    CANONICAL_RESOLVED.len().to_string(),
                ),
            ],
            body: CANONICAL_RESOLVED.to_vec(),
        };
        let expected_server_id = [
            0x0d, 0x61, 0xda, 0xe9, 0x42, 0x26, 0xa6, 0x8c, 0x24, 0x52, 0x59, 0x88, 0x98, 0xd3,
            0x3e, 0xf8, 0xeb, 0x97, 0xa7, 0x3a, 0x04, 0x02, 0x94, 0x82, 0x5c, 0x2e, 0xed, 0xb0,
            0x1d, 0x6a, 0xee, 0x40,
        ];
        let AccessResolution::Resolved(resolved) =
            validate_access_response(&response, &request, 1_000, Some(&expected_server_id))
                .expect("resolved")
        else {
            panic!("unexpected unavailable response");
        };
        assert_eq!(resolved.server_id, expected_server_id);
        assert_eq!(resolved.generation, [0x11; 32]);
        assert_eq!(resolved.with_grant(|grant| *grant), [0x33; 32]);
        assert_eq!(
            resolved.certificate_identities.gp1_spki_pin,
            [
                0x5c, 0xd2, 0x52, 0xfb, 0x0c, 0xe8, 0x93, 0x24, 0x36, 0xfa, 0xf8, 0xcc, 0xd1, 0x04,
                0x09, 0x81, 0xb8, 0x9e, 0xe4, 0xad, 0x6b, 0x9f, 0xe9, 0xe2, 0xa2, 0xb7, 0xe7, 0x1a,
                0xac, 0xb2, 0x7c, 0xd3,
            ]
        );
        assert!(matches!(
            validate_access_response(&response, &request, 1_000, Some(&[0; 32])),
            Err(AccessResponseError::Identity)
        ));
    }
}
