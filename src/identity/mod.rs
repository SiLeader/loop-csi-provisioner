use crate::proto::csi::v1::identity_server::Identity;
use crate::proto::csi::v1::{
    GetPluginCapabilitiesRequest, GetPluginCapabilitiesResponse, GetPluginInfoRequest,
    GetPluginInfoResponse, ProbeRequest, ProbeResponse,
};
use tonic::{Request, Response, Status, async_trait};

pub struct NfsLoopCsiIdentity {}

#[async_trait]
impl Identity for NfsLoopCsiIdentity {
    async fn get_plugin_info(
        &self,
        request: Request<GetPluginInfoRequest>,
    ) -> Result<Response<GetPluginInfoResponse>, Status> {
        todo!()
    }

    async fn get_plugin_capabilities(
        &self,
        request: Request<GetPluginCapabilitiesRequest>,
    ) -> Result<Response<GetPluginCapabilitiesResponse>, Status> {
        todo!()
    }

    async fn probe(
        &self,
        request: Request<ProbeRequest>,
    ) -> Result<Response<ProbeResponse>, Status> {
        todo!()
    }
}
