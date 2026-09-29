use crate::VERSION;
use crate::proto::csi::v1::identity_server::Identity;
use crate::proto::csi::v1::{
    GetPluginCapabilitiesRequest, GetPluginCapabilitiesResponse, GetPluginInfoRequest,
    GetPluginInfoResponse, PluginCapability, ProbeRequest, ProbeResponse, plugin_capability,
};
use tonic::{Request, Response, Status, async_trait};

pub struct LoopCsiIdentity {
    /// Whether this process also serves the Controller API.
    pub controller_service: bool,
}

#[async_trait]
impl Identity for LoopCsiIdentity {
    async fn get_plugin_info(
        &self,
        _request: Request<GetPluginInfoRequest>,
    ) -> Result<Response<GetPluginInfoResponse>, Status> {
        Ok(Response::new(GetPluginInfoResponse {
            name: "loop-csi-provisioner".to_string(),
            vendor_version: VERSION.to_string(),
            manifest: Default::default(),
        }))
    }

    async fn get_plugin_capabilities(
        &self,
        _request: Request<GetPluginCapabilitiesRequest>,
    ) -> Result<Response<GetPluginCapabilitiesResponse>, Status> {
        let mut capabilities = Vec::new();
        if self.controller_service {
            capabilities.push(PluginCapability {
                r#type: Some(plugin_capability::Type::Service(
                    plugin_capability::Service {
                        r#type: plugin_capability::service::Type::ControllerService as i32,
                    },
                )),
            });
            capabilities.push(PluginCapability {
                r#type: Some(plugin_capability::Type::VolumeExpansion(
                    plugin_capability::VolumeExpansion {
                        r#type: plugin_capability::volume_expansion::Type::Online as i32,
                    },
                )),
            });
        }
        Ok(Response::new(GetPluginCapabilitiesResponse {
            capabilities,
        }))
    }

    async fn probe(
        &self,
        _request: Request<ProbeRequest>,
    ) -> Result<Response<ProbeResponse>, Status> {
        Ok(Response::new(ProbeResponse { ready: Some(true) }))
    }
}
