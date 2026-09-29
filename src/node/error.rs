use crate::filesystem::FsError;
use std::process::Output;

#[derive(Debug, thiserror::Error)]
pub(crate) enum NodeError {
    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error("Command failed with code {code}: {message}")]
    CommandFailure { code: i32, message: String },

    #[error(transparent)]
    Mount(#[from] crate::mount::MountError),

    #[error("Failed to parse volume ID")]
    VolumeIdParse,

    #[error("Only ext4 filesystems are supported")]
    UnsupportedFilesystem,

    #[error("Volume {0} not found")]
    NotFound(String),

    #[error("Volume {0} is not staged")]
    NotStaged(String),

    #[error("Volume is not staged at {0}")]
    NotStagedAt(String),

    #[error("Device {0} holds data that is not a supported filesystem; refusing to format it")]
    NotBlank(String),

    #[error("Device {0} has no filesystem and the volume is read-only; refusing to format it")]
    ReadOnlyUnformatted(String),

    #[error("Loop device {device} has {actual} bytes but the volume has {expected}")]
    CapacityNotVisible {
        device: String,
        actual: u64,
        expected: u64,
    },

    #[error(transparent)]
    Fs(#[from] FsError),
}

impl From<Output> for NodeError {
    fn from(output: Output) -> Self {
        let code = output.status.code().unwrap_or(-1);
        let message = String::from_utf8_lossy(&output.stderr).to_string();
        NodeError::CommandFailure { code, message }
    }
}

impl From<NodeError> for tonic::Status {
    fn from(err: NodeError) -> Self {
        match err {
            NodeError::Io(e) => tonic::Status::internal(format!("IO error: {}", e)),
            NodeError::CommandFailure { code, message } => {
                tonic::Status::internal(format!("Command failed with code {}: {}", code, message))
            }
            NodeError::VolumeIdParse => {
                tonic::Status::invalid_argument("Failed to parse volume ID")
            }
            NodeError::UnsupportedFilesystem => {
                tonic::Status::failed_precondition("Only ext4 filesystems are supported")
            }
            NodeError::NotFound(volume_id) => {
                tonic::Status::not_found(format!("Volume {} not found", volume_id))
            }
            NodeError::NotStaged(volume_id) => {
                tonic::Status::failed_precondition(format!("Volume {} is not staged", volume_id))
            }
            NodeError::NotStagedAt(path) => {
                tonic::Status::failed_precondition(format!("Volume is not staged at {}", path))
            }
            NodeError::NotBlank(device) => tonic::Status::failed_precondition(format!(
                "Device {} holds data that is not a supported filesystem; refusing to format it",
                device
            )),
            e @ NodeError::ReadOnlyUnformatted(_) => {
                tonic::Status::failed_precondition(e.to_string())
            }
            // Usually the node has not yet seen the size the controller set; a retry helps.
            e @ NodeError::CapacityNotVisible { .. } => tonic::Status::unavailable(e.to_string()),
            NodeError::Mount(crate::mount::MountError::UrlNotAllowed(url)) => {
                tonic::Status::permission_denied(format!("Storage URL {} is not allowed", url))
            }
            NodeError::Mount(e @ crate::mount::MountError::Unresponsive(_)) => {
                tonic::Status::unavailable(format!("Mount error: {}", e))
            }
            NodeError::Mount(e) => tonic::Status::internal(format!("Mount error: {}", e)),
            NodeError::Fs(e) => tonic::Status::internal(format!("Fs error: {}", e)),
        }
    }
}
