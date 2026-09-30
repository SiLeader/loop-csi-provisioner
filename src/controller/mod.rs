mod error;
pub(crate) mod mutex;
pub(crate) mod operator;

use crate::capability::supported_capability;
use crate::controller::error::ControllerError;
use crate::controller::mutex::ControllerLeaseHolder;
use crate::controller::operator::ControllerOperator;
use crate::proto::csi::v1::CapacityRange;
use crate::proto::csi::v1::controller_server::Controller;
use crate::proto::csi::v1::{
    ControllerExpandVolumeRequest, ControllerExpandVolumeResponse,
    ControllerGetCapabilitiesRequest, ControllerGetCapabilitiesResponse,
    ControllerGetVolumeHealthRequest, ControllerGetVolumeHealthResponse,
    ControllerGetVolumeRequest, ControllerGetVolumeResponse, ControllerListVolumeHealthRequest,
    ControllerListVolumeHealthResponse, ControllerModifyVolumeRequest,
    ControllerModifyVolumeResponse, ControllerPublishVolumeRequest,
    ControllerPublishVolumeResponse, ControllerServiceCapability, ControllerUnpublishVolumeRequest,
    ControllerUnpublishVolumeResponse, CreateSnapshotRequest, CreateSnapshotResponse,
    CreateVolumeRequest, CreateVolumeResponse, DeleteSnapshotRequest, DeleteSnapshotResponse,
    DeleteVolumeRequest, DeleteVolumeResponse, GetCapacityRequest, GetCapacityResponse,
    GetSnapshotRequest, GetSnapshotResponse, ListSnapshotsRequest, ListSnapshotsResponse,
    ListVolumesRequest, ListVolumesResponse, ValidateVolumeCapabilitiesRequest,
    ValidateVolumeCapabilitiesResponse, Volume, controller_service_capability,
};
use crate::task::run_to_completion;
use crate::volume_id::valid_volume_name;
use std::collections::HashMap;
use std::sync::Arc;
use tonic::{Request, Response, Status, async_trait};

pub(crate) struct LoopCsiController {
    operator: Arc<ControllerOperator>,
    lease: ControllerLeaseHolder,
}

impl LoopCsiController {
    pub fn new(operator: ControllerOperator, lease: ControllerLeaseHolder) -> Self {
        Self {
            operator: Arc::new(operator),
            lease,
        }
    }

    async fn check_leadership(&self) -> Result<(), Status> {
        if self.lease.is_leader().await {
            Ok(())
        } else {
            Err(Status::unavailable(
                "This controller instance is not the leader",
            ))
        }
    }
}

/// Returns `(required_bytes, limit_bytes)`, with 0 meaning unspecified as in CSI.
fn capacity_range(range: Option<&CapacityRange>) -> Result<(i64, i64), Status> {
    let Some(range) = range else {
        return Ok((0, 0));
    };
    if range.required_bytes < 0
        || range.limit_bytes < 0
        || (range.limit_bytes > 0 && range.required_bytes > range.limit_bytes)
    {
        return Err(Status::invalid_argument("Invalid capacity range"));
    }
    Ok((range.required_bytes, range.limit_bytes))
}

#[async_trait]
impl Controller for LoopCsiController {
    async fn create_volume(
        &self,
        request: Request<CreateVolumeRequest>,
    ) -> Result<Response<CreateVolumeResponse>, Status> {
        self.check_leadership().await?;

        let request = request.into_inner();
        let Some(url) = request.parameters.get("url").cloned() else {
            return Err(Status::invalid_argument("Missing 'url' parameter"));
        };
        if !valid_volume_name(&request.name) {
            return Err(Status::invalid_argument("Invalid volume name"));
        }
        if request.volume_capabilities.is_empty()
            || !request.volume_capabilities.iter().all(supported_capability)
        {
            return Err(Status::invalid_argument(
                "Only ext4 mount volumes with single-node access are supported",
            ));
        }
        if request.volume_content_source.is_some() {
            return Err(Status::invalid_argument(
                "Creating volumes from snapshots or other volumes is not supported",
            ));
        }
        let (required, limit) = capacity_range(request.capacity_range.as_ref())?;
        let volume_id = format!("{}:{}", url, request.name);
        let operator = self.operator.clone();
        let (id, storage_url) = (volume_id.clone(), url.clone());
        let capacity_bytes = run_to_completion("CreateVolume", async move {
            operator
                .create_volume(&id, required, limit, &storage_url)
                .await
        })
        .await?;

        Ok(Response::new(CreateVolumeResponse {
            volume: Some(Volume {
                volume_id,
                capacity_bytes,
                volume_context: HashMap::from([("url".to_string(), url)]),
                ..Default::default()
            }),
        }))
    }

    async fn delete_volume(
        &self,
        request: Request<DeleteVolumeRequest>,
    ) -> Result<Response<DeleteVolumeResponse>, Status> {
        self.check_leadership().await?;

        let request = request.into_inner();
        if request.volume_id.is_empty() {
            return Err(Status::invalid_argument("Missing volume ID"));
        }
        let operator = self.operator.clone();
        run_to_completion("DeleteVolume", async move {
            match operator.delete_volume(&request.volume_id).await {
                // An ID this driver cannot have issued refers to no volume, which counts as deleted.
                Ok(()) | Err(ControllerError::VolumeIdParse) => Ok(()),
                Err(e) => Err(e),
            }
        })
        .await?;

        Ok(Response::new(DeleteVolumeResponse {}))
    }

    async fn controller_publish_volume(
        &self,
        request: Request<ControllerPublishVolumeRequest>,
    ) -> Result<Response<ControllerPublishVolumeResponse>, Status> {
        self.check_leadership().await?;

        let request = request.into_inner();
        if request.volume_id.is_empty() {
            return Err(Status::invalid_argument("Missing volume ID"));
        }
        if request.node_id.is_empty() {
            return Err(Status::invalid_argument("Missing node ID"));
        }
        match &request.volume_capability {
            None => return Err(Status::invalid_argument("Missing volume capability")),
            Some(capability) if !supported_capability(capability) => {
                return Err(Status::invalid_argument(
                    "Only ext4 mount volumes with single-node access are supported",
                ));
            }
            Some(_) => {}
        }
        let operator = self.operator.clone();
        run_to_completion("ControllerPublishVolume", async move {
            operator
                .publish_volume(&request.volume_id, &request.node_id)
                .await
        })
        .await?;

        Ok(Response::new(ControllerPublishVolumeResponse {
            publish_context: Default::default(),
        }))
    }

    async fn controller_unpublish_volume(
        &self,
        request: Request<ControllerUnpublishVolumeRequest>,
    ) -> Result<Response<ControllerUnpublishVolumeResponse>, Status> {
        self.check_leadership().await?;

        let request = request.into_inner();
        let operator = self.operator.clone();
        run_to_completion("ControllerUnpublishVolume", async move {
            operator
                .unpublish_volume(&request.volume_id, &request.node_id)
                .await
        })
        .await?;
        Ok(Response::new(ControllerUnpublishVolumeResponse {}))
    }

    async fn validate_volume_capabilities(
        &self,
        request: Request<ValidateVolumeCapabilitiesRequest>,
    ) -> Result<Response<ValidateVolumeCapabilitiesResponse>, Status> {
        let request = request.into_inner();
        if request.volume_id.is_empty() {
            return Err(Status::invalid_argument("Missing volume ID"));
        }
        if request.volume_capabilities.is_empty() {
            return Err(Status::invalid_argument("Missing volume capabilities"));
        }
        self.operator.volume_size(&request.volume_id).await?;
        if !request.volume_capabilities.iter().all(supported_capability) {
            return Ok(Response::new(ValidateVolumeCapabilitiesResponse {
                confirmed: None,
                message: "Only ext4 mount volumes with SINGLE_NODE_WRITER or \
                          SINGLE_NODE_READER_ONLY access and no mount flags are supported"
                    .to_string(),
            }));
        }
        Ok(Response::new(ValidateVolumeCapabilitiesResponse {
            confirmed: Some(
                crate::proto::csi::v1::validate_volume_capabilities_response::Confirmed {
                    volume_context: request.volume_context,
                    volume_capabilities: request.volume_capabilities,
                    parameters: request.parameters,
                    mutable_parameters: request.mutable_parameters,
                },
            ),
            message: String::new(),
        }))
    }

    async fn list_volumes(
        &self,
        _request: Request<ListVolumesRequest>,
    ) -> Result<Response<ListVolumesResponse>, Status> {
        Err(Status::unimplemented("ListVolumes is not implemented"))
    }

    async fn controller_list_volume_health(
        &self,
        _request: Request<ControllerListVolumeHealthRequest>,
    ) -> Result<Response<ControllerListVolumeHealthResponse>, Status> {
        Err(Status::unimplemented(
            "ControllerListVolumeHealth is not implemented",
        ))
    }

    async fn controller_get_volume_health(
        &self,
        _request: Request<ControllerGetVolumeHealthRequest>,
    ) -> Result<Response<ControllerGetVolumeHealthResponse>, Status> {
        Err(Status::unimplemented(
            "ControllerGetVolumeHealth is not implemented",
        ))
    }

    async fn get_capacity(
        &self,
        _request: Request<GetCapacityRequest>,
    ) -> Result<Response<GetCapacityResponse>, Status> {
        Err(Status::unimplemented("GetCapacity is not implemented"))
    }

    async fn controller_get_capabilities(
        &self,
        _request: Request<ControllerGetCapabilitiesRequest>,
    ) -> Result<Response<ControllerGetCapabilitiesResponse>, Status> {
        use controller_service_capability::rpc::Type;
        Ok(Response::new(ControllerGetCapabilitiesResponse {
            capabilities: [
                Type::CreateDeleteVolume,
                Type::PublishUnpublishVolume,
                Type::ExpandVolume,
            ]
            .into_iter()
            .map(|kind| ControllerServiceCapability {
                r#type: Some(controller_service_capability::Type::Rpc(
                    controller_service_capability::Rpc {
                        r#type: kind as i32,
                    },
                )),
            })
            .collect(),
        }))
    }

    async fn create_snapshot(
        &self,
        _request: Request<CreateSnapshotRequest>,
    ) -> Result<Response<CreateSnapshotResponse>, Status> {
        Err(Status::unimplemented("CreateSnapshot is not implemented"))
    }

    async fn delete_snapshot(
        &self,
        _request: Request<DeleteSnapshotRequest>,
    ) -> Result<Response<DeleteSnapshotResponse>, Status> {
        Err(Status::unimplemented("DeleteSnapshot is not implemented"))
    }

    async fn list_snapshots(
        &self,
        _request: Request<ListSnapshotsRequest>,
    ) -> Result<Response<ListSnapshotsResponse>, Status> {
        Err(Status::unimplemented("ListSnapshots is not implemented"))
    }

    async fn get_snapshot(
        &self,
        _request: Request<GetSnapshotRequest>,
    ) -> Result<Response<GetSnapshotResponse>, Status> {
        Err(Status::unimplemented("GetSnapshot is not implemented"))
    }

    async fn controller_expand_volume(
        &self,
        request: Request<ControllerExpandVolumeRequest>,
    ) -> Result<Response<ControllerExpandVolumeResponse>, Status> {
        self.check_leadership().await?;

        let request = request.into_inner();
        if request.volume_id.is_empty() {
            return Err(Status::invalid_argument("Missing volume ID"));
        }
        let (required, limit) = capacity_range(request.capacity_range.as_ref())?;
        let operator = self.operator.clone();
        let capacity_bytes = run_to_completion("ControllerExpandVolume", async move {
            operator
                .expand_volume(&request.volume_id, required, limit)
                .await
        })
        .await?;
        Ok(Response::new(ControllerExpandVolumeResponse {
            capacity_bytes,
            node_expansion_required: true,
        }))
    }

    async fn controller_get_volume(
        &self,
        _request: Request<ControllerGetVolumeRequest>,
    ) -> Result<Response<ControllerGetVolumeResponse>, Status> {
        Err(Status::unimplemented(
            "ControllerGetVolume is not implemented",
        ))
    }

    async fn controller_modify_volume(
        &self,
        _request: Request<ControllerModifyVolumeRequest>,
    ) -> Result<Response<ControllerModifyVolumeResponse>, Status> {
        Err(Status::unimplemented(
            "ControllerModifyVolume is not implemented",
        ))
    }
}
