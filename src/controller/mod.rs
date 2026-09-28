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
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::path::PathBuf;
use tonic::{Request, Response, Status, async_trait};

pub struct LoopCsiController {
    default_size: i64,
    storage_base_directory: PathBuf,
    metadata_base_directory: PathBuf,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Metadata {
    volume_id: String,
    attached_node: String,
}

#[async_trait]
impl Controller for LoopCsiController {
    async fn create_volume(
        &self,
        request: Request<CreateVolumeRequest>,
    ) -> Result<Response<CreateVolumeResponse>, Status> {
        let request = request.into_inner();
        let size = request
            .capacity_range
            .as_ref()
            .map(|c| c.required_bytes)
            .unwrap_or(self.default_size);

        let file_name = format!("{}.img", request.name);
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .open(&self.storage_base_directory.join(&file_name))?;
        file.set_len(size as u64)?;

        Ok(Response::new(CreateVolumeResponse {
            volume: Some(Volume {
                volume_id: file_name,
                capacity_bytes: size,
                ..Default::default()
            }),
        }))
    }

    async fn delete_volume(
        &self,
        request: Request<DeleteVolumeRequest>,
    ) -> Result<Response<DeleteVolumeResponse>, Status> {
        todo!()
    }

    async fn controller_publish_volume(
        &self,
        request: Request<ControllerPublishVolumeRequest>,
    ) -> Result<Response<ControllerPublishVolumeResponse>, Status> {
        let request = request.into_inner();
        let metadata_file_name = format!("{}.json", request.volume_id);
        let metadata_file_path = self.metadata_base_directory.join(&metadata_file_name);
        let metadata: Option<Metadata> = {
            let file = match File::open(&metadata_file_path) {
                Ok(f) => Some(f),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(e) => {
                    return Err(Status::not_found(format!(
                        "Metadata file {} not found: {}",
                        metadata_file_path.display(),
                        e
                    )));
                }
            };
            match file {
                Some(f) => Some(serde_json::from_reader(f).map_err(|e| {
                    Status::internal(format!(
                        "Failed to parse metadata file {}: {}",
                        metadata_file_path.display(),
                        e
                    ))
                })?),
                None => None,
            }
        };
        if let Some(metadata) = metadata {
            if metadata.attached_node != request.node_id {
                return Err(Status::failed_precondition(format!(
                    "Volume {} is attached to node {}, cannot attach to node {}",
                    request.volume_id, metadata.attached_node, request.node_id
                )));
            }
        } else {
            let metadata = Metadata {
                volume_id: request.volume_id.clone(),
                attached_node: request.node_id.clone(),
            };
            let metadata_file = File::create(&metadata_file_path).map_err(|e| {
                Status::internal(format!(
                    "Failed to create metadata file {}: {}",
                    metadata_file_path.display(),
                    e
                ))
            })?;
            serde_json::to_writer(metadata_file, &metadata).map_err(|e| {
                Status::internal(format!(
                    "Failed to write metadata file {}: {}",
                    metadata_file_path.display(),
                    e
                ))
            })?;
        }

        Ok(Response::new(ControllerPublishVolumeResponse {
            publish_context: Default::default(),
        }))
    }

    async fn controller_unpublish_volume(
        &self,
        request: Request<ControllerUnpublishVolumeRequest>,
    ) -> Result<Response<ControllerUnpublishVolumeResponse>, Status> {
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
        _request: Request<ControllerExpandVolumeRequest>,
    ) -> Result<Response<ControllerExpandVolumeResponse>, Status> {
        Err(Status::unavailable(
            "ControllerExpandVolume is not implemented",
        ))
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
