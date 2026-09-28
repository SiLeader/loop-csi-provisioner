use std::process::Output;
use tonic::codegen::http::uri::InvalidUri;

#[derive(Debug, thiserror::Error)]
pub(crate) enum MountError {
    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    InvalidUri(#[from] InvalidUri),

    #[error("Schema is missing in the URI")]
    SchemaIsMissing,

    #[error("Host is missing in the URI")]
    HostIsMissing,

    #[error("Unsupported protocol: {0}")]
    UnsupportedProtocol(String),

    #[error("Command failed with code {code}: {message}")]
    CommandFailure { code: i32, message: String },
}

impl From<Output> for MountError {
    fn from(output: Output) -> Self {
        let code = output.status.code().unwrap_or(-1);
        let message = String::from_utf8_lossy(&output.stderr).to_string();
        MountError::CommandFailure { code, message }
    }
}
