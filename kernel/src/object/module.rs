use crate::exec::profile::{Abi, ImageFormat};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ModuleIdentity {
    pub object_id: u128,
    pub format: ImageFormat,
    pub abi: Abi,
}
