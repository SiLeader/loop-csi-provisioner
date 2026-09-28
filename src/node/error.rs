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
            NodeError::Mount(e) => tonic::Status::internal(format!("Mount error: {}", e)),
            NodeError::VolumeIdParse => {
                tonic::Status::invalid_argument("Failed to parse volume ID")
            }
        }
    }
}
