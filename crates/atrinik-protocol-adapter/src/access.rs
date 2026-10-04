//! Bounded GP1 access negotiation at the generated-contract boundary.

use atrinik_protocol::game::v1::{
    AccessAuth, AccessPolicy, AccessResult, AccessStatus, Capability, ClientHello, Platform,
    ProtocolVersion, ServerHello, SessionId,
};
use atrinik_protocol::validation;
use atrinik_session::Event;
use std::error::Error;
use std::fmt::{Display, Formatter};
use zeroize::Zeroize;

const SESSION_ID_BYTES: usize = 16;
const ACCESS_CODE_BYTES: usize = 16;
const SUPPORTED_PROTOCOL_MINOR: u32 = 1;
const ACCESS_ALPHABET: &[u8] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccessNegotiation {
    pub session_id: [u8; SESSION_ID_BYTES],
    pub event: Event,
}

/// A validated GP1 1.1 access offer for immediate encrypted serialization.
pub struct AccessClientHello {
    message: ClientHello,
}

impl AccessClientHello {
    /// Borrows the shared-contract-validated offer only for its immediate send.
    pub fn with_message<T>(&self, send: impl FnOnce(&ClientHello) -> T) -> T {
        send(&self.message)
    }
}

impl Drop for AccessClientHello {
    fn drop(&mut self) {
        let bytes = std::mem::take(&mut self.message.client_nonce);
        if let Ok(mut bytes) = bytes.try_into_mut() {
            bytes.as_mut().zeroize();
        }
    }
}

/// A session-bound GP1 credential message whose code bytes are never printable.
pub struct AccessAuthentication {
    message: AccessAuth,
}

impl AccessAuthentication {
    /// Borrows the generated message only for immediate encrypted serialization.
    pub fn with_message<T>(&self, use_message: impl FnOnce(&AccessAuth) -> T) -> T {
        use_message(&self.message)
    }

    /// Returns the state transition to apply only after the encrypted write succeeds.
    #[must_use]
    pub const fn sent_event(&self) -> Event {
        Event::AccessAuthenticationSent
    }
}

impl Drop for AccessAuthentication {
    fn drop(&mut self) {
        let bytes = std::mem::take(&mut self.message.code);
        if let Ok(mut bytes) = bytes.try_into_mut() {
            bytes.as_mut().zeroize();
        }
        if let Some(session_id) = &mut self.message.session_id {
            let bytes = std::mem::take(&mut session_id.value);
            if let Ok(mut bytes) = bytes.try_into_mut() {
                bytes.as_mut().zeroize();
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccessAdapterError {
    InvalidClientHello,
    UnsupportedVersion,
    MissingCapability,
    InvalidPolicy,
    InvalidSession,
    InvalidIdentity,
    InvalidCode,
    InvalidResult,
}

impl Display for AccessAdapterError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidClientHello => "client access offer is invalid",
            Self::UnsupportedVersion => "server does not support GP1 access negotiation",
            Self::MissingCapability => "server omitted the access-token capability",
            Self::InvalidPolicy => "server access policy is invalid",
            Self::InvalidSession => "server session identity is invalid",
            Self::InvalidIdentity => "server transport identity changed",
            Self::InvalidCode => "access code is invalid",
            Self::InvalidResult => "server access result is invalid",
        })
    }
}

impl Error for AccessAdapterError {}

/// Constructs the current client's exact access offer and validates it through
/// the shared protocol contract before the caller can serialize or send it.
pub fn access_client_hello(
    client_nonce: [u8; 32],
) -> Result<AccessClientHello, AccessAdapterError> {
    let platform = if cfg!(target_os = "linux") {
        Platform::Linux
    } else if cfg!(target_os = "windows") {
        Platform::Windows
    } else {
        return Err(AccessAdapterError::InvalidClientHello);
    };
    let message = ClientHello {
        version: Some(ProtocolVersion {
            major: 1,
            minor: SUPPORTED_PROTOCOL_MINOR,
        }),
        capabilities: vec![Capability::AccessTokensV1 as i32],
        locale: "en".to_owned(),
        platform: platform as i32,
        build_id: env!("CARGO_PKG_VERSION").to_owned(),
        client_nonce: client_nonce.to_vec().into(),
    };
    validation::access_client_hello(&message)
        .map_err(|_| AccessAdapterError::InvalidClientHello)?;
    Ok(AccessClientHello { message })
}

pub fn negotiate_server_access(
    hello: &ServerHello,
    expected_gp1_spki_pin: &[u8; 32],
) -> Result<AccessNegotiation, AccessAdapterError> {
    let version = hello
        .version
        .as_ref()
        .ok_or(AccessAdapterError::UnsupportedVersion)?;
    if version.major != 1 || version.minor != SUPPORTED_PROTOCOL_MINOR {
        return Err(AccessAdapterError::UnsupportedVersion);
    }
    if !hello
        .capabilities
        .contains(&(Capability::AccessTokensV1 as i32))
    {
        return Err(AccessAdapterError::MissingCapability);
    }
    let session_id: [u8; SESSION_ID_BYTES] = hello
        .session_id
        .as_ref()
        .ok_or(AccessAdapterError::InvalidSession)?
        .value
        .as_ref()
        .try_into()
        .map_err(|_| AccessAdapterError::InvalidSession)?;
    let server_identity: [u8; 32] = hello
        .server_identity
        .as_ref()
        .ok_or(AccessAdapterError::InvalidIdentity)?
        .value
        .as_ref()
        .try_into()
        .map_err(|_| AccessAdapterError::InvalidIdentity)?;
    if &server_identity != expected_gp1_spki_pin {
        return Err(AccessAdapterError::InvalidIdentity);
    }
    let event = match AccessPolicy::try_from(hello.access_policy) {
        Ok(AccessPolicy::Open) => Event::Connected,
        Ok(AccessPolicy::Protected) => Event::AccessRequired,
        Ok(AccessPolicy::Unspecified) | Err(_) => return Err(AccessAdapterError::InvalidPolicy),
    };
    Ok(AccessNegotiation { session_id, event })
}

pub fn access_auth(
    canonical_code: &[u8; ACCESS_CODE_BYTES],
    session_id: [u8; SESSION_ID_BYTES],
) -> Result<AccessAuthentication, AccessAdapterError> {
    if !canonical_code
        .iter()
        .all(|value| ACCESS_ALPHABET.contains(value))
    {
        return Err(AccessAdapterError::InvalidCode);
    }
    Ok(AccessAuthentication {
        message: AccessAuth {
            code: canonical_code.to_vec().into(),
            session_id: Some(SessionId {
                value: session_id.to_vec().into(),
            }),
        },
    })
}

pub fn access_result_event(result: &AccessResult) -> Result<Event, AccessAdapterError> {
    match AccessStatus::try_from(result.status) {
        Ok(AccessStatus::Accepted) => Ok(Event::AccessAccepted),
        Ok(AccessStatus::Unavailable) => Ok(Event::AccessUnavailable),
        Ok(AccessStatus::Unspecified) | Err(_) => Err(AccessAdapterError::InvalidResult),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atrinik_protocol::game::v1::Digest256;

    #[test]
    fn constructed_client_hello_is_the_validated_access_offer() {
        let hello = access_client_hello([5; 32]).expect("validated hello");
        hello.with_message(|message| {
            assert_eq!(
                message.version,
                Some(ProtocolVersion { major: 1, minor: 1 })
            );
            assert_eq!(message.capabilities, [Capability::AccessTokensV1 as i32]);
            assert_eq!(message.locale, "en");
            let expected_platform = if cfg!(target_os = "windows") {
                Platform::Windows
            } else {
                Platform::Linux
            };
            assert_eq!(message.platform, expected_platform as i32);
            assert_eq!(message.build_id, env!("CARGO_PKG_VERSION"));
            assert_eq!(message.client_nonce.as_ref(), &[5; 32]);
            assert_eq!(validation::access_client_hello(message), Ok(()));
        });
    }

    fn hello(policy: AccessPolicy) -> ServerHello {
        ServerHello {
            version: Some(ProtocolVersion { major: 1, minor: 1 }),
            capabilities: vec![Capability::AccessTokensV1 as i32],
            session_id: Some(SessionId {
                value: vec![7; SESSION_ID_BYTES].into(),
            }),
            server_identity: Some(Digest256 {
                value: vec![9; 32].into(),
            }),
            maximum_gameplay_frame_bytes: 1,
            maximum_resource_frame_bytes: 1,
            idle_timeout: None,
            access_policy: policy as i32,
        }
    }

    #[test]
    fn mandatory_policy_maps_to_session_gate() {
        assert_eq!(
            negotiate_server_access(&hello(AccessPolicy::Open), &[9; 32]),
            Ok(AccessNegotiation {
                session_id: [7; SESSION_ID_BYTES],
                event: Event::Connected,
            })
        );
        assert_eq!(
            negotiate_server_access(&hello(AccessPolicy::Protected), &[9; 32]),
            Ok(AccessNegotiation {
                session_id: [7; SESSION_ID_BYTES],
                event: Event::AccessRequired,
            })
        );
    }

    #[test]
    fn missing_capability_unknown_policy_and_changed_pin_fail_closed() {
        for minor in [0, 2, u32::MAX] {
            let mut invalid = hello(AccessPolicy::Open);
            invalid.version = Some(ProtocolVersion { major: 1, minor });
            assert_eq!(
                negotiate_server_access(&invalid, &[9; 32]),
                Err(AccessAdapterError::UnsupportedVersion)
            );
        }
        let mut invalid = hello(AccessPolicy::Open);
        invalid.capabilities.clear();
        assert_eq!(
            negotiate_server_access(&invalid, &[9; 32]),
            Err(AccessAdapterError::MissingCapability)
        );
        invalid = hello(AccessPolicy::Open);
        invalid.access_policy = 99;
        assert_eq!(
            negotiate_server_access(&invalid, &[9; 32]),
            Err(AccessAdapterError::InvalidPolicy)
        );
        assert_eq!(
            negotiate_server_access(&hello(AccessPolicy::Open), &[8; 32]),
            Err(AccessAdapterError::InvalidIdentity)
        );
    }

    #[test]
    fn access_auth_is_session_bound_and_results_are_indistinguishable() {
        let auth = access_auth(b"0123456789ABCDEF", [7; SESSION_ID_BYTES]).expect("auth");
        auth.with_message(|message| {
            assert_eq!(message.code.as_ref(), b"0123456789ABCDEF");
            assert_eq!(
                message.session_id.as_ref().expect("session").value.as_ref(),
                &[7; SESSION_ID_BYTES]
            );
        });
        assert!(access_auth(b"0123456789ABCDEO", [7; SESSION_ID_BYTES]).is_err());
        assert_eq!(auth.sent_event(), Event::AccessAuthenticationSent);
        assert_eq!(
            access_result_event(&AccessResult {
                status: AccessStatus::Accepted as i32,
            }),
            Ok(Event::AccessAccepted)
        );
        assert_eq!(
            access_result_event(&AccessResult {
                status: AccessStatus::Unavailable as i32,
            }),
            Ok(Event::AccessUnavailable)
        );
    }
}
