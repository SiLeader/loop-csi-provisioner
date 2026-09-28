mod error;
pub(crate) mod operator;

use crate::controller::operator::ControllerOperator;
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
    ValidateVolumeCapabilitiesResponse, Volume, VolumeCapability, controller_service_capability,
};
use crate::volume_id::valid_volume_name;
use std::collections::HashMap;
use tonic::{Request, Response, Status, async_trait};

pub(crate) struct LoopCsiController {
    operator: ControllerOperator,
}

impl LoopCsiController {
    pub fn new(operator: ControllerOperator) -> Self {
        Self { operator }
    }
}

fn supported_capability(capability: &VolumeCapability) -> bool {
    use crate::proto::csi::v1::volume_capability::{AccessType, access_mode::Mode};
    let mount_ok = matches!(&capability.access_type, Some(AccessType::Mount(mount))
        if (mount.fs_type.is_empty() || mount.fs_type == "ext4") && mount.mount_flags.is_empty() && mount.volume_mount_group.is_empty());
    let mode_ok = capability.access_mode.as_ref().is_some_and(|mode| {
        matches!(
            Mode::try_from(mode.mode),
            Ok(Mode::SingleNodeWriter | Mode::SingleNodeReaderOnly)
        )
    });
    mount_ok && mode_ok
}

#[async_trait]
impl Controller for LoopCsiController {
    async fn create_volume(
        &self,
        request: Request<CreateVolumeRequest>,
    ) -> Result<Response<CreateVolumeResponse>, Status> {
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
        if let Some(range) = &request.capacity_range {
            if range.required_bytes < 0
                || range.limit_bytes < 0
                || (range.limit_bytes > 0 && range.required_bytes > range.limit_bytes)
            {
                return Err(Status::invalid_argument("Invalid capacity range"));
            }
            if range.limit_bytes > 0
                && range.required_bytes == 0
                && self.operator.default_size() > range.limit_bytes
            {
                return Err(Status::out_of_range(
                    "Default volume size exceeds capacity limit",
                ));
            }
        }
        let volume_id = format!("{}:{}", url, request.name);
        let capacity_bytes = self
            .operator
            .create_volume(
                &volume_id,
                request.capacity_range.as_ref().map(|c| c.required_bytes),
                &url,
            )
            .await?;
        if request
            .capacity_range
            .as_ref()
            .is_some_and(|range| range.limit_bytes > 0 && capacity_bytes > range.limit_bytes)
        {
            return Err(Status::out_of_range("Volume exceeds capacity limit"));
        }

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
        let request = request.into_inner();
        self.operator.delete_volume(&request.volume_id).await?;

        Ok(Response::new(DeleteVolumeResponse {}))
    }

    async fn controller_publish_volume(
        &self,
        request: Request<ControllerPublishVolumeRequest>,
    ) -> Result<Response<ControllerPublishVolumeResponse>, Status> {
        let request = request.into_inner();
        if request.node_id.is_empty() {
            return Err(Status::invalid_argument("Missing node ID"));
        }
        self.operator
            .publish_volume(&request.volume_id, &request.node_id)
            .await?;

        Ok(Response::new(ControllerPublishVolumeResponse {
            publish_context: Default::default(),
        }))
    }

    async fn controller_unpublish_volume(
        &self,
        request: Request<ControllerUnpublishVolumeRequest>,
    ) -> Result<Response<ControllerUnpublishVolumeResponse>, Status> {
        let request = request.into_inner();
        self.operator
            .unpublish_volume(&request.volume_id, &request.node_id)
            .await?;
        Ok(Response::new(ControllerUnpublishVolumeResponse {}))
    }

    async fn validate_volume_capabilities(
        &self,
        request: Request<ValidateVolumeCapabilitiesRequest>,
    ) -> Result<Response<ValidateVolumeCapabilitiesResponse>, Status> {
        let request = request.into_inner();
        self.operator.volume_size(&request.volume_id).await?;
        if request.volume_capabilities.is_empty() {
            return Err(Status::invalid_argument("Missing volume capabilities"));
        }
        let confirmed = request
            .volume_capabilities
            .iter()
            .all(supported_capability)
            .then_some(
                crate::proto::csi::v1::validate_volume_capabilities_response::Confirmed {
                    volume_context: request.volume_context,
                    volume_capabilities: request.volume_capabilities,
                    parameters: request.parameters,
                    mutable_parameters: request.mutable_parameters,
                },
            );
        Ok(Response::new(ValidateVolumeCapabilitiesResponse {
            confirmed,
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
        let request = request.into_inner();
        let size = request.capacity_range.as_ref().map(|c| c.required_bytes);
        if request.capacity_range.as_ref().is_some_and(|range| {
            range.required_bytes < 0
                || range.limit_bytes < 0
                || (range.limit_bytes > 0 && range.required_bytes > range.limit_bytes)
        }) {
            return Err(Status::invalid_argument("Invalid capacity range"));
        }
        if request.capacity_range.as_ref().is_some_and(|range| {
            range.limit_bytes > 0
                && range.required_bytes == 0
                && self.operator.default_size() > range.limit_bytes
        }) {
            return Err(Status::out_of_range(
                "Default volume size exceeds capacity limit",
            ));
        }
        let capacity_bytes = self
            .operator
            .expand_volume(&request.volume_id, size)
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
