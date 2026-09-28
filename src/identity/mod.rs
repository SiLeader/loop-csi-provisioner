use crate::proto::csi::v1::identity_server::Identity;
use crate::proto::csi::v1::{plugin_capability, GetPluginCapabilitiesRequest, GetPluginCapabilitiesResponse, GetPluginInfoRequest, GetPluginInfoResponse, PluginCapability, ProbeRequest, ProbeResponse};
use tonic::{Request, Response, Status, async_trait};
use crate::VERSION;

pub struct NfsLoopCsiIdentity {}

#[async_trait]
impl Identity for NfsLoopCsiIdentity {
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
        Ok(Response::new(GetPluginCapabilitiesResponse {
            capabilities: vec![
                PluginCapability {
                    r#type: Some(plugin_capability::Type::Service(
                        plugin_capability::Service {
                            r#type: plugin_capability::service::Type::ControllerService as i32,
                        },
                    )),
                },
                PluginCapability{
                    r#type: Some(plugin_capability::Type::VolumeExpansion(
                        plugin_capability::VolumeExpansion{
                            r#type: plugin_capability::volume_expansion::Type::Online as i32,
                        }
                    )),
                }
            ],
        }))
    }

    async fn probe(
        &self,
        request: Request<ProbeRequest>,
    ) -> Result<Response<ProbeResponse>, Status> {
        todo!()
    }
}
