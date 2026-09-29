mod error;
pub(crate) mod operator;

use crate::capability::{is_read_only, supported_capability};
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
use crate::task::run_to_completion;
use std::sync::Arc;
use tonic::{Request, Response, Status, async_trait};

pub(crate) struct LoopCsiNode {
    operator: Arc<NodeOperator>,
    node_id: String,
}

impl LoopCsiNode {
    pub fn new(operator: NodeOperator, node_id: String) -> Self {
        Self {
            operator: Arc::new(operator),
            node_id,
        }
    }
}

fn require(value: &str, name: &str) -> Result<(), Status> {
    if value.is_empty() {
        Err(Status::invalid_argument(format!("Missing {name}")))
    } else {
        Ok(())
    }
}

/// Validates the capability and returns whether it only allows reading.
fn require_capability(capability: Option<&VolumeCapability>) -> Result<bool, Status> {
    match capability {
        None => Err(Status::invalid_argument("Missing volume capability")),
        Some(capability) if !supported_capability(capability) => Err(Status::invalid_argument(
            "Only ext4 mount volumes with single-node access are supported",
        )),
        Some(capability) => Ok(is_read_only(capability)),
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
        let read_only = require_capability(request.volume_capability.as_ref())?;

        let operator = self.operator.clone();
        run_to_completion("NodeStageVolume", async move {
            operator
                .stage_volume(&request.volume_id, &request.staging_target_path, read_only)
                .await
        })
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
        let operator = self.operator.clone();
        run_to_completion("NodeUnstageVolume", async move {
            operator
                .unstage_volume(&request.volume_id, &request.staging_target_path)
                .await
        })
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
        let read_only = require_capability(request.volume_capability.as_ref())? || request.readonly;

        let operator = self.operator.clone();
        run_to_completion("NodePublishVolume", async move {
            operator
                .publish_volume(
                    &request.volume_id,
                    &request.staging_target_path,
                    &request.target_path,
                    read_only,
                )
                .await
        })
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
        let operator = self.operator.clone();
        run_to_completion("NodeUnpublishVolume", async move {
            operator.unpublish_volume(&request.target_path).await
        })
        .await?;
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
        let required = request
            .capacity_range
            .as_ref()
            .map_or(0, |range| range.required_bytes);

        let operator = self.operator.clone();
        let capacity_bytes = run_to_completion("NodeExpandVolume", async move {
            operator.expand_volume(&request.volume_id, required).await
        })
        .await?;

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
        Ok(Response::new(NodeGetInfoResponse {
            node_id: self.node_id.clone(),
            max_volumes_per_node: 0,
            accessible_topology: None,
        }))
    }
}
