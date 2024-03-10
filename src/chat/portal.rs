use ed25519_dalek::VerifyingKey;
use rexa::{
    captp::{object::RemoteBootstrap, GenericResolver},
    impl_object,
};

pub struct Portal {}

#[impl_object]
impl Portal {
    #[deliver(always_ok)]
    async fn list_peers(&self) -> Vec<VerifyingKey> {
        todo!()
    }
}
