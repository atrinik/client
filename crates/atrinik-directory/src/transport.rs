//! Fixed-origin HTTPS transport for static directory retrieval.

use crate::cache::valid_strong_etag;
use atrinik_protocol_adapter::directory::DIRECTORY_BODY_BYTES_LIMIT;
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::io::Read;
use std::time::Duration;
use ureq::Agent;

pub const DIRECTORY_URL: &str = "https://meta.atrinik.org/index.json";
pub const DIRECTORY_MEDIA_TYPE: &str = "application/json; charset=utf-8";
pub const DIRECTORY_USER_AGENT: &str = concat!("atrinik-client/", env!("CARGO_PKG_VERSION"));
const MAXIMUM_RESPONSE_HEADER_BYTES: usize = 8 * 1024;
const MAXIMUM_TRANSFER_BODY_BYTES: u64 = (DIRECTORY_BODY_BYTES_LIMIT + 8 * 1024) as u64;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DirectoryRequest {
    pub url: &'static str,
    pub accept: &'static str,
    pub cache_control: &'static str,
    pub if_none_match: Option<String>,
}

impl DirectoryRequest {
    pub fn new(if_none_match: Option<String>) -> Self {
        Self {
            url: DIRECTORY_URL,
            accept: DIRECTORY_MEDIA_TYPE,
            cache_control: "no-cache",
            if_none_match,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DirectoryResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl DirectoryResponse {
    #[must_use]
    pub fn header_values(&self, name: &str) -> Vec<&str> {
        self.headers
            .iter()
            .filter(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
            .collect()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DirectoryTransportError {
    Offline,
    Timeout,
    Tls,
    Protocol,
    BodyTooLarge,
}

impl Display for DirectoryTransportError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Offline => "directory transport is offline",
            Self::Timeout => "directory request timed out",
            Self::Tls => "directory TLS validation failed",
            Self::Protocol => "directory HTTP response is invalid",
            Self::BodyTooLarge => "directory response exceeds its byte limit",
        })
    }
}

impl Error for DirectoryTransportError {}

pub trait DirectoryTransport {
    fn fetch(
        &mut self,
        request: &DirectoryRequest,
    ) -> Result<DirectoryResponse, DirectoryTransportError>;
}

#[derive(Clone, Debug)]
pub struct UreqDirectoryTransport {
    agent: Agent,
}

impl Default for UreqDirectoryTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl UreqDirectoryTransport {
    #[must_use]
    pub fn new() -> Self {
        let config = Agent::config_builder()
            .https_only(true)
            .http_status_as_error(false)
            .max_redirects(0)
            .max_response_header_size(MAXIMUM_RESPONSE_HEADER_BYTES)
            .timeout_global(Some(REQUEST_TIMEOUT))
            .timeout_connect(Some(CONNECT_TIMEOUT))
            .user_agent(DIRECTORY_USER_AGENT)
            .build();
        Self {
            agent: config.new_agent(),
        }
    }
}

impl DirectoryTransport for UreqDirectoryTransport {
    fn fetch(
        &mut self,
        request: &DirectoryRequest,
    ) -> Result<DirectoryResponse, DirectoryTransportError> {
        if request.url != DIRECTORY_URL
            || request.accept != DIRECTORY_MEDIA_TYPE
            || request.cache_control != "no-cache"
            || request
                .if_none_match
                .as_deref()
                .is_some_and(|etag| !valid_strong_etag(etag))
        {
            return Err(DirectoryTransportError::Protocol);
        }
        let mut outbound = self
            .agent
            .get(request.url)
            .header("Accept", request.accept)
            .header("Cache-Control", request.cache_control);
        if let Some(etag) = &request.if_none_match {
            outbound = outbound.header("If-None-Match", etag);
        }
        let mut response = outbound
            .call()
            .map_err(|error| classify_ureq_error(&error))?;
        let status = response.status().as_u16();
        let headers = selected_headers(response.headers())?;
        let body = if matches!(status, 200 | 304) {
            read_bounded_body(&mut response)?
        } else {
            Vec::new()
        };
        Ok(DirectoryResponse {
            status,
            headers,
            body,
        })
    }
}

fn selected_headers(
    headers: &ureq::http::HeaderMap,
) -> Result<Vec<(String, String)>, DirectoryTransportError> {
    const SELECTED: &[&str] = &[
        "content-encoding",
        "content-length",
        "content-type",
        "etag",
        "last-modified",
        "retry-after",
    ];
    let mut output = Vec::new();
    for (name, value) in headers {
        if SELECTED.contains(&name.as_str()) {
            output.push((
                name.as_str().to_owned(),
                value
                    .to_str()
                    .map_err(|_| DirectoryTransportError::Protocol)?
                    .to_owned(),
            ));
        }
    }
    Ok(output)
}

fn read_bounded_body(
    response: &mut ureq::http::Response<ureq::Body>,
) -> Result<Vec<u8>, DirectoryTransportError> {
    let mut reader = response
        .body_mut()
        .with_config()
        .limit(MAXIMUM_TRANSFER_BODY_BYTES)
        .reader();
    let mut output = Vec::new();
    let mut buffer = [0u8; 8 * 1024];
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|error| classify_body_error(&error))?;
        if read == 0 {
            return Ok(output);
        }
        let next = output
            .len()
            .checked_add(read)
            .ok_or(DirectoryTransportError::BodyTooLarge)?;
        if next > DIRECTORY_BODY_BYTES_LIMIT {
            return Err(DirectoryTransportError::BodyTooLarge);
        }
        output.extend_from_slice(&buffer[..read]);
    }
}

fn classify_body_error(error: &std::io::Error) -> DirectoryTransportError {
    if error.kind() == std::io::ErrorKind::TimedOut {
        return DirectoryTransportError::Timeout;
    }
    error
        .get_ref()
        .and_then(|source| source.downcast_ref::<ureq::Error>())
        .map_or(DirectoryTransportError::Offline, classify_ureq_error)
}

fn classify_ureq_error(error: &ureq::Error) -> DirectoryTransportError {
    match error {
        ureq::Error::Timeout(_) => DirectoryTransportError::Timeout,
        ureq::Error::HostNotFound | ureq::Error::ConnectionFailed | ureq::Error::Io(_) => {
            DirectoryTransportError::Offline
        }
        ureq::Error::Tls(_) | ureq::Error::Rustls(_) => DirectoryTransportError::Tls,
        ureq::Error::BodyExceedsLimit(_) | ureq::Error::LargeResponseHeader(_, _) => {
            DirectoryTransportError::BodyTooLarge
        }
        _ => DirectoryTransportError::Protocol,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ureq::Body;
    use ureq::http::{HeaderMap, HeaderValue, Response};

    #[test]
    fn request_contract_is_fixed_and_tampering_fails_before_network() {
        assert_eq!(
            DirectoryRequest::new(Some("\"etag\"".to_owned())),
            DirectoryRequest {
                url: "https://meta.atrinik.org/index.json",
                accept: "application/json; charset=utf-8",
                cache_control: "no-cache",
                if_none_match: Some("\"etag\"".to_owned()),
            }
        );
        let mut transport = UreqDirectoryTransport::new();
        let invalid = DirectoryRequest {
            url: "https://example.invalid/directory",
            accept: DIRECTORY_MEDIA_TYPE,
            cache_control: "no-cache",
            if_none_match: None,
        };
        assert_eq!(
            transport.fetch(&invalid),
            Err(DirectoryTransportError::Protocol)
        );

        let invalid_validator = DirectoryRequest {
            url: DIRECTORY_URL,
            accept: DIRECTORY_MEDIA_TYPE,
            cache_control: "no-cache",
            if_none_match: Some("W/\"weak\"".to_owned()),
        };
        assert_eq!(
            transport.fetch(&invalid_validator),
            Err(DirectoryTransportError::Protocol)
        );
    }

    #[test]
    fn selected_metadata_is_allowlisted_and_invalid_values_fail_closed() {
        let mut headers = HeaderMap::new();
        headers.append("etag", HeaderValue::from_static("\"one\""));
        headers.append("etag", HeaderValue::from_static("\"two\""));
        headers.insert("set-cookie", HeaderValue::from_static("secret=value"));
        assert_eq!(
            selected_headers(&headers).expect("selected"),
            vec![
                ("etag".to_owned(), "\"one\"".to_owned()),
                ("etag".to_owned(), "\"two\"".to_owned()),
            ]
        );

        let mut invalid = HeaderMap::new();
        invalid.insert(
            "etag",
            HeaderValue::from_bytes(&[0xff]).expect("opaque header"),
        );
        assert_eq!(
            selected_headers(&invalid),
            Err(DirectoryTransportError::Protocol)
        );
    }

    #[test]
    fn decoded_body_limit_is_enforced_independently_of_http_metadata() {
        let exact = vec![b'a'; DIRECTORY_BODY_BYTES_LIMIT];
        let mut response = Response::builder()
            .status(200)
            .body(Body::builder().data(exact.clone()))
            .expect("response");
        assert_eq!(read_bounded_body(&mut response), Ok(exact));

        let oversized = vec![b'a'; DIRECTORY_BODY_BYTES_LIMIT + 1];
        let mut response = Response::builder()
            .status(200)
            .body(Body::builder().data(oversized))
            .expect("response");
        assert_eq!(
            read_bounded_body(&mut response),
            Err(DirectoryTransportError::BodyTooLarge)
        );
    }
}
