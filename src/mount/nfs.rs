use crate::mount::Mounter;
use crate::mount::error::MountError;
use crate::syscall::{MountOptions, MountSource, Syscall};
use tonic::async_trait;
use tonic::transport::Uri;

#[derive(Debug, Default)]
pub struct NfsMounter {
    syscall: Syscall,
}

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
        self.syscall
            .mount(MountSource::nfs(&src), mount_point, MountOptions::default())
            .await?;
        Ok(())
    }
}
