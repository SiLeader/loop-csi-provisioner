use std::process::Output;
use tokio::process::Command;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum Filesystem {
    Ext4,
    Xfs,
}

#[derive(Debug, Default)]
pub(crate) struct FilesystemManager {}

#[derive(Debug, thiserror::Error)]
#[error("Failed to execute command: exit code: {status}: {stderr}")]
pub(crate) struct CommandError {
    pub(crate) stderr: String,
    pub(crate) status: i32,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum FsError {
    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Command(CommandError),
}

impl FilesystemManager {
    pub async fn resize(&self, filesystem: Filesystem, device: &str) -> Result<(), FsError> {
        let res = match filesystem {
            Filesystem::Ext4 => Command::new("resize2fs").arg(device).output().await?,
            Filesystem::Xfs => Command::new("xfs_glowfs").arg(device).output().await?,
        };
        if res.status.success() {
            Ok(())
        } else {
            Err(res.into())
        }
    }

    pub async fn create(
        &self,
        filesystem: Filesystem,
        device: &str,
    ) -> Result<Filesystem, FsError> {
        let res = match filesystem {
            Filesystem::Ext4 => {
                Command::new("mkfs.ext4")
                    .args(["-F", device])
                    .output()
                    .await?
            }
            Filesystem::Xfs => {
                Command::new("mkfs.xfs")
                    .args(["-f", device])
                    .output()
                    .await?
            }
        };
        if res.status.success() {
            Ok(filesystem)
        } else {
            Err(res.into())
        }
    }
}

impl From<Output> for CommandError {
    fn from(output: Output) -> Self {
        let status = output.status.code().unwrap_or(-1);
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        Self { stderr, status }
    }
}

impl From<Output> for FsError {
    fn from(value: Output) -> Self {
        Self::Command(value.into())
    }
}
