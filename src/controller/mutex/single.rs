use crate::controller::error::ControllerError;
use crate::controller::mutex::ControllerMutex;
use tonic::async_trait;

pub(crate) struct SingleNodeControllerMutex;

#[async_trait]
impl ControllerMutex for SingleNodeControllerMutex {
    async fn is_leader(&self) -> Result<bool, ControllerError> {
        Ok(true)
    }
}
