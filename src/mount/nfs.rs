use crate::mount::Mounter;
use crate::mount::error::MountError;
use tokio::process::Command;
use tonic::async_trait;
use tonic::transport::Uri;

pub struct NfsMounter;

#[async_trait]
impl Mounter for NfsMounter {
    fn protocol_scheme(&self) -> &str {
        "nfs"
    }

    fn mount_point(&self, source: &Uri, base: &str) -> Result<String, MountError> {
        let Some(host) = source.host() else {
            return Err(MountError::HostIsMissing);
        };
        let path = source
            .path()
            .replace("/", "-")
            .trim_matches('-')
            .to_string();
        let mount_point = format!("{}/{}-{}", base, host, path);
        Ok(mount_point)
    }

    async fn mount(&self, source: &Uri, mount_point: &str) -> Result<(), MountError> {
        let Some(host) = source.host() else {
            return Err(MountError::HostIsMissing);
        };
        let src = format!("{}:{}", host, source.path());
        let res = Command::new("mount")
            .args(["-t", "nfs", &src, mount_point])
            .output()
            .await?;
        if res.status.success() {
            Ok(())
        } else {
            Err(res.into())
        }
    }

    async fn unmount(&self, mount_point: &str) -> Result<(), MountError> {
        let res = Command::new("umount").arg(mount_point).output().await?;

        if res.status.success() {
            Ok(())
        } else {
            Err(res.into())
        }
    }
}
