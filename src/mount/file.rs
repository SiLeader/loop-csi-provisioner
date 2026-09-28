use crate::mount::Mounter;
use crate::mount::error::MountError;
use tonic::async_trait;
use tonic::transport::Uri;

pub struct FileMounter;

#[async_trait]
impl Mounter for FileMounter {
    fn protocol_scheme(&self) -> &str {
        "file"
    }

    fn mount_point(&self, source: &Uri, base: &str) -> Result<String, MountError> {
        let path = source
            .path()
            .replace("/", "-")
            .trim_matches('-')
            .to_string();
        let mount_point = format!("{}/{}", base, path);
        Ok(mount_point)
    }

    async fn mount(&self, source: &Uri, mount_point: &str) -> Result<(), MountError> {
        let target = std::path::Path::new(source.path());
        if !target.is_absolute() || !target.is_dir() {
            return Err(MountError::InvalidFilePath(source.path().to_string()));
        }
        if let Some(parent) = std::path::Path::new(mount_point).parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        match tokio::fs::symlink_metadata(mount_point).await {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                if tokio::fs::read_link(mount_point).await? != target {
                    return Err(MountError::InvalidFilePath(source.path().to_string()));
                }
            }
            Ok(_) => return Err(MountError::InvalidFilePath(source.path().to_string())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tokio::fs::symlink(target, mount_point).await?;
            }
            Err(e) => return Err(e.into()),
        }
        Ok(())
    }
}
