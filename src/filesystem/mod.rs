use std::process::Output;
use tokio::process::Command;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum Filesystem {
    Ext4,
    /// Recognized so it is never formatted over, but not supported for mounting or resizing.
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

fn unsupported(filesystem: Filesystem) -> FsError {
    FsError::Io(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        format!("{filesystem:?} is not supported"),
    ))
}

impl FilesystemManager {
    pub async fn resize(&self, filesystem: Filesystem, device: &str) -> Result<(), FsError> {
        if filesystem != Filesystem::Ext4 {
            return Err(unsupported(filesystem));
        }
        let res = Command::new("resize2fs")
            .arg("--")
            .arg(device)
            .output()
            .await?;
        if res.status.success() {
            Ok(())
        } else {
            Err(res.into())
        }
    }

    /// Formats `device`. Callers must make sure the device holds no data worth keeping:
    /// no `-F`-style override is passed, so `mkfs` also refuses devices it finds in use.
    ///
    /// The image lives on shared storage that other nodes can attach too, e.g. after a
    /// node that stopped responding was force-detached. Multi-mount protection (`mmp`)
    /// makes the kernel refuse to mount the filesystem while another host has it mounted,
    /// instead of letting two hosts corrupt it. A cleanly unmounted filesystem mounts at
    /// once; otherwise (in use elsewhere, or its node crashed) the mount first waits a few
    /// MMP intervals, typically tens of seconds, to see whether someone is still using it.
    pub async fn create(
        &self,
        filesystem: Filesystem,
        device: &str,
    ) -> Result<Filesystem, FsError> {
        if filesystem != Filesystem::Ext4 {
            return Err(unsupported(filesystem));
        }
        let res = Command::new("mkfs.ext4")
            .args(["-O", "mmp", "--"])
            .arg(device)
            .output()
            .await?;
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
