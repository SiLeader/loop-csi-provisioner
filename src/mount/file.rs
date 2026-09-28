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
        tokio::fs::symlink(source.path(), mount_point).await?;
        Ok(())
    }

    async fn unmount(&self, mount_point: &str) -> Result<(), MountError> {
        tokio::fs::remove_file(mount_point).await?;
        Ok(())
    }
}
