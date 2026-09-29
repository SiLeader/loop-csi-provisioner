pub(crate) use crate::mount::error::MountError;

use std::collections::HashMap;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::str::FromStr;
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

pub(crate) struct MountManager {
    mounters: HashMap<String, Box<dyn Mounter>>,
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
        }
    }

    pub async fn mount(&self, url: &str, base: &str) -> Result<String, MountError> {
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
        if let Some(mounter) = self.mounters.get(scheme) {
            let mount_point = mounter.mount_point(&uri, base)?;
            if scheme == "file" {
                mounter.mount(&uri, &mount_point).await?;
                return Ok(mount_point);
            }
            tokio::fs::create_dir_all(&mount_point).await?;
            if self.check_mount_point(&mount_point).await? {
                return Ok(mount_point);
            }
            mounter.mount(&uri, &mount_point).await?;
            Ok(mount_point)
        } else {
            Err(MountError::UnsupportedProtocol(scheme.to_string()))
        }
    }

    pub async fn check_mount_point(&self, target: &str) -> Result<bool, MountError> {
        is_mountpoint(target).await
    }
}

async fn is_mountpoint(path: impl AsRef<Path>) -> Result<bool, MountError> {
    let path = path.as_ref();

    let meta = tokio::fs::metadata(path).await?;
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
