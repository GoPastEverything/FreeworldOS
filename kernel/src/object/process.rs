use crate::exec::profile::ExecutionProfile;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProcessIdentity {
    pub object_id: u128,
    pub profile: ExecutionProfile,
}
