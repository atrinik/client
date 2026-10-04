//! Certificate identity derivation for directory and GP1 trust boundaries.

use sha2::{Digest, Sha256};
use std::error::Error;
use std::fmt::{Display, Formatter};
use x509_cert::Certificate;
use x509_cert::der::asn1::ObjectIdentifier;
use x509_cert::der::{Decode, Encode};

pub const CERTIFICATE_DER_BYTES_LIMIT: usize = 2_048;
const EC_PUBLIC_KEY_OID: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.2.1");
const P256_CURVE_OID: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.3.1.7");

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CertificateIdentities {
    /// Metaserver identity: SHA-256 of the complete canonical DER certificate.
    pub server_id: [u8; 32],
    /// GP1 transport pin: SHA-256 of `SubjectPublicKeyInfo` DER from that certificate.
    pub gp1_spki_pin: [u8; 32],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CertificateIdentityError {
    InvalidLength,
    InvalidDer,
    InvalidAlgorithm,
    IdentityMismatch,
}

impl Display for CertificateIdentityError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidLength => "certificate DER length is invalid",
            Self::InvalidDer => "certificate DER is invalid",
            Self::InvalidAlgorithm => "certificate key algorithm is invalid",
            Self::IdentityMismatch => "certificate identity does not match the selected server",
        })
    }
}

impl Error for CertificateIdentityError {}

pub fn derive_certificate_identities(
    certificate_der: &[u8],
) -> Result<CertificateIdentities, CertificateIdentityError> {
    if certificate_der.is_empty() || certificate_der.len() > CERTIFICATE_DER_BYTES_LIMIT {
        return Err(CertificateIdentityError::InvalidLength);
    }
    let certificate =
        Certificate::from_der(certificate_der).map_err(|_| CertificateIdentityError::InvalidDer)?;
    let canonical_certificate = certificate
        .to_der()
        .map_err(|_| CertificateIdentityError::InvalidDer)?;
    if canonical_certificate != certificate_der {
        return Err(CertificateIdentityError::InvalidDer);
    }
    let spki = certificate.tbs_certificate().subject_public_key_info();
    let curve = spki
        .algorithm
        .parameters
        .as_ref()
        .and_then(|parameters| parameters.decode_as::<ObjectIdentifier>().ok());
    if spki.algorithm.oid != EC_PUBLIC_KEY_OID || curve != Some(P256_CURVE_OID) {
        return Err(CertificateIdentityError::InvalidAlgorithm);
    }
    let spki_der = spki
        .to_der()
        .map_err(|_| CertificateIdentityError::InvalidDer)?;
    Ok(CertificateIdentities {
        server_id: Sha256::digest(certificate_der).into(),
        gp1_spki_pin: Sha256::digest(spki_der).into(),
    })
}

pub fn verify_certificate_identity(
    certificate_der: &[u8],
    expected_server_id: &[u8; 32],
) -> Result<CertificateIdentities, CertificateIdentityError> {
    let identities = derive_certificate_identities(certificate_der)?;
    if &identities.server_id != expected_server_id {
        return Err(CertificateIdentityError::IdentityMismatch);
    }
    Ok(identities)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SYNTHETIC_P256_CERTIFICATE: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/access-resolve-v1/synthetic-p256.der"
    ));

    #[test]
    fn leaf_identity_and_gp1_pin_are_distinct_hashes_of_the_same_certificate() {
        let identities =
            derive_certificate_identities(SYNTHETIC_P256_CERTIFICATE).expect("fixture");
        assert_eq!(
            identities.server_id,
            [
                0x0d, 0x61, 0xda, 0xe9, 0x42, 0x26, 0xa6, 0x8c, 0x24, 0x52, 0x59, 0x88, 0x98, 0xd3,
                0x3e, 0xf8, 0xeb, 0x97, 0xa7, 0x3a, 0x04, 0x02, 0x94, 0x82, 0x5c, 0x2e, 0xed, 0xb0,
                0x1d, 0x6a, 0xee, 0x40,
            ]
        );
        assert_eq!(
            identities.gp1_spki_pin,
            [
                0x5c, 0xd2, 0x52, 0xfb, 0x0c, 0xe8, 0x93, 0x24, 0x36, 0xfa, 0xf8, 0xcc, 0xd1, 0x04,
                0x09, 0x81, 0xb8, 0x9e, 0xe4, 0xad, 0x6b, 0x9f, 0xe9, 0xe2, 0xa2, 0xb7, 0xe7, 0x1a,
                0xac, 0xb2, 0x7c, 0xd3,
            ]
        );
        assert_ne!(identities.server_id, identities.gp1_spki_pin);
        assert_eq!(
            verify_certificate_identity(SYNTHETIC_P256_CERTIFICATE, &identities.server_id),
            Ok(identities)
        );
        assert_eq!(
            verify_certificate_identity(SYNTHETIC_P256_CERTIFICATE, &[0; 32]),
            Err(CertificateIdentityError::IdentityMismatch)
        );
    }

    #[test]
    fn malformed_trailing_empty_and_oversized_certificates_fail_closed() {
        for invalid in [b"".as_slice(), b"not DER".as_slice()] {
            assert!(derive_certificate_identities(invalid).is_err());
        }
        assert_eq!(
            derive_certificate_identities(&vec![0u8; CERTIFICATE_DER_BYTES_LIMIT + 1]),
            Err(CertificateIdentityError::InvalidLength)
        );
    }
}
