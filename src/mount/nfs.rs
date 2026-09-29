use crate::mount::error::MountError;
use crate::mount::{Mounter, encode_component};
use std::time::Duration;
use tokio::process::Command;
use tonic::async_trait;
use tonic::transport::Uri;

const MOUNT_TIMEOUT: Duration = Duration::from_secs(60);

/// Mounts NFS exports with the `mount` helper (`mount.nfs`), which resolves host names
/// and negotiates the protocol version; the raw `mount(2)` call does neither.
#[derive(Debug, Default)]
pub struct NfsMounter {}

#[async_trait]
impl Mounter for NfsMounter {
    fn protocol_scheme(&self) -> &str {
        "nfs"
    }

    fn mount_point(&self, source: &Uri, base: &str) -> Result<String, MountError> {
        let Some(authority) = source.authority() else {
            return Err(MountError::HostIsMissing);
        };
        if source.host().is_none() {
            return Err(MountError::HostIsMissing);
        }
        let name = encode_component(&format!("{}{}", authority, source.path()))?;
        Ok(format!("{}/nfs-{}", base, name))
    }

    async fn mount(&self, source: &Uri, mount_point: &str) -> Result<(), MountError> {
        let Some(host) = source.host() else {
            return Err(MountError::HostIsMissing);
        };
        let src = format!("{}:{}", host, source.path());

        let mut command = Command::new("mount");
        command.args(["-t", "nfs"]);
        if let Some(port) = source.port_u16() {
            command.arg("-o").arg(format!("port={port}"));
        }
        command
            .arg("--")
            .arg(&src)
            .arg(mount_point)
            .kill_on_drop(true);

        let output = tokio::time::timeout(MOUNT_TIMEOUT, command.output())
            .await
            .map_err(|_| MountError::CommandFailure {
                code: -1,
                message: format!("mounting {src} timed out after {MOUNT_TIMEOUT:?}"),
            })??;
        if output.status.success() {
            Ok(())
        } else {
            Err(output.into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn point(url: &str) -> String {
        NfsMounter::default()
            .mount_point(&Uri::from_str(url).unwrap(), "/base")
            .unwrap()
    }

    #[test]
    fn mount_points_do_not_collide() {
        let names = [
            point("nfs://h/a/b"),
            point("nfs://h/a-b"),
            point("nfs://h/a_b"),
            point("nfs://h:2049/a/b"),
            point("nfs://h2/a/b"),
        ];
        for (i, a) in names.iter().enumerate() {
            for b in &names[i + 1..] {
                assert_ne!(a, b);
            }
        }
    }
}
