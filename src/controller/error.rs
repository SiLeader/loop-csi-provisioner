#[derive(Debug, thiserror::Error)]
pub(crate) enum ControllerError {
    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),

    #[error("Volume {0} not found")]
    NotFound(String),

    #[error(transparent)]
    Mount(#[from] crate::mount::MountError),

    #[error(
        "Volume {volume_id} is already attached to node {attached_node}, cannot attach to node {requested_node}"
    )]
    AlreadyAttached {
        volume_id: String,
        attached_node: String,
        requested_node: String,
    },

    #[error("Volume {volume_id} is still attached to node {attached_node}")]
    StillAttached {
        volume_id: String,
        attached_node: String,
    },

    #[error("Failed to parse volume ID")]
    VolumeIdParse,

    #[error("Existing volume size {0} exceeds requested size {1}")]
    ExistingSize(i64, i64),
}

impl From<ControllerError> for tonic::Status {
    fn from(err: ControllerError) -> Self {
        match err {
            ControllerError::Io(e) => tonic::Status::internal(format!("IO error: {}", e)),
            ControllerError::Json(e) => tonic::Status::internal(format!("JSON error: {}", e)),
            ControllerError::NotFound(volume_id) => {
                tonic::Status::not_found(format!("Volume {} not found", volume_id))
            }
            ControllerError::Mount(crate::mount::MountError::UrlNotAllowed(url)) => {
                tonic::Status::permission_denied(format!("Storage URL {} is not allowed", url))
            }
            ControllerError::Mount(
                e @ (crate::mount::MountError::InvalidUrl(_)
                | crate::mount::MountError::UnsupportedProtocol(_)),
            ) => tonic::Status::invalid_argument(format!("Mount error: {}", e)),
            ControllerError::Mount(e) => tonic::Status::internal(format!("Mount error: {}", e)),
            ControllerError::AlreadyAttached {
                volume_id,
                attached_node,
                requested_node,
            } => tonic::Status::failed_precondition(format!(
                "Volume {} is already attached to node {}, cannot attach to node {}",
                volume_id, attached_node, requested_node
            )),
            ControllerError::StillAttached {
                volume_id,
                attached_node,
            } => tonic::Status::failed_precondition(format!(
                "Volume {} is still attached to node {}",
                volume_id, attached_node
            )),
            ControllerError::VolumeIdParse => {
                tonic::Status::invalid_argument("Failed to parse volume ID")
            }
            ControllerError::ExistingSize(current, requested) => tonic::Status::already_exists(
                format!("Existing volume size {current} exceeds requested size {requested}"),
            ),
        }
    }
}
