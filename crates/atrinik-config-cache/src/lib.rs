#![forbid(unsafe_code)]
//! Independent settings, credential, resource-cache, log, and crash boundaries.

use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageClass {
    Credentials,
    TrustIdentity,
    Settings,
    Layout,
    ResourceCache,
    Logs,
    Screenshots,
    CrashData,
}

impl StorageClass {
    pub const fn directory(self) -> &'static str {
        match self {
            Self::Credentials => "credentials",
            Self::TrustIdentity => "trust",
            Self::Settings => "settings",
            Self::Layout => "layout",
            Self::ResourceCache => "resources",
            Self::Logs => "logs",
            Self::Screenshots => "screenshots",
            Self::CrashData => "crashes",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceClaim {
    pub stable_id: String,
    pub sha256: [u8; 32],
    pub compressed_bytes: u64,
    pub expanded_bytes: u64,
    pub media_type: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CacheError {
    InvalidIdentity,
    UnsupportedMedia,
    SizeLimit,
    DigestMismatch,
    ExecutableContent,
    Missing,
}
impl Display for CacheError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidIdentity => "resource identity is invalid",
            Self::UnsupportedMedia => "resource media type is not allowlisted",
            Self::SizeLimit => "resource exceeds transfer or expansion bounds",
            Self::DigestMismatch => "resource digest does not match authenticated claim",
            Self::ExecutableContent => "server-selected executable content is forbidden",
            Self::Missing => "authenticated resource is not available",
        })
    }
}
impl Error for CacheError {}

pub trait ResourceProvider {
    fn load(&self, claim: &ResourceClaim) -> Result<Arc<[u8]>, CacheError>;
}

#[derive(Default)]
pub struct MemoryResourceProvider {
    resources: BTreeMap<String, ([u8; 32], Arc<[u8]>)>,
    total_bytes: usize,
}

impl MemoryResourceProvider {
    pub fn insert(&mut self, claim: &ResourceClaim, bytes: &[u8]) -> Result<(), CacheError> {
        verify_resource(claim, bytes)?;
        let replaced = self
            .resources
            .get(&claim.stable_id)
            .map_or(0, |(_, current)| current.len());
        let total = self
            .total_bytes
            .checked_sub(replaced)
            .and_then(|value| value.checked_add(bytes.len()))
            .ok_or(CacheError::SizeLimit)?;
        if total > 256 * 1024 * 1024 {
            return Err(CacheError::SizeLimit);
        }
        self.resources
            .insert(claim.stable_id.clone(), (claim.sha256, Arc::from(bytes)));
        self.total_bytes = total;
        Ok(())
    }
}

impl ResourceProvider for MemoryResourceProvider {
    fn load(&self, claim: &ResourceClaim) -> Result<Arc<[u8]>, CacheError> {
        self.resources
            .get(&claim.stable_id)
            .filter(|(digest, _)| *digest == claim.sha256)
            .map(|(_, bytes)| Arc::clone(bytes))
            .ok_or(CacheError::Missing)
    }
}

pub fn verify_resource(claim: &ResourceClaim, bytes: &[u8]) -> Result<(), CacheError> {
    if claim.stable_id.is_empty()
        || claim.stable_id.len() > 160
        || !claim
            .stable_id
            .bytes()
            .all(|value| value.is_ascii_alphanumeric() || b"._:-/".contains(&value))
    {
        return Err(CacheError::InvalidIdentity);
    }
    if !matches!(
        claim.media_type.as_str(),
        "image/png" | "audio/ogg" | "application/vnd.atrinik.content"
    ) {
        return Err(CacheError::UnsupportedMedia);
    }
    if claim.compressed_bytes > 64 * 1024 * 1024
        || claim.expanded_bytes > 256 * 1024 * 1024
        || claim.expanded_bytes < claim.compressed_bytes
        || bytes.len() as u64 != claim.compressed_bytes
    {
        return Err(CacheError::SizeLimit);
    }
    if bytes.starts_with(b"\x7fELF")
        || bytes.starts_with(b"MZ")
        || bytes.starts_with(b"#!")
        || bytes.starts_with(b"\0asm")
    {
        return Err(CacheError::ExecutableContent);
    }
    let digest: [u8; 32] = Sha256::digest(bytes).into();
    if digest != claim.sha256 {
        return Err(CacheError::DigestMismatch);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    #[test]
    fn storage_classes_never_alias() {
        let classes = [
            StorageClass::Credentials,
            StorageClass::TrustIdentity,
            StorageClass::Settings,
            StorageClass::Layout,
            StorageClass::ResourceCache,
            StorageClass::Logs,
            StorageClass::Screenshots,
            StorageClass::CrashData,
        ];
        let names: BTreeSet<_> = classes.into_iter().map(StorageClass::directory).collect();
        assert_eq!(names.len(), classes.len());
    }
    #[test]
    fn authenticated_data_is_accepted_but_executables_fail() {
        let bytes = b"fixture";
        let claim = ResourceClaim {
            stable_id: "fixture:image".into(),
            sha256: Sha256::digest(bytes).into(),
            compressed_bytes: bytes.len() as u64,
            expanded_bytes: bytes.len() as u64,
            media_type: "image/png".into(),
        };
        assert_eq!(verify_resource(&claim, bytes), Ok(()));
        let mut resources = MemoryResourceProvider::default();
        resources.insert(&claim, bytes).expect("insert");
        assert_eq!(resources.load(&claim).expect("load").as_ref(), bytes);
        let executable = b"MZfixture";
        let claim = ResourceClaim {
            sha256: Sha256::digest(executable).into(),
            compressed_bytes: executable.len() as u64,
            expanded_bytes: executable.len() as u64,
            ..claim
        };
        assert_eq!(
            verify_resource(&claim, executable),
            Err(CacheError::ExecutableContent)
        );
    }
}
