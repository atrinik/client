//! Transactional last-known-good directory cache storage.

use crate::{directory_body_sha256, lowercase_hex};
use atrinik_protocol_adapter::directory::{
    DIRECTORY_BODY_BYTES_LIMIT, DIRECTORY_CLOCK_SKEW_SECONDS,
};
use sha2::{Digest, Sha256};
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

const CACHE_MAGIC_V1: &[u8] = b"ATRINIK-DIRECTORY-CACHE-V1\n";
const CACHE_MAGIC_V2: &[u8] = b"ATRINIK-DIRECTORY-CACHE-V2\n";
const CACHE_ENTRY_PREFIX: &str = "directory-";
const CACHE_ENTRY_SUFFIX: &str = ".cache";
const TEMP_PREFIX: &str = ".tmp-";
const CACHE_METADATA_ALLOWANCE: usize = 512;
const MAXIMUM_CACHE_ENTRY_BYTES: usize = DIRECTORY_BODY_BYTES_LIMIT + CACHE_METADATA_ALLOWANCE;
const MAXIMUM_CACHE_SCAN: usize = 64;
const MAXIMUM_CACHE_CANDIDATES: usize = 4;
const STALE_TEMP_LIFETIME: Duration = Duration::from_hours(24);
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CachedDirectory {
    pub received_at: u64,
    /// Exact static-alias publication second from `Last-Modified`.
    /// `None` is accepted only while reading a verified V1 cache record.
    pub published_at: Option<u64>,
    pub etag: String,
    pub body_sha256: [u8; 32],
    pub body: Vec<u8>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CacheRead {
    pub candidates: Vec<CachedDirectory>,
    pub rejected_entries: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DirectoryCacheError {
    Io,
    Capacity,
    InvalidRecord,
    Conflict,
}

impl Display for DirectoryCacheError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Io => "directory cache I/O failed",
            Self::Capacity => "directory cache entry scan exceeded its bound",
            Self::InvalidRecord => "directory cache record is invalid",
            Self::Conflict => "directory cache publication conflicted",
        })
    }
}

impl Error for DirectoryCacheError {}

pub trait DirectoryCache {
    fn load(&mut self) -> Result<CacheRead, DirectoryCacheError>;
    fn store(&mut self, record: &CachedDirectory) -> Result<(), DirectoryCacheError>;
}

#[derive(Default)]
pub struct MemoryDirectoryCache {
    records: Vec<CachedDirectory>,
    rejected_entries: usize,
    fail_load: Option<DirectoryCacheError>,
    fail_store: Option<DirectoryCacheError>,
}

impl MemoryDirectoryCache {
    pub fn with_records(records: Vec<CachedDirectory>) -> Self {
        Self {
            records,
            ..Self::default()
        }
    }

    pub fn set_rejected_entries(&mut self, value: usize) {
        self.rejected_entries = value;
    }

    pub fn fail_next_load(&mut self, error: DirectoryCacheError) {
        self.fail_load = Some(error);
    }

    pub fn fail_next_store(&mut self, error: DirectoryCacheError) {
        self.fail_store = Some(error);
    }

    pub fn records(&self) -> &[CachedDirectory] {
        &self.records
    }
}

impl DirectoryCache for MemoryDirectoryCache {
    fn load(&mut self) -> Result<CacheRead, DirectoryCacheError> {
        if let Some(error) = self.fail_load.take() {
            return Err(error);
        }
        let mut candidates = self.records.clone();
        candidates.sort_by(|left, right| {
            right
                .received_at
                .cmp(&left.received_at)
                .then_with(|| right.etag.cmp(&left.etag))
        });
        candidates.truncate(MAXIMUM_CACHE_CANDIDATES);
        Ok(CacheRead {
            candidates,
            rejected_entries: self.rejected_entries,
        })
    }

    fn store(&mut self, record: &CachedDirectory) -> Result<(), DirectoryCacheError> {
        if let Some(error) = self.fail_store.take() {
            return Err(error);
        }
        validate_record_shape(record)?;
        self.records.push(record.clone());
        self.records.sort_by(|left, right| {
            right
                .received_at
                .cmp(&left.received_at)
                .then_with(|| right.etag.cmp(&left.etag))
        });
        self.records.truncate(MAXIMUM_CACHE_CANDIDATES);
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileDirectoryCache {
    root: PathBuf,
}

impl FileDirectoryCache {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

impl DirectoryCache for FileDirectoryCache {
    fn load(&mut self) -> Result<CacheRead, DirectoryCacheError> {
        let metadata = match fs::symlink_metadata(&self.root) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(CacheRead::default()),
            Err(_) => return Err(DirectoryCacheError::Io),
        };
        if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
            return Err(DirectoryCacheError::Io);
        }
        let entries = match fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(CacheRead::default()),
            Err(_) => return Err(DirectoryCacheError::Io),
        };
        let mut recognized = 0usize;
        let mut rejected_entries = 0usize;
        let mut candidates = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|_| DirectoryCacheError::Io)?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if !recognized_cache_name(name) {
                continue;
            }
            recognized = recognized
                .checked_add(1)
                .ok_or(DirectoryCacheError::Capacity)?;
            if recognized > MAXIMUM_CACHE_SCAN {
                return Err(DirectoryCacheError::Capacity);
            }
            let file_type = entry.file_type().map_err(|_| DirectoryCacheError::Io)?;
            if !file_type.is_file() {
                rejected_entries = rejected_entries.saturating_add(1);
                continue;
            }
            let metadata = entry.metadata().map_err(|_| DirectoryCacheError::Io)?;
            if metadata.len() > MAXIMUM_CACHE_ENTRY_BYTES as u64 {
                rejected_entries = rejected_entries.saturating_add(1);
                continue;
            }
            let bytes = fs::read(entry.path()).map_err(|_| DirectoryCacheError::Io)?;
            match decode_record(&bytes) {
                Ok(record) if cache_filename(&record) == name => candidates.push(record),
                _ => rejected_entries = rejected_entries.saturating_add(1),
            }
        }
        candidates.sort_by(|left, right| {
            right
                .received_at
                .cmp(&left.received_at)
                .then_with(|| right.etag.cmp(&left.etag))
        });
        candidates.truncate(MAXIMUM_CACHE_CANDIDATES);
        Ok(CacheRead {
            candidates,
            rejected_entries,
        })
    }

    fn store(&mut self, record: &CachedDirectory) -> Result<(), DirectoryCacheError> {
        validate_record_shape(record)?;
        create_cache_root(&self.root)?;
        let encoded = encode_record(record)?;
        let final_path = self.root.join(cache_filename(record));
        if final_path.exists() {
            return existing_matches(&final_path, &encoded);
        }

        let mut temporary = TemporaryFile::create(&self.root)?;
        temporary.write_and_sync(&encoded)?;
        match fs::hard_link(&temporary.path, &final_path) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                existing_matches(&final_path, &encoded)?;
            }
            Err(_) => return Err(DirectoryCacheError::Io),
        }
        drop(temporary);
        sync_directory(&self.root)?;
        cleanup_cache(&self.root, &final_path);
        Ok(())
    }
}

struct TemporaryFile {
    path: PathBuf,
    file: File,
}

impl TemporaryFile {
    fn create(root: &Path) -> Result<Self, DirectoryCacheError> {
        for _ in 0..8 {
            let path = root.join(format!(
                "{TEMP_PREFIX}{}-{}",
                std::process::id(),
                TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            let mut options = OpenOptions::new();
            options.create_new(true).write(true);
            #[cfg(unix)]
            options.mode(0o600);
            match options.open(&path) {
                Ok(file) => return Ok(Self { path, file }),
                Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
                Err(_) => return Err(DirectoryCacheError::Io),
            }
        }
        Err(DirectoryCacheError::Conflict)
    }

    fn write_and_sync(&mut self, bytes: &[u8]) -> Result<(), DirectoryCacheError> {
        self.file
            .write_all(bytes)
            .and_then(|()| self.file.sync_all())
            .map_err(|_| DirectoryCacheError::Io)
    }
}

impl Drop for TemporaryFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn create_cache_root(root: &Path) -> Result<(), DirectoryCacheError> {
    fs::create_dir_all(root).map_err(|_| DirectoryCacheError::Io)?;
    validate_existing_cache_root(root)?;
    #[cfg(unix)]
    {
        let mut permissions = fs::metadata(root)
            .map_err(|_| DirectoryCacheError::Io)?
            .permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(root, permissions).map_err(|_| DirectoryCacheError::Io)?;
    }
    Ok(())
}

fn validate_existing_cache_root(root: &Path) -> Result<(), DirectoryCacheError> {
    let metadata = fs::symlink_metadata(root).map_err(|_| DirectoryCacheError::Io)?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(DirectoryCacheError::Io);
    }
    Ok(())
}

fn sync_directory(root: &Path) -> Result<(), DirectoryCacheError> {
    #[cfg(unix)]
    {
        File::open(root)
            .and_then(|directory| directory.sync_all())
            .map_err(|_| DirectoryCacheError::Io)?;
    }
    #[cfg(not(unix))]
    let _ = root;
    Ok(())
}

fn cleanup_cache(root: &Path, current: &Path) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    let now = SystemTime::now();
    let mut valid = Vec::new();
    for entry in entries.flatten().take(MAXIMUM_CACHE_SCAN + 1) {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if name.starts_with(TEMP_PREFIX) {
            if entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .ok()
                .and_then(|modified| now.duration_since(modified).ok())
                .is_some_and(|age| age > STALE_TEMP_LIFETIME)
            {
                let _ = fs::remove_file(entry.path());
            }
            continue;
        }
        if !recognized_cache_name(name) {
            continue;
        }
        let path = entry.path();
        let record = fs::read(&path)
            .ok()
            .filter(|bytes| bytes.len() <= MAXIMUM_CACHE_ENTRY_BYTES)
            .and_then(|bytes| decode_record(&bytes).ok())
            .filter(|record| cache_filename(record) == name);
        if let Some(record) = record {
            valid.push((record.received_at, record.etag, path));
        } else if path != current {
            let _ = fs::remove_file(path);
        }
    }
    valid.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| right.1.cmp(&left.1)));
    for (_, _, path) in valid.into_iter().skip(MAXIMUM_CACHE_CANDIDATES) {
        if path != current {
            let _ = fs::remove_file(path);
        }
    }
}

fn encode_record(record: &CachedDirectory) -> Result<Vec<u8>, DirectoryCacheError> {
    validate_record_shape(record)?;
    let published_at = record
        .published_at
        .ok_or(DirectoryCacheError::InvalidRecord)?;
    let mut output = Vec::with_capacity(record.body.len() + CACHE_METADATA_ALLOWANCE);
    output.extend_from_slice(CACHE_MAGIC_V2);
    output.extend_from_slice(format!("received-at:{}\n", record.received_at).as_bytes());
    output.extend_from_slice(format!("published-at:{published_at}\n").as_bytes());
    output.extend_from_slice(b"etag:");
    output.extend_from_slice(record.etag.as_bytes());
    output.extend_from_slice(b"\nbody-sha256:");
    output.extend_from_slice(lowercase_hex(&record.body_sha256).as_bytes());
    output.extend_from_slice(b"\nbody-bytes:");
    output.extend_from_slice(record.body.len().to_string().as_bytes());
    output.extend_from_slice(b"\n\n");
    output.extend_from_slice(&record.body);
    if output.len() > MAXIMUM_CACHE_ENTRY_BYTES {
        return Err(DirectoryCacheError::InvalidRecord);
    }
    Ok(output)
}

fn decode_record(input: &[u8]) -> Result<CachedDirectory, DirectoryCacheError> {
    if input.len() > MAXIMUM_CACHE_ENTRY_BYTES {
        return Err(DirectoryCacheError::InvalidRecord);
    }
    if input.starts_with(CACHE_MAGIC_V2) {
        decode_v2_record(input)
    } else if input.starts_with(CACHE_MAGIC_V1) {
        decode_v1_record(input)
    } else {
        Err(DirectoryCacheError::InvalidRecord)
    }
}

fn metadata_and_body<'a>(
    input: &'a [u8],
    magic: &[u8],
) -> Result<(&'a str, &'a [u8]), DirectoryCacheError> {
    let metadata_start = magic.len();
    let delimiter = input[metadata_start..]
        .windows(2)
        .position(|window| window == b"\n\n")
        .map(|position| metadata_start + position)
        .ok_or(DirectoryCacheError::InvalidRecord)?;
    let metadata = std::str::from_utf8(&input[metadata_start..delimiter])
        .map_err(|_| DirectoryCacheError::InvalidRecord)?;
    Ok((metadata, &input[delimiter + 2..]))
}

fn decode_v2_record(input: &[u8]) -> Result<CachedDirectory, DirectoryCacheError> {
    let (metadata, body) = metadata_and_body(input, CACHE_MAGIC_V2)?;
    let mut lines = metadata.lines();
    let received_at = canonical_u64(
        lines
            .next()
            .and_then(|line| line.strip_prefix("received-at:"))
            .ok_or(DirectoryCacheError::InvalidRecord)?,
    )?;
    let published_at = canonical_u64(
        lines
            .next()
            .and_then(|line| line.strip_prefix("published-at:"))
            .ok_or(DirectoryCacheError::InvalidRecord)?,
    )?;
    let etag = lines
        .next()
        .and_then(|line| line.strip_prefix("etag:"))
        .filter(|value| valid_strong_etag(value))
        .ok_or(DirectoryCacheError::InvalidRecord)?
        .to_owned();
    let body_sha256 = lines
        .next()
        .and_then(|line| line.strip_prefix("body-sha256:"))
        .and_then(decode_sha256)
        .ok_or(DirectoryCacheError::InvalidRecord)?;
    let body_length = canonical_usize(
        lines
            .next()
            .and_then(|line| line.strip_prefix("body-bytes:"))
            .ok_or(DirectoryCacheError::InvalidRecord)?,
    )?;
    if lines.next().is_some() || body_length > DIRECTORY_BODY_BYTES_LIMIT {
        return Err(DirectoryCacheError::InvalidRecord);
    }
    if body.len() != body_length {
        return Err(DirectoryCacheError::InvalidRecord);
    }
    let record = CachedDirectory {
        received_at,
        published_at: Some(published_at),
        etag,
        body_sha256,
        body: body.to_vec(),
    };
    validate_record_shape(&record)?;
    Ok(record)
}

fn decode_v1_record(input: &[u8]) -> Result<CachedDirectory, DirectoryCacheError> {
    let (metadata, body) = metadata_and_body(input, CACHE_MAGIC_V1)?;
    let mut lines = metadata.lines();
    let received_at = canonical_u64(
        lines
            .next()
            .and_then(|line| line.strip_prefix("received-at:"))
            .ok_or(DirectoryCacheError::InvalidRecord)?,
    )?;
    let etag = lines
        .next()
        .and_then(|line| line.strip_prefix("etag:"))
        .filter(|value| legacy_body_etag_digest(value).is_some())
        .ok_or(DirectoryCacheError::InvalidRecord)?
        .to_owned();
    let body_length = canonical_usize(
        lines
            .next()
            .and_then(|line| line.strip_prefix("body-bytes:"))
            .ok_or(DirectoryCacheError::InvalidRecord)?,
    )?;
    if lines.next().is_some()
        || body_length > DIRECTORY_BODY_BYTES_LIMIT
        || body.len() != body_length
    {
        return Err(DirectoryCacheError::InvalidRecord);
    }
    let body_sha256 = directory_body_sha256(body);
    if legacy_body_etag_digest(&etag) != Some(body_sha256) {
        return Err(DirectoryCacheError::InvalidRecord);
    }
    Ok(CachedDirectory {
        received_at,
        published_at: None,
        etag,
        body_sha256,
        body: body.to_vec(),
    })
}

fn validate_record_shape(record: &CachedDirectory) -> Result<(), DirectoryCacheError> {
    if record.published_at.is_none()
        || record.body.len() > DIRECTORY_BODY_BYTES_LIMIT
        || !valid_strong_etag(&record.etag)
        || directory_body_sha256(&record.body) != record.body_sha256
        || record.published_at.is_some_and(|published_at| {
            published_at
                > record
                    .received_at
                    .saturating_add(DIRECTORY_CLOCK_SKEW_SECONDS)
        })
    {
        return Err(DirectoryCacheError::InvalidRecord);
    }
    Ok(())
}

pub(crate) fn valid_strong_etag(value: &str) -> bool {
    (3..=128).contains(&value.len())
        && value.starts_with('"')
        && value.ends_with('"')
        && value[1..value.len() - 1]
            .bytes()
            .all(|current| (0x21..=0x7e).contains(&current) && !matches!(current, b'"' | b'\\'))
}

fn legacy_body_etag_digest(value: &str) -> Option<[u8; 32]> {
    const PREFIX: &str = "\"atrinik-directory-v1-sha256-";
    if value.len() != PREFIX.len() + 64 + 1 || !value.starts_with(PREFIX) || !value.ends_with('"') {
        return None;
    }
    decode_sha256(&value[PREFIX.len()..value.len() - 1])
}

fn decode_sha256(value: &str) -> Option<[u8; 32]> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|current| current.is_ascii_digit() || (b'a'..=b'f').contains(&current))
    {
        return None;
    }
    let mut output = [0u8; 32];
    for (destination, pair) in output.iter_mut().zip(value.as_bytes().chunks_exact(2)) {
        let high = hex_nibble(pair[0])?;
        let low = hex_nibble(pair[1])?;
        *destination = (high << 4) | low;
    }
    Some(output)
}

const fn hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        _ => None,
    }
}

fn canonical_u64(value: &str) -> Result<u64, DirectoryCacheError> {
    if value.is_empty()
        || (value.len() > 1 && value.starts_with('0'))
        || !value.bytes().all(|current| current.is_ascii_digit())
    {
        return Err(DirectoryCacheError::InvalidRecord);
    }
    value
        .parse()
        .map_err(|_| DirectoryCacheError::InvalidRecord)
}

fn canonical_usize(value: &str) -> Result<usize, DirectoryCacheError> {
    let parsed = canonical_u64(value)?;
    usize::try_from(parsed).map_err(|_| DirectoryCacheError::InvalidRecord)
}

fn recognized_cache_name(value: &str) -> bool {
    let Some(stem) = value
        .strip_prefix(CACHE_ENTRY_PREFIX)
        .and_then(|value| value.strip_suffix(CACHE_ENTRY_SUFFIX))
    else {
        return false;
    };
    let Some((received_at, digest)) = stem.split_once('-') else {
        return false;
    };
    received_at.len() == 20
        && received_at.bytes().all(|current| current.is_ascii_digit())
        && digest.len() == 64
        && digest
            .bytes()
            .all(|current| current.is_ascii_digit() || (b'a'..=b'f').contains(&current))
}

fn cache_filename(record: &CachedDirectory) -> String {
    let digest = record
        .published_at
        .map_or(record.body_sha256, |published_at| {
            let mut hasher = Sha256::new();
            hasher.update(b"ATRINIK-DIRECTORY-CACHE-KEY-V2\0");
            hasher.update(record.received_at.to_be_bytes());
            hasher.update(published_at.to_be_bytes());
            hasher.update(
                u16::try_from(record.etag.len())
                    .expect("validated ETag length fits u16")
                    .to_be_bytes(),
            );
            hasher.update(record.etag.as_bytes());
            hasher.update(record.body_sha256);
            hasher.finalize().into()
        });
    format!(
        "{CACHE_ENTRY_PREFIX}{:020}-{}{CACHE_ENTRY_SUFFIX}",
        record.received_at,
        lowercase_hex(&digest),
    )
}

fn existing_matches(path: &Path, expected: &[u8]) -> Result<(), DirectoryCacheError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| DirectoryCacheError::Io)?;
    if !metadata.file_type().is_file() || metadata.len() > MAXIMUM_CACHE_ENTRY_BYTES as u64 {
        return Err(DirectoryCacheError::Conflict);
    }
    let existing = fs::read(path).map_err(|_| DirectoryCacheError::Io)?;
    if existing == expected {
        Ok(())
    } else {
        Err(DirectoryCacheError::Conflict)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "atrinik-directory-cache-{name}-{}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn record(received_at: u64, body: &[u8]) -> CachedDirectory {
        CachedDirectory {
            received_at,
            published_at: Some(received_at.saturating_sub(1)),
            etag: format!("\"origin-object-{received_at}\""),
            body_sha256: directory_body_sha256(body),
            body: body.to_vec(),
        }
    }

    #[test]
    fn record_codec_is_exact_bounded_and_rejects_noncanonical_metadata() {
        let expected = record(42, b"public directory\n");
        let encoded = encode_record(&expected).expect("encode");
        assert_eq!(decode_record(&encoded), Ok(expected.clone()));
        assert!(encoded.starts_with(CACHE_MAGIC_V2));

        let invalid = [
            b"ATRINIK-DIRECTORY-CACHE-V2\nreceived-at:042\npublished-at:41\netag:\"origin\"\nbody-sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\nbody-bytes:0\n\n".as_slice(),
            b"ATRINIK-DIRECTORY-CACHE-V2\nreceived-at:42\npublished-at:41\netag:W/\"origin\"\nbody-sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\nbody-bytes:0\n\n".as_slice(),
            b"ATRINIK-DIRECTORY-CACHE-V2\nreceived-at:42\npublished-at:41\netag:\"origin\"\nbody-sha256:E3B0C44298FC1C149AFBF4C8996FB92427AE41E4649B934CA495991B7852B855\nbody-bytes:0\n\n".as_slice(),
            b"ATRINIK-DIRECTORY-CACHE-V2\nreceived-at:42\npublished-at:41\netag:\"origin\"\nbody-sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\nbody-bytes:0\n\n".as_slice(),
            b"ATRINIK-DIRECTORY-CACHE-V2\nreceived-at:42\npublished-at:343\netag:\"origin\"\nbody-sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\nbody-bytes:0\n\n".as_slice(),
        ];
        for value in invalid {
            assert_eq!(
                decode_record(value),
                Err(DirectoryCacheError::InvalidRecord)
            );
        }
    }

    #[test]
    fn verified_v1_record_remains_a_last_known_good_migration_candidate() {
        let body = b"legacy directory\n";
        let digest = lowercase_hex(&directory_body_sha256(body));
        let mut encoded = format!(
            "ATRINIK-DIRECTORY-CACHE-V1\nreceived-at:42\n\
             etag:\"atrinik-directory-v1-sha256-{digest}\"\n\
             body-bytes:{}\n\n",
            body.len(),
        )
        .into_bytes();
        encoded.extend_from_slice(body);

        assert_eq!(
            decode_record(&encoded),
            Ok(CachedDirectory {
                received_at: 42,
                published_at: None,
                etag: format!("\"atrinik-directory-v1-sha256-{digest}\""),
                body_sha256: directory_body_sha256(body),
                body: body.to_vec(),
            }),
        );

        let wrong = encoded
            .iter()
            .copied()
            .take(encoded.len() - 1)
            .chain(*b"!")
            .collect::<Vec<_>>();
        assert_eq!(
            decode_record(&wrong),
            Err(DirectoryCacheError::InvalidRecord),
        );
    }

    #[test]
    fn verified_v1_filename_loads_and_v2_metadata_cannot_collide() {
        let root = test_root("versioned-filenames");
        create_cache_root(&root).expect("root");
        let body = b"legacy directory\n";
        let digest = lowercase_hex(&directory_body_sha256(body));
        let mut encoded = format!(
            "ATRINIK-DIRECTORY-CACHE-V1\nreceived-at:42\n\
             etag:\"atrinik-directory-v1-sha256-{digest}\"\n\
             body-bytes:{}\n\n",
            body.len(),
        )
        .into_bytes();
        encoded.extend_from_slice(body);
        fs::write(
            root.join(format!(
                "{CACHE_ENTRY_PREFIX}{:020}-{digest}{CACHE_ENTRY_SUFFIX}",
                42,
            )),
            encoded,
        )
        .expect("write legacy cache");

        let mut cache = FileDirectoryCache::new(&root);
        let loaded = cache.load().expect("load legacy cache");
        assert_eq!(loaded.candidates.len(), 1);
        assert_eq!(loaded.candidates[0].published_at, None);

        let first = record(43, body);
        let mut second = first.clone();
        second.etag = "\"replacement-object\"".to_owned();
        assert_ne!(cache_filename(&first), cache_filename(&second));
        cache.store(&first).expect("store first v2");
        cache.store(&second).expect("store second v2");
        let loaded = cache.load().expect("load v2 records");
        assert_eq!(loaded.candidates.len(), 3);
        assert!(loaded.candidates.contains(&first));
        assert!(loaded.candidates.contains(&second));
        fs::remove_dir_all(root).expect("remove owned test cache");
    }

    #[test]
    fn strong_etag_syntax_is_opaque_bounded_and_exact() {
        assert!(valid_strong_etag("\"a\""));
        assert!(valid_strong_etag(&format!("\"{}\"", "x".repeat(126))));
        for invalid in [
            String::new(),
            "\"\"".to_owned(),
            "W/\"a\"".to_owned(),
            "\"with space\"".to_owned(),
            "\"with\\slash\"".to_owned(),
            "\"with\"quote\"".to_owned(),
            "\"é\"".to_owned(),
            format!("\"{}\"", "x".repeat(127)),
        ] {
            assert!(!valid_strong_etag(&invalid), "{invalid:?}");
        }
    }

    #[test]
    fn filesystem_cache_publishes_atomically_and_retains_four_newest() {
        let root = test_root("atomic");
        let mut cache = FileDirectoryCache::new(&root);
        for timestamp in 1..=6 {
            cache
                .store(&record(timestamp, format!("body-{timestamp}").as_bytes()))
                .expect("store");
        }
        let read = cache.load().expect("load");
        assert_eq!(read.rejected_entries, 0);
        assert_eq!(
            read.candidates
                .iter()
                .map(|value| value.received_at)
                .collect::<Vec<_>>(),
            vec![6, 5, 4, 3]
        );
        #[cfg(unix)]
        assert_eq!(
            fs::metadata(&root)
                .expect("root metadata")
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        fs::remove_dir_all(root).expect("remove owned test cache");
    }

    #[test]
    fn corrupt_newest_entry_is_reported_without_hiding_older_lkg() {
        let root = test_root("corrupt");
        let mut cache = FileDirectoryCache::new(&root);
        let older = record(10, b"older");
        cache.store(&older).expect("older");
        create_cache_root(&root).expect("root");
        fs::write(
            root.join(format!(
                "{CACHE_ENTRY_PREFIX}{:020}-{}{}",
                11,
                "a".repeat(64),
                CACHE_ENTRY_SUFFIX
            )),
            b"corrupt",
        )
        .expect("write corrupt fixture");
        let read = cache.load().expect("load");
        assert_eq!(read.rejected_entries, 1);
        assert_eq!(read.candidates, vec![older]);
        fs::remove_dir_all(root).expect("remove owned test cache");
    }

    #[cfg(unix)]
    #[test]
    fn symlink_entries_and_conflicts_are_never_followed() {
        use std::os::unix::fs::symlink;

        let root = test_root("symlink");
        create_cache_root(&root).expect("root");
        let record = record(10, b"directory");
        let final_path = root.join(cache_filename(&record));
        let target = root.join("target");
        fs::write(&target, b"unrelated").expect("target");
        symlink(&target, &final_path).expect("symlink");

        let mut cache = FileDirectoryCache::new(&root);
        let read = cache.load().expect("load");
        assert_eq!(read.rejected_entries, 1);
        assert!(read.candidates.is_empty());
        assert_eq!(cache.store(&record), Err(DirectoryCacheError::Conflict));
        assert_eq!(fs::read(target).expect("target preserved"), b"unrelated");
        fs::remove_dir_all(root).expect("remove owned test cache");
    }

    #[cfg(unix)]
    #[test]
    fn symlink_cache_root_is_never_followed_or_repermissioned() {
        use std::os::unix::fs::symlink;

        let parent = test_root("root-symlink-parent");
        let target = parent.join("target");
        let link = parent.join("link");
        fs::create_dir_all(&target).expect("target");
        let mut permissions = fs::metadata(&target).expect("metadata").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&target, permissions).expect("mode");
        symlink(&target, &link).expect("symlink");
        let mut cache = FileDirectoryCache::new(&link);
        assert_eq!(cache.load(), Err(DirectoryCacheError::Io));
        assert_eq!(
            cache.store(&record(1, b"body")),
            Err(DirectoryCacheError::Io)
        );
        assert_eq!(
            fs::metadata(&target)
                .expect("target metadata")
                .permissions()
                .mode()
                & 0o777,
            0o755
        );
        fs::remove_dir_all(parent).expect("remove owned test cache");
    }

    #[test]
    fn stored_cache_contains_only_the_public_snapshot_envelope() {
        let root = test_root("privacy");
        let body = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/metaserver-directory-v1/canonical.json"
        ));
        let mut cache = FileDirectoryCache::new(&root);
        cache.store(&record(42, body)).expect("store");
        let path = fs::read_dir(&root)
            .expect("read cache")
            .filter_map(Result::ok)
            .find(|entry| recognized_cache_name(entry.file_name().to_str().unwrap_or_default()))
            .expect("cache entry")
            .path();
        let persisted = fs::read(path).expect("read entry");
        for forbidden in [
            b"rendezvousToken".as_slice(),
            b"ticket".as_slice(),
            b"candidate".as_slice(),
            b"invite".as_slice(),
            b"sourceIp".as_slice(),
        ] {
            assert!(
                !persisted
                    .windows(forbidden.len())
                    .any(|value| value == forbidden)
            );
        }
        fs::remove_dir_all(root).expect("remove owned test cache");
    }
}
