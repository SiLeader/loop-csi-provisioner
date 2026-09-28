mod error;
pub(crate) mod operator;

use crate::node::operator::NodeOperator;
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
use tonic::{Request, Response, Status, async_trait};

pub(crate) struct LoopCsiNode {
    operator: NodeOperator,
}

impl LoopCsiNode {
    pub fn new(operator: NodeOperator) -> Self {
        Self { operator }
    }
}

#[async_trait]
impl Node for LoopCsiNode {
    async fn node_stage_volume(
        &self,
        request: Request<NodeStageVolumeRequest>,
    ) -> Result<Response<NodeStageVolumeResponse>, Status> {
        let request = request.into_inner();

        self.operator
            .stage_volume(&request.volume_id, &request.staging_target_path)
            .await?;

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

        self.operator
            .publish_volume(&request.staging_target_path, &request.target_path)
            .await?;
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
        request: Request<NodeExpandVolumeRequest>,
    ) -> Result<Response<NodeExpandVolumeResponse>, Status> {
        let request = request.into_inner();

        let capacity_bytes = self.operator.expand_volume(&request.volume_id).await?;

        Ok(Response::new(NodeExpandVolumeResponse { capacity_bytes }))
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
