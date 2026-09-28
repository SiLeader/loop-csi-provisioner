mod error;
mod operator;

use crate::controller::operator::ControllerOperator;
use crate::mount::MountManager;
use crate::proto::csi::v1::controller_server::Controller;
use crate::proto::csi::v1::{
    ControllerExpandVolumeRequest, ControllerExpandVolumeResponse,
    ControllerGetCapabilitiesRequest, ControllerGetCapabilitiesResponse,
    ControllerGetVolumeHealthRequest, ControllerGetVolumeHealthResponse,
    ControllerGetVolumeRequest, ControllerGetVolumeResponse, ControllerListVolumeHealthRequest,
    ControllerListVolumeHealthResponse, ControllerModifyVolumeRequest,
    ControllerModifyVolumeResponse, ControllerPublishVolumeRequest,
    ControllerPublishVolumeResponse, ControllerUnpublishVolumeRequest,
    ControllerUnpublishVolumeResponse, CreateSnapshotRequest, CreateSnapshotResponse,
    CreateVolumeRequest, CreateVolumeResponse, DeleteSnapshotRequest, DeleteSnapshotResponse,
    DeleteVolumeRequest, DeleteVolumeResponse, GetCapacityRequest, GetCapacityResponse,
    GetSnapshotRequest, GetSnapshotResponse, ListSnapshotsRequest, ListSnapshotsResponse,
    ListVolumesRequest, ListVolumesResponse, ValidateVolumeCapabilitiesRequest,
    ValidateVolumeCapabilitiesResponse, Volume,
};
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
        let capacity_bytes = self
            .operator
            .create_volume(
                &request.name,
                request.capacity_range.as_ref().map(|c| c.required_bytes),
                &url,
            )
            .await?;

        Ok(Response::new(CreateVolumeResponse {
            volume: Some(Volume {
                volume_id: format!("{}:{}", url, request.name),
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
        request.volume_context;
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
        todo!()
    }

    async fn validate_volume_capabilities(
        &self,
        request: Request<ValidateVolumeCapabilitiesRequest>,
    ) -> Result<Response<ValidateVolumeCapabilitiesResponse>, Status> {
        todo!()
    }

    async fn list_volumes(
        &self,
        _request: Request<ListVolumesRequest>,
    ) -> Result<Response<ListVolumesResponse>, Status> {
        Err(Status::unavailable("ListVolumes is not implemented"))
    }

    async fn controller_list_volume_health(
        &self,
        _request: Request<ControllerListVolumeHealthRequest>,
    ) -> Result<Response<ControllerListVolumeHealthResponse>, Status> {
        Err(Status::unavailable(
            "ControllerListVolumeHealth is not implemented",
        ))
    }

    async fn controller_get_volume_health(
        &self,
        _request: Request<ControllerGetVolumeHealthRequest>,
    ) -> Result<Response<ControllerGetVolumeHealthResponse>, Status> {
        Err(Status::unavailable(
            "ControllerGetVolumeHealth is not implemented",
        ))
    }

    async fn get_capacity(
        &self,
        _request: Request<GetCapacityRequest>,
    ) -> Result<Response<GetCapacityResponse>, Status> {
        Err(Status::unavailable("GetCapacity is not implemented"))
    }

    async fn controller_get_capabilities(
        &self,
        request: Request<ControllerGetCapabilitiesRequest>,
    ) -> Result<Response<ControllerGetCapabilitiesResponse>, Status> {
        todo!()
    }

    async fn create_snapshot(
        &self,
        _request: Request<CreateSnapshotRequest>,
    ) -> Result<Response<CreateSnapshotResponse>, Status> {
        Err(Status::unavailable("CreateSnapshot is not implemented"))
    }

    async fn delete_snapshot(
        &self,
        _request: Request<DeleteSnapshotRequest>,
    ) -> Result<Response<DeleteSnapshotResponse>, Status> {
        Err(Status::unavailable("DeleteSnapshot is not implemented"))
    }

    async fn list_snapshots(
        &self,
        _request: Request<ListSnapshotsRequest>,
    ) -> Result<Response<ListSnapshotsResponse>, Status> {
        Err(Status::unavailable("ListSnapshots is not implemented"))
    }

    async fn get_snapshot(
        &self,
        _request: Request<GetSnapshotRequest>,
    ) -> Result<Response<GetSnapshotResponse>, Status> {
        Err(Status::unavailable("GetSnapshot is not implemented"))
    }

    async fn controller_expand_volume(
        &self,
        request: Request<ControllerExpandVolumeRequest>,
    ) -> Result<Response<ControllerExpandVolumeResponse>, Status> {
        let request = request.into_inner();
        let size = request.capacity_range.as_ref().map(|c| c.required_bytes);
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
        Err(Status::unavailable(
            "ControllerGetVolume is not implemented",
        ))
    }

    async fn controller_modify_volume(
        &self,
        _request: Request<ControllerModifyVolumeRequest>,
    ) -> Result<Response<ControllerModifyVolumeResponse>, Status> {
        Err(Status::unavailable(
            "ControllerModifyVolume is not implemented",
        ))
    }
}
