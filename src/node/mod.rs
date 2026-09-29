mod error;
pub(crate) mod operator;

use crate::capability::supported_capability;
use crate::node::operator::NodeOperator;
use crate::proto::csi::v1::node_server::Node;
use crate::proto::csi::v1::{
    NodeExpandVolumeRequest, NodeExpandVolumeResponse, NodeGetCapabilitiesRequest,
    NodeGetCapabilitiesResponse, NodeGetInfoRequest, NodeGetInfoResponse,
    NodeGetStorageHealthRequest, NodeGetStorageHealthResponse, NodeGetVolumeHealthRequest,
    NodeGetVolumeHealthResponse, NodeGetVolumeStatsRequest, NodeGetVolumeStatsResponse,
    NodePublishVolumeRequest, NodePublishVolumeResponse, NodeServiceCapability,
    NodeStageVolumeRequest, NodeStageVolumeResponse, NodeUnpublishVolumeRequest,
    NodeUnpublishVolumeResponse, NodeUnstageVolumeRequest, NodeUnstageVolumeResponse,
    VolumeCapability, node_service_capability,
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

fn require(value: &str, name: &str) -> Result<(), Status> {
    if value.is_empty() {
        Err(Status::invalid_argument(format!("Missing {name}")))
    } else {
        Ok(())
    }
}

fn require_capability(capability: Option<&VolumeCapability>) -> Result<(), Status> {
    match capability {
        None => Err(Status::invalid_argument("Missing volume capability")),
        Some(capability) if !supported_capability(capability) => Err(Status::invalid_argument(
            "Only ext4 mount volumes with single-node access are supported",
        )),
        Some(_) => Ok(()),
    }
}

#[async_trait]
impl Node for LoopCsiNode {
    async fn node_stage_volume(
        &self,
        request: Request<NodeStageVolumeRequest>,
    ) -> Result<Response<NodeStageVolumeResponse>, Status> {
        let request = request.into_inner();
        require(&request.volume_id, "volume ID")?;
        require(&request.staging_target_path, "staging target path")?;
        require_capability(request.volume_capability.as_ref())?;

        self.operator
            .stage_volume(&request.volume_id, &request.staging_target_path)
            .await?;

        Ok(Response::new(NodeStageVolumeResponse {}))
    }

    async fn node_unstage_volume(
        &self,
        request: Request<NodeUnstageVolumeRequest>,
    ) -> Result<Response<NodeUnstageVolumeResponse>, Status> {
        let request = request.into_inner();
        require(&request.volume_id, "volume ID")?;
        require(&request.staging_target_path, "staging target path")?;
        self.operator
            .unstage_volume(&request.volume_id, &request.staging_target_path)
            .await?;
        Ok(Response::new(NodeUnstageVolumeResponse {}))
    }

    async fn node_publish_volume(
        &self,
        request: Request<NodePublishVolumeRequest>,
    ) -> Result<Response<NodePublishVolumeResponse>, Status> {
        let request = request.into_inner();
        require(&request.volume_id, "volume ID")?;
        require(&request.staging_target_path, "staging target path")?;
        require(&request.target_path, "target path")?;
        require_capability(request.volume_capability.as_ref())?;

        self.operator
            .publish_volume(
                &request.staging_target_path,
                &request.target_path,
                request.readonly,
            )
            .await?;
        Ok(Response::new(NodePublishVolumeResponse {}))
    }

    async fn node_unpublish_volume(
        &self,
        request: Request<NodeUnpublishVolumeRequest>,
    ) -> Result<Response<NodeUnpublishVolumeResponse>, Status> {
        let request = request.into_inner();
        require(&request.volume_id, "volume ID")?;
        require(&request.target_path, "target path")?;
        self.operator.unpublish_volume(&request.target_path).await?;
        Ok(Response::new(NodeUnpublishVolumeResponse {}))
    }

    async fn node_get_volume_stats(
        &self,
        _request: Request<NodeGetVolumeStatsRequest>,
    ) -> Result<Response<NodeGetVolumeStatsResponse>, Status> {
        Err(Status::unimplemented(
            "NodeGetVolumeStats is not implemented",
        ))
    }

    async fn node_get_volume_health(
        &self,
        _request: Request<NodeGetVolumeHealthRequest>,
    ) -> Result<Response<NodeGetVolumeHealthResponse>, Status> {
        Err(Status::unimplemented(
            "NodeGetVolumeHealth is not implemented",
        ))
    }

    async fn node_get_storage_health(
        &self,
        _request: Request<NodeGetStorageHealthRequest>,
    ) -> Result<Response<NodeGetStorageHealthResponse>, Status> {
        Err(Status::unimplemented(
            "NodeGetStorageHealth is not implemented",
        ))
    }

    async fn node_expand_volume(
        &self,
        request: Request<NodeExpandVolumeRequest>,
    ) -> Result<Response<NodeExpandVolumeResponse>, Status> {
        let request = request.into_inner();
        require(&request.volume_id, "volume ID")?;

        let capacity_bytes = self.operator.expand_volume(&request.volume_id).await?;

        Ok(Response::new(NodeExpandVolumeResponse { capacity_bytes }))
    }

    async fn node_get_capabilities(
        &self,
        _request: Request<NodeGetCapabilitiesRequest>,
    ) -> Result<Response<NodeGetCapabilitiesResponse>, Status> {
        use node_service_capability::rpc::Type;
        Ok(Response::new(NodeGetCapabilitiesResponse {
            capabilities: [Type::StageUnstageVolume, Type::ExpandVolume]
                .into_iter()
                .map(|kind| NodeServiceCapability {
                    r#type: Some(node_service_capability::Type::Rpc(
                        node_service_capability::Rpc {
                            r#type: kind as i32,
                        },
                    )),
                })
                .collect(),
        }))
    }

    async fn node_get_info(
        &self,
        _request: Request<NodeGetInfoRequest>,
    ) -> Result<Response<NodeGetInfoResponse>, Status> {
        let node_id = std::env::var("NODE_ID")
            .ok()
            .filter(|id| !id.is_empty())
            .or_else(|| {
                std::fs::read_to_string("/etc/hostname")
                    .ok()
                    .map(|s| s.trim().to_string())
            })
            .filter(|id| !id.is_empty())
            .ok_or_else(|| Status::internal("Unable to determine node ID"))?;
        Ok(Response::new(NodeGetInfoResponse {
            node_id,
            max_volumes_per_node: 0,
            accessible_topology: None,
        }))
    }
}
