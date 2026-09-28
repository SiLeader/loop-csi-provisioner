use crate::proto::csi::v1::node_server::Node;
use crate::proto::csi::v1::{
    NodeExpandVolumeRequest, NodeExpandVolumeResponse, NodeGetCapabilitiesRequest,
    NodeGetCapabilitiesResponse, NodeGetInfoRequest, NodeGetInfoResponse,
    NodeGetStorageHealthRequest, NodeGetStorageHealthResponse, NodeGetVolumeHealthRequest,
    NodeGetVolumeHealthResponse, NodeGetVolumeStatsRequest, NodeGetVolumeStatsResponse,
    NodePublishVolumeRequest, NodePublishVolumeResponse, NodeStageVolumeRequest,
    NodeStageVolumeResponse, NodeUnpublishVolumeRequest, NodeUnpublishVolumeResponse,
    NodeUnstageVolumeRequest, NodeUnstageVolumeResponse,
};
use std::path::PathBuf;
use tokio::process::Command;
use tonic::{Request, Response, Status, async_trait};

pub struct LoopCsiNode {
    base_directory: PathBuf,
}

#[async_trait]
impl Node for LoopCsiNode {
    async fn node_stage_volume(
        &self,
        request: Request<NodeStageVolumeRequest>,
    ) -> Result<Response<NodeStageVolumeResponse>, Status> {
        let request = request.into_inner();
        let file_name = format!("{}.img", request.volume_id);
        let res = Command::new("losetup")
            .args([
                "--find",
                "--show",
                &self.base_directory.join(&file_name).to_string_lossy(),
            ])
            .output()
            .await?;
        if !res.status.success() {
            return Err(Status::internal(format!(
                "losetup failed: {}",
                String::from_utf8_lossy(&res.stderr)
            )));
        }
        let loop_device = String::from_utf8_lossy(&res.stdout).trim().to_string();

        let res = Command::new("blkid").args([&loop_device]).output().await?;
        if !res.status.success() {
            return Err(Status::internal(format!(
                "blkid failed: {}",
                String::from_utf8_lossy(&res.stderr)
            )));
        }
        if res.stdout.is_empty() {
            let res = Command::new("mkfs.ext4")
                .args(["-F", &loop_device])
                .output()
                .await?;
            if !res.status.success() {
                return Err(Status::internal(format!(
                    "mkfs.ext4 failed: {}",
                    String::from_utf8_lossy(&res.stderr)
                )));
            }
        }

        let res = Command::new("mount")
            .args([&loop_device, &request.staging_target_path])
            .output()
            .await?;
        if !res.status.success() {
            return Err(Status::internal(format!(
                "mount failed: {}",
                String::from_utf8_lossy(&res.stderr)
            )));
        }

        Ok(Response::new(NodeStageVolumeResponse {}))
    }

    async fn node_unstage_volume(
        &self,
        request: Request<NodeUnstageVolumeRequest>,
    ) -> Result<Response<NodeUnstageVolumeResponse>, Status> {
        todo!()
    }

    async fn node_publish_volume(
        &self,
        request: Request<NodePublishVolumeRequest>,
    ) -> Result<Response<NodePublishVolumeResponse>, Status> {
        let request = request.into_inner();

        let res = Command::new("mount")
            .args([&request.staging_target_path, &request.target_path])
            .output()
            .await?;
        if !res.status.success() {
            return Err(Status::internal(format!(
                "mount failed: {}",
                String::from_utf8_lossy(&res.stderr)
            )));
        }
        Ok(Response::new(NodePublishVolumeResponse {}))
    }

    async fn node_unpublish_volume(
        &self,
        request: Request<NodeUnpublishVolumeRequest>,
    ) -> Result<Response<NodeUnpublishVolumeResponse>, Status> {
        todo!()
    }

    async fn node_get_volume_stats(
        &self,
        _request: Request<NodeGetVolumeStatsRequest>,
    ) -> Result<Response<NodeGetVolumeStatsResponse>, Status> {
        Err(Status::unavailable("NodeGetVolumeStats is not implemented"))
    }

    async fn node_get_volume_health(
        &self,
        _request: Request<NodeGetVolumeHealthRequest>,
    ) -> Result<Response<NodeGetVolumeHealthResponse>, Status> {
        Err(Status::unavailable(
            "NodeGetVolumeHealth is not implemented",
        ))
    }

    async fn node_get_storage_health(
        &self,
        _request: Request<NodeGetStorageHealthRequest>,
    ) -> Result<Response<NodeGetStorageHealthResponse>, Status> {
        Err(Status::unavailable(
            "NodeGetStorageHealth is not implemented",
        ))
    }

    async fn node_expand_volume(
        &self,
        _request: Request<NodeExpandVolumeRequest>,
    ) -> Result<Response<NodeExpandVolumeResponse>, Status> {
        Err(Status::unavailable("NodeExpandVolume is not implemented"))
    }

    async fn node_get_capabilities(
        &self,
        request: Request<NodeGetCapabilitiesRequest>,
    ) -> Result<Response<NodeGetCapabilitiesResponse>, Status> {
        todo!()
    }

    async fn node_get_info(
        &self,
        request: Request<NodeGetInfoRequest>,
    ) -> Result<Response<NodeGetInfoResponse>, Status> {
        todo!()
    }
}
