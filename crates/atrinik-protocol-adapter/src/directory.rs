//! Bounded adapter from the released static-directory wire model to client data.

use atrinik_protocol::metaserver::directory::{
    DirectoryError as ProtocolDirectoryError, MAXIMUM_DIRECTORY_BODY_BYTES,
    MAXIMUM_DIRECTORY_FUTURE_SKEW, MAXIMUM_DIRECTORY_SERVERS, directory_server_compatible,
    parse_directory_json,
};
use atrinik_protocol::metaserver::v1::{
    DirectEndpoint as ProtocolEndpoint, DirectoryServer as ProtocolServer,
    DirectoryServerStatus as ProtocolStatus,
};
use std::error::Error;
use std::fmt::{Display, Formatter};

pub const DIRECTORY_BODY_BYTES_LIMIT: usize = MAXIMUM_DIRECTORY_BODY_BYTES;
pub const DIRECTORY_SERVER_LIMIT: usize = MAXIMUM_DIRECTORY_SERVERS;
pub const DIRECTORY_CLOCK_SKEW_SECONDS: u64 = MAXIMUM_DIRECTORY_FUTURE_SKEW;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DirectoryValidationError {
    InvalidJson,
    NonCanonicalJson,
    UnsupportedSchema,
    BodyTooLarge,
    TooManyServers,
    InvalidGeneration,
    InvalidFreshness,
    InvalidIdentity,
    InvalidText,
    InvalidRegion,
    InvalidProtocol,
    InvalidContent,
    InvalidPlayers,
    InvalidStatus,
    InvalidEndpoint,
    UnorderedServers,
    InternalContract,
}

impl DirectoryValidationError {
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidJson => "invalid_json",
            Self::NonCanonicalJson => "noncanonical_json",
            Self::UnsupportedSchema => "unsupported_schema",
            Self::BodyTooLarge => "body_too_large",
            Self::TooManyServers => "too_many_servers",
            Self::InvalidGeneration => "invalid_generation",
            Self::InvalidFreshness => "invalid_freshness",
            Self::InvalidIdentity => "invalid_identity",
            Self::InvalidText => "invalid_text",
            Self::InvalidRegion => "invalid_region",
            Self::InvalidProtocol => "invalid_protocol",
            Self::InvalidContent => "invalid_content",
            Self::InvalidPlayers => "invalid_players",
            Self::InvalidStatus => "invalid_status",
            Self::InvalidEndpoint => "invalid_endpoint",
            Self::UnorderedServers => "unordered_servers",
            Self::InternalContract => "internal_contract",
        }
    }
}

impl Display for DirectoryValidationError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "invalid static directory: {}", self.code())
    }
}

impl Error for DirectoryValidationError {}

impl From<ProtocolDirectoryError> for DirectoryValidationError {
    fn from(value: ProtocolDirectoryError) -> Self {
        match value {
            ProtocolDirectoryError::InvalidJson => Self::InvalidJson,
            ProtocolDirectoryError::NonCanonicalJson => Self::NonCanonicalJson,
            ProtocolDirectoryError::UnsupportedSchema => Self::UnsupportedSchema,
            ProtocolDirectoryError::BodyTooLarge => Self::BodyTooLarge,
            ProtocolDirectoryError::TooManyServers => Self::TooManyServers,
            ProtocolDirectoryError::InvalidGeneration => Self::InvalidGeneration,
            ProtocolDirectoryError::InvalidFreshness => Self::InvalidFreshness,
            ProtocolDirectoryError::InvalidIdentity => Self::InvalidIdentity,
            ProtocolDirectoryError::InvalidText => Self::InvalidText,
            ProtocolDirectoryError::InvalidRegion => Self::InvalidRegion,
            ProtocolDirectoryError::InvalidProtocol => Self::InvalidProtocol,
            ProtocolDirectoryError::InvalidContent => Self::InvalidContent,
            ProtocolDirectoryError::InvalidPlayers => Self::InvalidPlayers,
            ProtocolDirectoryError::InvalidStatus => Self::InvalidStatus,
            ProtocolDirectoryError::InvalidEndpoint => Self::InvalidEndpoint,
            ProtocolDirectoryError::UnorderedServers => Self::UnorderedServers,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InstalledCompatibility {
    Unavailable,
    Exact {
        protocol_minor: u32,
        content_id: String,
        content_revision_sha256: [u8; 32],
    },
}

impl InstalledCompatibility {
    pub fn exact(
        protocol_minor: u32,
        content_id: impl Into<String>,
        content_revision_sha256: [u8; 32],
    ) -> Result<Self, DirectoryValidationError> {
        let content_id = content_id.into();
        if protocol_minor > 65_535 {
            return Err(DirectoryValidationError::InvalidProtocol);
        }
        if !valid_content_id(&content_id) {
            return Err(DirectoryValidationError::InvalidContent);
        }
        Ok(Self::Exact {
            protocol_minor,
            content_id,
            content_revision_sha256,
        })
    }

    pub fn exact_hex(
        protocol_minor: &str,
        content_id: &str,
        content_revision_sha256: &str,
    ) -> Result<Self, DirectoryValidationError> {
        if protocol_minor.is_empty()
            || (protocol_minor.len() > 1 && protocol_minor.starts_with('0'))
            || !protocol_minor.bytes().all(|value| value.is_ascii_digit())
        {
            return Err(DirectoryValidationError::InvalidProtocol);
        }
        let protocol_minor = protocol_minor
            .parse::<u32>()
            .map_err(|_| DirectoryValidationError::InvalidProtocol)?;
        if content_revision_sha256.len() != 64
            || !content_revision_sha256
                .bytes()
                .all(|value| value.is_ascii_digit() || (b'a'..=b'f').contains(&value))
        {
            return Err(DirectoryValidationError::InvalidContent);
        }
        let mut digest = [0u8; 32];
        for (output, pair) in digest
            .iter_mut()
            .zip(content_revision_sha256.as_bytes().chunks_exact(2))
        {
            *output = (hex_nibble(pair[0]) << 4) | hex_nibble(pair[1]);
        }
        Self::exact(protocol_minor, content_id, digest)
    }

    fn matches(&self, server: &ProtocolServer) -> bool {
        match self {
            Self::Unavailable => false,
            Self::Exact {
                protocol_minor,
                content_id,
                content_revision_sha256,
            } => directory_server_compatible(
                server,
                1,
                *protocol_minor,
                content_id,
                content_revision_sha256,
            ),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DirectoryServerStatus {
    Online,
    Full,
    Maintenance,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DirectEndpoint {
    pub hostname: String,
    pub port: u16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DirectoryServer {
    pub server_id: [u8; 32],
    pub certificate_sha256: [u8; 32],
    pub name: String,
    pub description: String,
    pub region: Option<String>,
    pub protocol_minor: u32,
    pub content_id: String,
    pub content_revision_sha256: [u8; 32],
    pub players_online: u32,
    pub players_capacity: u32,
    pub status: DirectoryServerStatus,
    pub password_required: bool,
    pub endpoint: Option<DirectEndpoint>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DirectorySnapshot {
    pub generation: u64,
    pub generated_at: u64,
    pub expires_at: u64,
    pub servers: Vec<DirectoryServer>,
    pub incompatible_servers: usize,
}

impl DirectorySnapshot {
    #[must_use]
    pub fn fresh_at(&self, now: u64) -> bool {
        if self.generated_at > now && self.generated_at - now > DIRECTORY_CLOCK_SKEW_SECONDS {
            return false;
        }
        now < self.expires_at
    }
}

pub fn parse_directory(
    input: &[u8],
    compatibility: &InstalledCompatibility,
) -> Result<DirectorySnapshot, DirectoryValidationError> {
    let parsed = parse_directory_json(input)?;
    let mut servers = Vec::with_capacity(parsed.servers.len());
    let mut incompatible_servers = 0usize;
    for server in parsed.servers {
        if compatibility.matches(&server) {
            servers.push(convert_server(server)?);
        } else {
            incompatible_servers = incompatible_servers
                .checked_add(1)
                .ok_or(DirectoryValidationError::InternalContract)?;
        }
    }
    Ok(DirectorySnapshot {
        generation: parsed.generation,
        generated_at: parsed.generated_at_unix_seconds,
        expires_at: parsed.expires_at_unix_seconds,
        servers,
        incompatible_servers,
    })
}

fn convert_server(server: ProtocolServer) -> Result<DirectoryServer, DirectoryValidationError> {
    let server_id = fixed_digest(server.server_id.as_ref())?;
    let certificate_sha256 = fixed_digest(server.certificate_sha256.as_ref())?;
    let content_revision_sha256 = fixed_digest(server.content_revision_sha256.as_ref())?;
    if server_id != certificate_sha256 {
        return Err(DirectoryValidationError::InvalidIdentity);
    }
    let status = match ProtocolStatus::try_from(server.status) {
        Ok(ProtocolStatus::Online) => DirectoryServerStatus::Online,
        Ok(ProtocolStatus::Full) => DirectoryServerStatus::Full,
        Ok(ProtocolStatus::Maintenance) => DirectoryServerStatus::Maintenance,
        Ok(ProtocolStatus::Unspecified) | Err(_) => {
            return Err(DirectoryValidationError::InvalidStatus);
        }
    };
    let endpoint = server.endpoint.map(convert_endpoint).transpose()?;
    Ok(DirectoryServer {
        server_id,
        certificate_sha256,
        name: server.name,
        description: server.description,
        region: server.region,
        protocol_minor: server.protocol_minor,
        content_id: server.content_id,
        content_revision_sha256,
        players_online: server.players_online,
        players_capacity: server.players_capacity,
        status,
        password_required: server.password_required,
        endpoint,
    })
}

fn convert_endpoint(
    endpoint: ProtocolEndpoint,
) -> Result<DirectEndpoint, DirectoryValidationError> {
    let port =
        u16::try_from(endpoint.port).map_err(|_| DirectoryValidationError::InvalidEndpoint)?;
    if port == 0 {
        return Err(DirectoryValidationError::InvalidEndpoint);
    }
    Ok(DirectEndpoint {
        hostname: endpoint.hostname,
        port,
    })
}

fn fixed_digest(input: &[u8]) -> Result<[u8; 32], DirectoryValidationError> {
    input
        .try_into()
        .map_err(|_| DirectoryValidationError::InternalContract)
}

fn valid_content_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    let Some(first) = bytes.first() else {
        return false;
    };
    let Some(last) = bytes.last() else {
        return false;
    };
    bytes.len() <= 64
        && lower_alphanumeric(*first)
        && lower_alphanumeric(*last)
        && bytes
            .iter()
            .skip(1)
            .take(bytes.len().saturating_sub(2))
            .all(|current| lower_alphanumeric(*current) || b"._-".contains(current))
}

const fn lower_alphanumeric(value: u8) -> bool {
    value.is_ascii_lowercase() || value.is_ascii_digit()
}

const fn hex_nibble(value: u8) -> u8 {
    if value.is_ascii_digit() {
        value - b'0'
    } else {
        value - b'a' + 10
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atrinik_protocol::metaserver::directory::{marshal_directory_json, parse_directory_json};

    const CANONICAL: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/metaserver-directory-v1/canonical.json"
    ));
    const CONTENT_REVISION: [u8; 32] = [0xaa; 32];

    #[test]
    fn released_fixture_maps_to_client_owned_types_and_filters_before_display() {
        let compatibility = InstalledCompatibility::exact(0, "atrinik-main", CONTENT_REVISION)
            .expect("compatibility");
        let snapshot = parse_directory(CANONICAL, &compatibility).expect("canonical fixture");
        assert_eq!(snapshot.generation, 42);
        assert_eq!(snapshot.generated_at, 1_786_219_200);
        assert_eq!(snapshot.expires_at, 1_786_233_600);
        assert_eq!(snapshot.incompatible_servers, 1);
        assert_eq!(snapshot.servers.len(), 1);
        let server = &snapshot.servers[0];
        assert_eq!(server.server_id, [0x11; 32]);
        assert_eq!(server.certificate_sha256, server.server_id);
        assert_eq!(server.name, "Atrinik \"Alpha\"");
        assert_eq!(server.description, "Cooperative Ω");
        assert_eq!(server.region.as_deref(), Some("eu-west"));
        assert_eq!(server.players_online, 3);
        assert_eq!(server.players_capacity, 64);
        assert_eq!(server.status, DirectoryServerStatus::Online);
        assert!(!server.password_required);
        assert_eq!(
            server.endpoint,
            Some(DirectEndpoint {
                hostname: "xn--bcher-kva.example.org".to_owned(),
                port: 13_327,
            })
        );
    }

    #[test]
    fn unavailable_compatibility_exposes_no_server() {
        let snapshot = parse_directory(CANONICAL, &InstalledCompatibility::Unavailable)
            .expect("valid directory");
        assert!(snapshot.servers.is_empty());
        assert_eq!(snapshot.incompatible_servers, 2);
    }

    #[test]
    fn maximum_server_snapshot_maps_transactionally_within_the_shared_body_limit() {
        let canonical = parse_directory_json(CANONICAL).expect("canonical model");
        let template = canonical.servers[0].clone();
        let mut maximum = canonical;
        maximum.servers.clear();
        for index in 0..DIRECTORY_SERVER_LIMIT {
            let mut server = template.clone();
            let mut identity = [0u8; 32];
            identity[30..].copy_from_slice(
                &u16::try_from(index)
                    .expect("bounded fixture index")
                    .to_be_bytes(),
            );
            server.server_id = identity.to_vec().into();
            server.certificate_sha256 = identity.to_vec().into();
            server.name = format!("Server {index:03}");
            server.description.clear();
            server.region = None;
            server.endpoint = None;
            maximum.servers.push(server);
        }
        let body = marshal_directory_json(&maximum).expect("maximum canonical body");
        assert!(body.len() <= DIRECTORY_BODY_BYTES_LIMIT);
        let snapshot = parse_directory(
            &body,
            &InstalledCompatibility::exact(0, "atrinik-main", CONTENT_REVISION)
                .expect("compatibility"),
        )
        .expect("maximum client model");
        assert_eq!(snapshot.servers.len(), DIRECTORY_SERVER_LIMIT);
        assert_eq!(snapshot.incompatible_servers, 0);
        assert_eq!(
            parse_directory(
                &vec![b' '; DIRECTORY_BODY_BYTES_LIMIT + 1],
                &InstalledCompatibility::Unavailable,
            ),
            Err(DirectoryValidationError::BodyTooLarge)
        );
    }

    #[test]
    fn compatibility_input_is_bounded_and_canonical() {
        assert_eq!(
            InstalledCompatibility::exact(65_536, "atrinik-main", CONTENT_REVISION),
            Err(DirectoryValidationError::InvalidProtocol)
        );
        for invalid in ["", "-bad", "bad-", "Bad", "a/b", &"a".repeat(65)] {
            assert_eq!(
                InstalledCompatibility::exact(0, invalid, CONTENT_REVISION),
                Err(DirectoryValidationError::InvalidContent)
            );
        }
        assert_eq!(
            InstalledCompatibility::exact_hex("0", "atrinik-main", &"aa".repeat(32)),
            InstalledCompatibility::exact(0, "atrinik-main", CONTENT_REVISION)
        );
        for (minor, content, digest) in [
            ("00", "atrinik-main", "aa".repeat(32)),
            ("x", "atrinik-main", "aa".repeat(32)),
            ("0", "Atrinik", "aa".repeat(32)),
            ("0", "atrinik-main", "AA".repeat(32)),
            ("0", "atrinik-main", "a".repeat(63)),
            ("0", "atrinik-main", "é".repeat(32)),
        ] {
            assert!(InstalledCompatibility::exact_hex(minor, content, &digest).is_err());
        }
    }

    #[test]
    fn every_language_neutral_negative_fixture_has_the_declared_error() {
        let fixtures: &[(&[u8], DirectoryValidationError)] = &[
            (
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../fixtures/metaserver-directory-v1/negative-unsupported-schema.json"
                )),
                DirectoryValidationError::UnsupportedSchema,
            ),
            (
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../fixtures/metaserver-directory-v1/negative-zero-generation.json"
                )),
                DirectoryValidationError::InvalidGeneration,
            ),
            (
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../fixtures/metaserver-directory-v1/negative-expired-at-generation.json"
                )),
                DirectoryValidationError::InvalidFreshness,
            ),
            (
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../fixtures/metaserver-directory-v1/negative-identity-mismatch.json"
                )),
                DirectoryValidationError::InvalidIdentity,
            ),
            (
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../fixtures/metaserver-directory-v1/negative-numeric-endpoint.json"
                )),
                DirectoryValidationError::InvalidEndpoint,
            ),
            (
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../fixtures/metaserver-directory-v1/negative-invalid-alabel.json"
                )),
                DirectoryValidationError::InvalidEndpoint,
            ),
            (
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../fixtures/metaserver-directory-v1/negative-status-count.json"
                )),
                DirectoryValidationError::InvalidStatus,
            ),
            (
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../fixtures/metaserver-directory-v1/negative-unordered-servers.json"
                )),
                DirectoryValidationError::UnorderedServers,
            ),
            (
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../fixtures/metaserver-directory-v1/negative-duplicate-server.json"
                )),
                DirectoryValidationError::UnorderedServers,
            ),
            (
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../fixtures/metaserver-directory-v1/negative-noncanonical-whitespace.json"
                )),
                DirectoryValidationError::NonCanonicalJson,
            ),
            (
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../fixtures/metaserver-directory-v1/negative-private-field.json"
                )),
                DirectoryValidationError::NonCanonicalJson,
            ),
            (
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../fixtures/metaserver-directory-v1/negative-xml-noncharacter.json"
                )),
                DirectoryValidationError::InvalidText,
            ),
        ];
        for (input, expected) in fixtures {
            assert_eq!(
                parse_directory(input, &InstalledCompatibility::Unavailable),
                Err(*expected)
            );
        }
    }

    #[test]
    fn truncations_and_single_byte_mutations_never_return_partial_state() {
        for length in 0..CANONICAL.len() {
            assert!(
                parse_directory(&CANONICAL[..length], &InstalledCompatibility::Unavailable)
                    .is_err()
            );
        }
        for index in (0..CANONICAL.len()).step_by(17) {
            let mut mutated = CANONICAL.to_vec();
            mutated[index] ^= 0x5a;
            let first = parse_directory(&mutated, &InstalledCompatibility::Unavailable);
            let second = parse_directory(&mutated, &InstalledCompatibility::Unavailable);
            assert_eq!(first, second);
            if let Ok(snapshot) = first {
                assert!(snapshot.servers.is_empty());
                assert_eq!(snapshot.incompatible_servers, 2);
            }
        }
    }
}
