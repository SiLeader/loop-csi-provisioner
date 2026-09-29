pub(crate) use crate::mount::error::MountError;

use crate::lock::KeyedLocks;
use std::collections::HashMap;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::str::FromStr;
use std::time::Duration;
use tonic::async_trait;
use tonic::transport::Uri;

mod error;
mod file;
mod nfs;

#[async_trait]
pub(crate) trait Mounter: Send + Sync {
    fn protocol_scheme(&self) -> &str;

    fn mount_point(&self, source: &Uri, base: &str) -> Result<String, MountError>;

    async fn mount(&self, source: &Uri, mount_point: &str) -> Result<(), MountError>;
}

/// Longest encoded mount point directory name; file names are limited to 255 bytes.
const MAX_MOUNT_POINT_NAME: usize = 200;

/// How long checking an existing mount point may take. A `stat` on a hard-mounted NFS
/// export whose server is gone never returns; this bounds how long a request waits for it.
const MOUNT_POINT_CHECK_TIMEOUT: Duration = Duration::from_secs(30);

/// Encodes `value` into a single path component. Every byte except ASCII alphanumerics
/// and `.` becomes `%XX`, so distinct inputs always yield distinct names.
pub(super) fn encode_component(value: &str) -> Result<String, MountError> {
    use std::fmt::Write;
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || byte == b'.' {
            encoded.push(byte as char);
        } else {
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    if encoded.len() > MAX_MOUNT_POINT_NAME {
        return Err(MountError::InvalidUrl(format!("{value} is too long")));
    }
    Ok(encoded)
}

/// Returns true when `prefixes` is empty or `url` equals a prefix or lies below it.
/// Matching is done on `/` boundaries, so `nfs://h/a` does not allow `nfs://h/ab`.
fn url_allowed(prefixes: &[String], url: &str) -> bool {
    prefixes.is_empty()
        || prefixes.iter().any(|prefix| {
            let prefix = prefix.trim_end_matches('/');
            url == prefix
                || url
                    .strip_prefix(prefix)
                    .is_some_and(|rest| rest.starts_with('/'))
        })
}

pub(crate) struct MountManager {
    mounters: HashMap<String, Box<dyn Mounter>>,
    allowed_prefixes: Vec<String>,
    /// Serializes mounting per mount point so concurrent requests cannot stack the same
    /// mount twice, while a slow or unreachable server only delays its own volumes.
    locks: KeyedLocks,
}

impl Default for MountManager {
    fn default() -> Self {
        let mounters: Vec<Box<dyn Mounter>> = vec![
            Box::new(file::FileMounter),
            Box::new(nfs::NfsMounter::default()),
        ];
        MountManager::new(mounters)
    }
}

impl MountManager {
    pub fn new(mounters: Vec<Box<dyn Mounter + 'static>>) -> Self {
        let mut mounters_map = HashMap::new();
        for mounter in mounters {
            mounters_map.insert(mounter.protocol_scheme().to_string(), mounter);
        }
        MountManager {
            mounters: mounters_map,
            allowed_prefixes: Vec::new(),
            locks: KeyedLocks::default(),
        }
    }

    /// Restricts the storage URLs that may be mounted. Empty means unrestricted.
    pub fn with_allowed_prefixes(mut self, prefixes: Vec<String>) -> Self {
        self.allowed_prefixes = prefixes;
        self
    }

    pub async fn mount(&self, url: &str, base: &str) -> Result<String, MountError> {
        if !url_allowed(&self.allowed_prefixes, url) {
            return Err(MountError::UrlNotAllowed(url.to_string()));
        }
        let normalized = if let Some(path) = url.strip_prefix("file:///") {
            format!("file://localhost/{path}")
        } else if url.starts_with("file://") {
            return Err(MountError::InvalidFilePath(url.to_string()));
        } else {
            url.to_string()
        };
        let uri = Uri::from_str(&normalized)?;
        let Some(scheme) = uri.scheme_str() else {
            return Err(MountError::SchemaIsMissing);
        };
        if uri.query().is_some() || uri.path().split('/').any(|s| s == "." || s == "..") {
            return Err(MountError::InvalidUrl(url.to_string()));
        }
        let Some(mounter) = self.mounters.get(scheme) else {
            return Err(MountError::UnsupportedProtocol(scheme.to_string()));
        };
        let mount_point = mounter.mount_point(&uri, base)?;
        let _guard = self.locks.lock(&mount_point).await;
        if scheme == "file" {
            mounter.mount(&uri, &mount_point).await?;
            return Ok(mount_point);
        }
        let mounted = tokio::time::timeout(MOUNT_POINT_CHECK_TIMEOUT, async {
            tokio::fs::create_dir_all(&mount_point).await?;
            self.check_mount_point(&mount_point).await
        })
        .await
        .map_err(|_| MountError::Unresponsive(mount_point.clone()))??;
        if !mounted {
            mounter.mount(&uri, &mount_point).await?;
        }
        Ok(mount_point)
    }

    pub async fn check_mount_point(&self, target: &str) -> Result<bool, MountError> {
        is_mountpoint(target).await
    }
}

async fn is_mountpoint(path: impl AsRef<Path>) -> Result<bool, MountError> {
    let path = path.as_ref();

    let meta = match tokio::fs::metadata(path).await {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(MountError::Io(e)),
    };
    if !meta.is_dir() {
        return Ok(false);
    }

    let parent = match path.parent() {
        Some(p) if p != path => p,
        _ => return Ok(true),
    };

    let parent_meta = tokio::fs::metadata(parent).await?;

    Ok(meta.dev() != parent_meta.dev())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoding_is_injective_and_slash_free() {
        assert_eq!(encode_component("/a/b").unwrap(), "%2Fa%2Fb");
        assert_ne!(
            encode_component("/a-b").unwrap(),
            encode_component("/a/b").unwrap()
        );
        assert_ne!(
            encode_component("%2F").unwrap(),
            encode_component("/").unwrap()
        );
        assert!(encode_component(&"a".repeat(300)).is_err());
    }

    #[test]
    fn allowed_prefixes_match_on_path_boundaries() {
        let prefixes = vec!["nfs://h/export/".to_string(), "file:///srv".to_string()];
        assert!(url_allowed(&[], "anything://x"));
        assert!(url_allowed(&prefixes, "nfs://h/export"));
        assert!(url_allowed(&prefixes, "nfs://h/export/sub"));
        assert!(url_allowed(&prefixes, "file:///srv/data"));
        assert!(!url_allowed(&prefixes, "nfs://h/export-evil"));
        assert!(!url_allowed(&prefixes, "nfs://h:2049/export"));
        assert!(!url_allowed(&prefixes, "file:///srvx"));
    }

    #[tokio::test]
    async fn rejects_disallowed_and_traversing_urls() {
        let manager = MountManager::default().with_allowed_prefixes(vec!["file:///srv".into()]);
        assert!(matches!(
            manager.mount("file:///etc", "/nonexistent").await,
            Err(MountError::UrlNotAllowed(_))
        ));
        assert!(matches!(
            manager.mount("file:///srv/../etc", "/nonexistent").await,
            Err(MountError::InvalidUrl(_))
        ));
        assert!(matches!(
            MountManager::default()
                .mount("nfs://h/x?vers=4", "/nonexistent")
                .await,
            Err(MountError::InvalidUrl(_))
        ));
    }

    #[tokio::test]
    async fn is_mountpoint_handles_missing_files_and_plain_dirs() {
        let dir = std::env::temp_dir().join(format!("loop-csi-mp-{}", std::process::id()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        assert!(!is_mountpoint(dir.join("missing")).await.unwrap());
        assert!(!is_mountpoint(&dir).await.unwrap());
        assert!(is_mountpoint("/").await.unwrap());
        tokio::fs::remove_dir_all(dir).await.unwrap();
    }
}
