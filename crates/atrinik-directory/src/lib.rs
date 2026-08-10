#![forbid(unsafe_code)]
//! Fixed-origin, bounded, transactional Game Protocol 1 server discovery.

pub mod cache;
pub mod transport;

use atrinik_protocol_adapter::directory::{
    DIRECTORY_CLOCK_SKEW_SECONDS, DirectEndpoint, DirectoryServerStatus, DirectorySnapshot,
    DirectoryValidationError, InstalledCompatibility, parse_directory,
};
use cache::{CachedDirectory, DirectoryCache, DirectoryCacheError};
use httpdate::parse_http_date;
use sha2::{Digest, Sha256};
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::net::IpAddr;
use std::time::UNIX_EPOCH;
use transport::{
    DIRECTORY_MEDIA_TYPE, DirectoryRequest, DirectoryResponse, DirectoryTransport,
    DirectoryTransportError,
};

pub use transport::DIRECTORY_URL;

pub const TRUSTED_RENDEZVOUS_ORIGIN: &str = "wss://rendezvous.meta.atrinik.org";
const MAXIMUM_STALE_DISPLAY_SECONDS: u64 = 24 * 60 * 60;
const MAXIMUM_RETRY_AFTER_SECONDS: u64 = 24 * 60 * 60;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DirectoryAvailability {
    Fresh,
    Empty,
    NoCompatibleServers,
    Stale,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DirectorySource {
    Network,
    Revalidated,
    Cache,
    None,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DirectoryFailure {
    Offline,
    Timeout,
    Tls,
    RateLimited { retry_after_seconds: u64 },
    HttpUnavailable,
    InvalidMetadata,
    IntegrityMismatch,
    InvalidDirectory(DirectoryValidationError),
    CacheUnavailable,
    CacheCorrupt,
    CacheWriteFailed,
}

impl DirectoryFailure {
    pub const fn code(self) -> &'static str {
        match self {
            Self::Offline => "offline",
            Self::Timeout => "timeout",
            Self::Tls => "tls",
            Self::RateLimited { .. } => "rate_limited",
            Self::HttpUnavailable => "http_unavailable",
            Self::InvalidMetadata => "invalid_metadata",
            Self::IntegrityMismatch => "integrity_mismatch",
            Self::InvalidDirectory(error) => error.code(),
            Self::CacheUnavailable => "cache_unavailable",
            Self::CacheCorrupt => "cache_corrupt",
            Self::CacheWriteFailed => "cache_write_failed",
        }
    }
}

impl Display for DirectoryFailure {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl Error for DirectoryFailure {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DirectoryView {
    pub availability: DirectoryAvailability,
    pub source: DirectorySource,
    pub notice: Option<DirectoryFailure>,
    pub received_at: Option<u64>,
    pub snapshot: Option<DirectorySnapshot>,
}

impl DirectoryView {
    #[must_use]
    pub const fn message(&self) -> &'static str {
        match self.availability {
            DirectoryAvailability::Fresh => "Server directory is current",
            DirectoryAvailability::Empty => "No public servers are currently listed",
            DirectoryAvailability::NoCompatibleServers => {
                "No listed server matches the installed protocol and content"
            }
            DirectoryAvailability::Stale => "Showing stale server directory data",
            DirectoryAvailability::Unavailable => "Server directory is unavailable",
        }
    }

    pub fn connection_plan(
        &self,
        server_id: &[u8; 32],
        now: u64,
        rendezvous: RendezvousSupport,
    ) -> Result<DiscoveredConnectionPlan, ConnectionPlanError> {
        let snapshot = self
            .snapshot
            .as_ref()
            .ok_or(ConnectionPlanError::DirectoryUnavailable)?;
        if !snapshot.fresh_at(now) {
            return Err(ConnectionPlanError::RefreshRequired);
        }
        let server = snapshot
            .servers
            .iter()
            .find(|server| &server.server_id == server_id)
            .ok_or(ConnectionPlanError::ServerMissing)?;
        match server.status {
            DirectoryServerStatus::Online => {}
            DirectoryServerStatus::Full => return Err(ConnectionPlanError::ServerFull),
            DirectoryServerStatus::Maintenance => {
                return Err(ConnectionPlanError::ServerMaintenance);
            }
        }
        let rendezvous_url = match rendezvous {
            RendezvousSupport::Disabled => None,
            RendezvousSupport::FixedV1 => Some(format!(
                "{TRUSTED_RENDEZVOUS_ORIGIN}/v1/servers/{}?role=client",
                lowercase_hex(&server.server_id)
            )),
        };
        if server.endpoint.is_none() && rendezvous_url.is_none() {
            return Err(ConnectionPlanError::NoRoute);
        }
        Ok(DiscoveredConnectionPlan {
            server_id: server.server_id,
            certificate_sha256: server.certificate_sha256,
            password_required: server.password_required,
            direct_endpoint: server.endpoint.clone(),
            rendezvous_url,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RendezvousSupport {
    Disabled,
    FixedV1,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiscoveredConnectionPlan {
    pub server_id: [u8; 32],
    pub certificate_sha256: [u8; 32],
    pub password_required: bool,
    pub direct_endpoint: Option<DirectEndpoint>,
    pub rendezvous_url: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfiguredConnectionPlan {
    pub address: IpAddr,
    pub port: u16,
    pub certificate_sha256: [u8; 32],
}

impl ConfiguredConnectionPlan {
    pub fn for_ip(
        address: IpAddr,
        port: u16,
        certificate_sha256: [u8; 32],
    ) -> Result<Self, ConnectionPlanError> {
        if port == 0 {
            return Err(ConnectionPlanError::InvalidConfiguredEndpoint);
        }
        Ok(Self {
            address,
            port,
            certificate_sha256,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConnectionPlanError {
    DirectoryUnavailable,
    RefreshRequired,
    ServerMissing,
    ServerFull,
    ServerMaintenance,
    NoRoute,
    InvalidConfiguredEndpoint,
}

impl Display for ConnectionPlanError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::DirectoryUnavailable => "server directory is unavailable",
            Self::RefreshRequired => "server directory must be refreshed before connecting",
            Self::ServerMissing => "server is not present in the current directory",
            Self::ServerFull => "server is full",
            Self::ServerMaintenance => "server is in maintenance",
            Self::NoRoute => "server has no supported route",
            Self::InvalidConfiguredEndpoint => "configured direct endpoint is invalid",
        })
    }
}

impl Error for ConnectionPlanError {}

pub struct DirectoryService<T, C> {
    transport: T,
    cache: C,
    compatibility: InstalledCompatibility,
}

impl<T, C> DirectoryService<T, C>
where
    T: DirectoryTransport,
    C: DirectoryCache,
{
    pub const fn new(transport: T, cache: C, compatibility: InstalledCompatibility) -> Self {
        Self {
            transport,
            cache,
            compatibility,
        }
    }

    pub fn refresh(&mut self, now: u64) -> DirectoryView {
        let (cached, cache_notice) = self.load_cache(now);
        let request = DirectoryRequest::new(cached.as_ref().map(|value| value.record.etag.clone()));
        let outcome = self
            .transport
            .fetch(&request)
            .map_err(DirectoryFailure::from)
            .and_then(|response| self.process_response(response, cached.as_ref(), now));
        match outcome {
            Ok(current) => {
                let store_notice = self
                    .cache
                    .store(&current.record)
                    .err()
                    .map(|_| DirectoryFailure::CacheWriteFailed);
                let source = current.source;
                let record = current.record;
                let snapshot = current.snapshot;
                let notice = store_notice.or(cache_notice);
                view_from_snapshot(snapshot, record.received_at, source, notice, now)
            }
            Err(failure) => fallback_view(cached, failure, now),
        }
    }

    pub fn into_parts(self) -> (T, C, InstalledCompatibility) {
        (self.transport, self.cache, self.compatibility)
    }

    fn load_cache(&mut self, now: u64) -> (Option<ValidatedRecord>, Option<DirectoryFailure>) {
        let Ok(read) = self.cache.load() else {
            return (None, Some(DirectoryFailure::CacheUnavailable));
        };
        let mut rejected = read.rejected_entries > 0;
        for record in read.candidates {
            match self.validate_record(record, now) {
                Ok(validated) => {
                    let notice = rejected.then_some(DirectoryFailure::CacheCorrupt);
                    return (Some(validated), notice);
                }
                Err(_) => rejected = true,
            }
        }
        (None, rejected.then_some(DirectoryFailure::CacheCorrupt))
    }

    fn validate_record(
        &self,
        record: CachedDirectory,
        now: u64,
    ) -> Result<ValidatedRecord, DirectoryFailure> {
        if record.received_at > now && record.received_at - now > DIRECTORY_CLOCK_SKEW_SECONDS {
            return Err(DirectoryFailure::CacheCorrupt);
        }
        let expected_etag = directory_etag(&record.body);
        if record.etag != expected_etag {
            return Err(DirectoryFailure::CacheCorrupt);
        }
        let snapshot = parse_directory(&record.body, &self.compatibility)
            .map_err(|_| DirectoryFailure::CacheCorrupt)?;
        Ok(ValidatedRecord {
            record,
            snapshot,
            source: DirectorySource::Cache,
        })
    }

    fn process_response(
        &self,
        response: DirectoryResponse,
        cached: Option<&ValidatedRecord>,
        now: u64,
    ) -> Result<ValidatedRecord, DirectoryFailure> {
        match response.status {
            200 => self.accept_body(response, now),
            304 => Self::accept_not_modified(&response, cached, now),
            429 => Err(DirectoryFailure::RateLimited {
                retry_after_seconds: retry_after_seconds(&response, now)?,
            }),
            _ => Err(DirectoryFailure::HttpUnavailable),
        }
    }

    fn accept_body(
        &self,
        response: DirectoryResponse,
        now: u64,
    ) -> Result<ValidatedRecord, DirectoryFailure> {
        reject_content_encoding(&response)?;
        if single_header(&response, "content-type")? != Some(DIRECTORY_MEDIA_TYPE) {
            return Err(DirectoryFailure::InvalidMetadata);
        }
        validate_content_length(&response, response.body.len())?;
        let supplied_etag =
            single_header(&response, "etag")?.ok_or(DirectoryFailure::InvalidMetadata)?;
        let expected_etag = directory_etag(&response.body);
        if supplied_etag != expected_etag {
            return Err(DirectoryFailure::IntegrityMismatch);
        }
        let snapshot = parse_directory(&response.body, &self.compatibility)
            .map_err(DirectoryFailure::InvalidDirectory)?;
        let modified = single_header(&response, "last-modified")?
            .ok_or(DirectoryFailure::InvalidMetadata)
            .and_then(parse_http_unix)?;
        if modified != snapshot.generated_at || !snapshot.fresh_at(now) {
            return Err(DirectoryFailure::InvalidMetadata);
        }
        Ok(ValidatedRecord {
            record: CachedDirectory {
                received_at: now,
                etag: expected_etag,
                body: response.body,
            },
            snapshot,
            source: DirectorySource::Network,
        })
    }

    fn accept_not_modified(
        response: &DirectoryResponse,
        cached: Option<&ValidatedRecord>,
        now: u64,
    ) -> Result<ValidatedRecord, DirectoryFailure> {
        let cached = cached.ok_or(DirectoryFailure::InvalidMetadata)?;
        reject_content_encoding(response)?;
        if !response.body.is_empty() {
            return Err(DirectoryFailure::InvalidMetadata);
        }
        if let Some(content_type) = single_header(response, "content-type")?
            && content_type != DIRECTORY_MEDIA_TYPE
        {
            return Err(DirectoryFailure::InvalidMetadata);
        }
        validate_content_length(response, 0)?;
        let etag = single_header(response, "etag")?.ok_or(DirectoryFailure::InvalidMetadata)?;
        if etag != cached.record.etag {
            return Err(DirectoryFailure::IntegrityMismatch);
        }
        if let Some(last_modified) = single_header(response, "last-modified")?
            && parse_http_unix(last_modified)? != cached.snapshot.generated_at
        {
            return Err(DirectoryFailure::InvalidMetadata);
        }
        if !cached.snapshot.fresh_at(now) {
            return Err(DirectoryFailure::InvalidMetadata);
        }
        let mut validated = cached.clone();
        validated.record.received_at = now;
        validated.source = DirectorySource::Revalidated;
        Ok(validated)
    }
}

#[derive(Clone)]
struct ValidatedRecord {
    record: CachedDirectory,
    snapshot: DirectorySnapshot,
    source: DirectorySource,
}

fn view_from_snapshot(
    snapshot: DirectorySnapshot,
    received_at: u64,
    source: DirectorySource,
    notice: Option<DirectoryFailure>,
    now: u64,
) -> DirectoryView {
    let availability = if !snapshot.fresh_at(now) {
        DirectoryAvailability::Stale
    } else if !snapshot.servers.is_empty() {
        DirectoryAvailability::Fresh
    } else if snapshot.incompatible_servers > 0 {
        DirectoryAvailability::NoCompatibleServers
    } else {
        DirectoryAvailability::Empty
    };
    DirectoryView {
        availability,
        source,
        notice,
        received_at: Some(received_at),
        snapshot: Some(snapshot),
    }
}

fn fallback_view(
    cached: Option<ValidatedRecord>,
    notice: DirectoryFailure,
    now: u64,
) -> DirectoryView {
    let Some(cached) = cached else {
        return DirectoryView {
            availability: DirectoryAvailability::Unavailable,
            source: DirectorySource::None,
            notice: Some(notice),
            received_at: None,
            snapshot: None,
        };
    };
    if cached.snapshot.fresh_at(now) {
        return view_from_snapshot(
            cached.snapshot,
            cached.record.received_at,
            DirectorySource::Cache,
            Some(notice),
            now,
        );
    }
    if now.saturating_sub(cached.snapshot.expires_at) <= MAXIMUM_STALE_DISPLAY_SECONDS {
        return DirectoryView {
            availability: DirectoryAvailability::Stale,
            source: DirectorySource::Cache,
            notice: Some(notice),
            received_at: Some(cached.record.received_at),
            snapshot: Some(cached.snapshot),
        };
    }
    DirectoryView {
        availability: DirectoryAvailability::Unavailable,
        source: DirectorySource::Cache,
        notice: Some(notice),
        received_at: Some(cached.record.received_at),
        snapshot: None,
    }
}

fn single_header<'a>(
    response: &'a DirectoryResponse,
    name: &str,
) -> Result<Option<&'a str>, DirectoryFailure> {
    let values = response.header_values(name);
    match values.as_slice() {
        [] => Ok(None),
        [value] => Ok(Some(*value)),
        _ => Err(DirectoryFailure::InvalidMetadata),
    }
}

fn reject_content_encoding(response: &DirectoryResponse) -> Result<(), DirectoryFailure> {
    if single_header(response, "content-encoding")?.is_some() {
        return Err(DirectoryFailure::InvalidMetadata);
    }
    Ok(())
}

fn validate_content_length(
    response: &DirectoryResponse,
    expected: usize,
) -> Result<(), DirectoryFailure> {
    let Some(value) = single_header(response, "content-length")? else {
        return Ok(());
    };
    if value.is_empty()
        || (value.len() > 1 && value.starts_with('0'))
        || !value.bytes().all(|current| current.is_ascii_digit())
        || value.parse::<usize>().ok() != Some(expected)
    {
        return Err(DirectoryFailure::InvalidMetadata);
    }
    Ok(())
}

fn parse_http_unix(value: &str) -> Result<u64, DirectoryFailure> {
    parse_http_date(value)
        .map_err(|_| DirectoryFailure::InvalidMetadata)?
        .duration_since(UNIX_EPOCH)
        .map_err(|_| DirectoryFailure::InvalidMetadata)
        .map(|duration| duration.as_secs())
}

fn retry_after_seconds(response: &DirectoryResponse, now: u64) -> Result<u64, DirectoryFailure> {
    let value = single_header(response, "retry-after")?.ok_or(DirectoryFailure::InvalidMetadata)?;
    let delay = if !value.is_empty()
        && (value.len() == 1 || !value.starts_with('0'))
        && value.bytes().all(|current| current.is_ascii_digit())
    {
        value
            .parse::<u64>()
            .map_err(|_| DirectoryFailure::InvalidMetadata)?
    } else {
        parse_http_unix(value)?.saturating_sub(now)
    };
    if !(1..=MAXIMUM_RETRY_AFTER_SECONDS).contains(&delay) {
        return Err(DirectoryFailure::InvalidMetadata);
    }
    Ok(delay)
}

#[must_use]
pub fn directory_etag(body: &[u8]) -> String {
    let digest: [u8; 32] = Sha256::digest(body).into();
    format!("\"atrinik-directory-v1-sha256-{}\"", lowercase_hex(&digest))
}

fn lowercase_hex(input: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(input.len() * 2);
    for value in input {
        output.push(char::from(HEX[usize::from(value >> 4)]));
        output.push(char::from(HEX[usize::from(value & 0x0f)]));
    }
    output
}

impl From<DirectoryTransportError> for DirectoryFailure {
    fn from(value: DirectoryTransportError) -> Self {
        match value {
            DirectoryTransportError::Offline => Self::Offline,
            DirectoryTransportError::Timeout => Self::Timeout,
            DirectoryTransportError::Tls => Self::Tls,
            DirectoryTransportError::Protocol | DirectoryTransportError::BodyTooLarge => {
                Self::HttpUnavailable
            }
        }
    }
}

impl From<DirectoryCacheError> for DirectoryFailure {
    fn from(_: DirectoryCacheError) -> Self {
        Self::CacheUnavailable
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::{DirectoryCacheError, MemoryDirectoryCache};
    use crate::transport::{DirectoryResponse, DirectoryTransportError};
    use std::collections::VecDeque;
    use std::net::{IpAddr, Ipv4Addr};
    use std::time::{Duration, UNIX_EPOCH};

    const NOW: u64 = 1_786_219_201;
    const CANONICAL: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/metaserver-directory-v1/canonical.json"
    ));
    const EMPTY: &[u8] = b"{\"schema\":\"atrinik-directory-v1\",\"generation\":\"1\",\"generatedAt\":\"1786219200\",\"expiresAt\":\"1786233600\",\"servers\":[]}\n";
    const EXPECTED_ETAG: &str = "\"atrinik-directory-v1-sha256-059f559d0fe439576cae10bd623eb79ab6dfd6d0a78420563730c07cf9727d78\"";

    #[derive(Default)]
    struct FakeTransport {
        responses: VecDeque<Result<DirectoryResponse, DirectoryTransportError>>,
        requests: Vec<DirectoryRequest>,
    }

    impl FakeTransport {
        fn returning(response: Result<DirectoryResponse, DirectoryTransportError>) -> Self {
            Self {
                responses: VecDeque::from([response]),
                requests: Vec::new(),
            }
        }
    }

    impl DirectoryTransport for FakeTransport {
        fn fetch(
            &mut self,
            request: &DirectoryRequest,
        ) -> Result<DirectoryResponse, DirectoryTransportError> {
            self.requests.push(request.clone());
            self.responses
                .pop_front()
                .unwrap_or(Err(DirectoryTransportError::Offline))
        }
    }

    fn compatibility() -> InstalledCompatibility {
        InstalledCompatibility::exact(0, "atrinik-main", [0xaa; 32]).expect("valid compatibility")
    }

    fn response(status: u16, body: &[u8]) -> DirectoryResponse {
        let mut headers = Vec::new();
        if status == 200 {
            headers.extend([
                ("content-type".to_owned(), DIRECTORY_MEDIA_TYPE.to_owned()),
                ("content-length".to_owned(), body.len().to_string()),
                ("etag".to_owned(), directory_etag(body)),
                (
                    "last-modified".to_owned(),
                    httpdate::fmt_http_date(UNIX_EPOCH + Duration::from_hours(496_172)),
                ),
            ]);
        }
        DirectoryResponse {
            status,
            headers,
            body: body.to_vec(),
        }
    }

    fn cached(received_at: u64) -> CachedDirectory {
        CachedDirectory {
            received_at,
            etag: EXPECTED_ETAG.to_owned(),
            body: CANONICAL.to_vec(),
        }
    }

    #[test]
    fn accepted_network_snapshot_is_bounded_filtered_and_cached() {
        let transport = FakeTransport::returning(Ok(response(200, CANONICAL)));
        let mut service =
            DirectoryService::new(transport, MemoryDirectoryCache::default(), compatibility());
        let view = service.refresh(NOW);
        assert_eq!(view.availability, DirectoryAvailability::Fresh);
        assert_eq!(view.source, DirectorySource::Network);
        assert_eq!(view.notice, None);
        assert_eq!(view.snapshot.as_ref().expect("snapshot").servers.len(), 1);
        let (transport, cache, _) = service.into_parts();
        assert_eq!(transport.requests, [DirectoryRequest::new(None)]);
        assert_eq!(cache.records(), &[cached(NOW)]);
    }

    #[test]
    fn empty_and_no_compatible_states_are_distinct() {
        let transport = FakeTransport::returning(Ok(response(200, EMPTY)));
        let mut service =
            DirectoryService::new(transport, MemoryDirectoryCache::default(), compatibility());
        let empty = service.refresh(NOW);
        assert_eq!(empty.availability, DirectoryAvailability::Empty);
        assert_eq!(empty.message(), "No public servers are currently listed");

        let transport = FakeTransport::returning(Ok(response(200, CANONICAL)));
        let mut service = DirectoryService::new(
            transport,
            MemoryDirectoryCache::default(),
            InstalledCompatibility::Unavailable,
        );
        let incompatible = service.refresh(NOW);
        assert_eq!(
            incompatible.availability,
            DirectoryAvailability::NoCompatibleServers
        );
        assert_eq!(
            incompatible.message(),
            "No listed server matches the installed protocol and content"
        );
    }

    #[test]
    fn conditional_revalidation_refreshes_receipt_without_changing_body() {
        let mut not_modified = response(304, &[]);
        not_modified.headers = vec![("etag".to_owned(), EXPECTED_ETAG.to_owned())];
        let transport = FakeTransport::returning(Ok(not_modified));
        let cache = MemoryDirectoryCache::with_records(vec![cached(NOW - 10)]);
        let mut service = DirectoryService::new(transport, cache, compatibility());
        let view = service.refresh(NOW);
        assert_eq!(view.source, DirectorySource::Revalidated);
        assert_eq!(view.received_at, Some(NOW));
        let (transport, cache, _) = service.into_parts();
        assert_eq!(
            transport.requests,
            [DirectoryRequest::new(Some(EXPECTED_ETAG.to_owned()))]
        );
        assert_eq!(cache.records()[0].received_at, NOW);
    }

    #[test]
    fn network_failures_use_fresh_then_stale_lkg_but_never_stale_for_connection() {
        let transport = FakeTransport::returning(Err(DirectoryTransportError::Timeout));
        let mut service = DirectoryService::new(
            transport,
            MemoryDirectoryCache::with_records(vec![cached(NOW - 10)]),
            compatibility(),
        );
        let fresh = service.refresh(NOW);
        assert_eq!(fresh.availability, DirectoryAvailability::Fresh);
        assert_eq!(fresh.source, DirectorySource::Cache);
        assert_eq!(fresh.notice, Some(DirectoryFailure::Timeout));

        let (_, cache, compatibility) = service.into_parts();
        let transport = FakeTransport::returning(Err(DirectoryTransportError::Offline));
        let mut service = DirectoryService::new(transport, cache, compatibility);
        let stale_now = 1_786_233_601;
        let stale = service.refresh(stale_now);
        assert_eq!(stale.availability, DirectoryAvailability::Stale);
        assert_eq!(
            stale.connection_plan(&[0x11; 32], stale_now, RendezvousSupport::FixedV1),
            Err(ConnectionPlanError::RefreshRequired)
        );

        let (_, cache, compatibility) = service.into_parts();
        let transport = FakeTransport::returning(Err(DirectoryTransportError::Offline));
        let mut service = DirectoryService::new(transport, cache, compatibility);
        let expired = service.refresh(1_786_233_600 + MAXIMUM_STALE_DISPLAY_SECONDS + 1);
        assert_eq!(expired.availability, DirectoryAvailability::Unavailable);
        assert!(expired.snapshot.is_none());
    }

    #[test]
    fn corrupt_cache_does_not_poison_request_or_hide_transport_failure() {
        let mut bad = cached(NOW - 1);
        bad.etag = directory_etag(b"different");
        let transport = FakeTransport::returning(Err(DirectoryTransportError::Tls));
        let cache = MemoryDirectoryCache::with_records(vec![bad]);
        let mut service = DirectoryService::new(transport, cache, compatibility());
        let view = service.refresh(NOW);
        assert_eq!(view.availability, DirectoryAvailability::Unavailable);
        assert_eq!(view.notice, Some(DirectoryFailure::Tls));
        let (transport, _, _) = service.into_parts();
        assert_eq!(transport.requests, [DirectoryRequest::new(None)]);
    }

    #[test]
    fn cache_write_failure_keeps_valid_network_result_visible() {
        let transport = FakeTransport::returning(Ok(response(200, CANONICAL)));
        let mut cache = MemoryDirectoryCache::default();
        cache.fail_next_store(DirectoryCacheError::Io);
        let mut service = DirectoryService::new(transport, cache, compatibility());
        let view = service.refresh(NOW);
        assert_eq!(view.availability, DirectoryAvailability::Fresh);
        assert_eq!(view.notice, Some(DirectoryFailure::CacheWriteFailed));
    }

    #[test]
    fn metadata_integrity_and_clock_fail_closed_without_replacing_lkg() {
        let cases = [
            {
                let mut value = response(200, CANONICAL);
                value.headers.retain(|(name, _)| name != "etag");
                value
            },
            {
                let mut value = response(200, CANONICAL);
                value
                    .headers
                    .push(("etag".to_owned(), EXPECTED_ETAG.to_owned()));
                value
            },
            {
                let mut value = response(200, CANONICAL);
                value.headers.retain(|(name, _)| name != "content-type");
                value
            },
            {
                let mut value = response(200, CANONICAL);
                value
                    .headers
                    .push(("content-encoding".to_owned(), "gzip".to_owned()));
                value
            },
        ];
        for response in cases {
            let transport = FakeTransport::returning(Ok(response));
            let cache = MemoryDirectoryCache::with_records(vec![cached(NOW - 1)]);
            let mut service = DirectoryService::new(transport, cache, compatibility());
            let view = service.refresh(NOW);
            assert_eq!(view.source, DirectorySource::Cache);
            assert!(view.notice.is_some());
        }

        let transport = FakeTransport::returning(Ok(response(200, CANONICAL)));
        let mut service =
            DirectoryService::new(transport, MemoryDirectoryCache::default(), compatibility());
        let view = service.refresh(1_786_218_899);
        assert_eq!(view.availability, DirectoryAvailability::Unavailable);
        assert_eq!(view.notice, Some(DirectoryFailure::InvalidMetadata));
    }

    #[test]
    fn rate_limit_is_typed_bounded_and_preserves_lkg() {
        let mut limited = response(429, &[]);
        limited
            .headers
            .push(("retry-after".to_owned(), "60".to_owned()));
        let transport = FakeTransport::returning(Ok(limited));
        let cache = MemoryDirectoryCache::with_records(vec![cached(NOW - 1)]);
        let mut service = DirectoryService::new(transport, cache, compatibility());
        let view = service.refresh(NOW);
        assert_eq!(view.source, DirectorySource::Cache);
        assert_eq!(
            view.notice,
            Some(DirectoryFailure::RateLimited {
                retry_after_seconds: 60
            })
        );

        let mut dated = response(429, &[]);
        dated.headers.push((
            "retry-after".to_owned(),
            httpdate::fmt_http_date(UNIX_EPOCH + Duration::from_secs(NOW + 90)),
        ));
        let transport = FakeTransport::returning(Ok(dated));
        let mut service =
            DirectoryService::new(transport, MemoryDirectoryCache::default(), compatibility());
        assert_eq!(
            service.refresh(NOW).notice,
            Some(DirectoryFailure::RateLimited {
                retry_after_seconds: 90
            })
        );
    }

    #[test]
    fn freshness_boundaries_use_exact_protocol_skew_and_expiry_rules() {
        let transport = FakeTransport::returning(Ok(response(200, CANONICAL)));
        let mut service =
            DirectoryService::new(transport, MemoryDirectoryCache::default(), compatibility());
        assert_eq!(
            service.refresh(1_786_218_900).availability,
            DirectoryAvailability::Fresh
        );

        let transport = FakeTransport::returning(Err(DirectoryTransportError::Offline));
        let mut service = DirectoryService::new(
            transport,
            MemoryDirectoryCache::with_records(vec![cached(NOW)]),
            compatibility(),
        );
        assert_eq!(
            service.refresh(1_786_233_600).availability,
            DirectoryAvailability::Stale
        );
    }

    #[test]
    fn addressless_server_uses_only_fixed_rendezvous_and_remains_certificate_pinned() {
        let mut body = CANONICAL.to_vec();
        let endpoint = b",\"endpoint\":{\"hostname\":\"xn--bcher-kva.example.org\",\"port\":13327}";
        let start = body
            .windows(endpoint.len())
            .position(|window| window == endpoint)
            .expect("endpoint");
        body.drain(start..start + endpoint.len());
        let transport = FakeTransport::returning(Ok(response(200, &body)));
        let mut service =
            DirectoryService::new(transport, MemoryDirectoryCache::default(), compatibility());
        let view = service.refresh(NOW);
        let plan = view
            .connection_plan(&[0x11; 32], NOW, RendezvousSupport::FixedV1)
            .expect("addressless rendezvous plan");
        assert_eq!(plan.server_id, plan.certificate_sha256);
        assert_eq!(plan.direct_endpoint, None);
        assert_eq!(
            plan.rendezvous_url.as_deref(),
            Some(
                "wss://rendezvous.meta.atrinik.org/v1/servers/1111111111111111111111111111111111111111111111111111111111111111?role=client"
            )
        );
        assert_eq!(
            view.connection_plan(&[0x11; 32], NOW, RendezvousSupport::Disabled),
            Err(ConnectionPlanError::NoRoute)
        );
    }

    #[test]
    fn direct_configured_connection_is_independent_of_discovery() {
        let plan =
            ConfiguredConnectionPlan::for_ip(IpAddr::V4(Ipv4Addr::LOCALHOST), 13_327, [7; 32])
                .expect("configured plan");
        assert_eq!(plan.address, IpAddr::V4(Ipv4Addr::LOCALHOST));
        assert_eq!(plan.certificate_sha256, [7; 32]);
        assert_eq!(
            ConfiguredConnectionPlan::for_ip(IpAddr::V4(Ipv4Addr::LOCALHOST), 0, [7; 32]),
            Err(ConnectionPlanError::InvalidConfiguredEndpoint)
        );
    }

    #[test]
    fn etag_matches_the_language_neutral_manifest() {
        assert_eq!(directory_etag(CANONICAL), EXPECTED_ETAG);
    }
}
