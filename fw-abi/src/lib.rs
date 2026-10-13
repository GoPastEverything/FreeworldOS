#![no_std]
//! FreeWorld native status registry, v0.1. Numeric assignments are permanent.
//! Each negative code is returned sign-extended in RAX (x86-64).
pub type FwStatus = i32;
pub const FW_OK: FwStatus = 0;

// General subsystem: [-4096, -4351].
pub const FW_ERR_NO_SUCH_CALL: FwStatus = -4098;
pub const FW_ERR_UNSUPPORTED: FwStatus = -4099;
// Memory subsystem: [-8192, -8447].
pub const FW_ERR_USER_ADDRESS: FwStatus = -8193;

#[inline]
pub const fn encode_error(status: FwStatus) -> u64 {
    assert!(status < 0);
    (status as i64) as u64
}
#[inline]
pub const fn decode_result(raw: u64) -> i64 {
    raw as i64
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn code_ranges_and_sign_extension() {
        assert_eq!(FW_OK, 0);
        assert_eq!(encode_error(FW_ERR_USER_ADDRESS) as i64, -8193);
        assert_eq!(encode_error(FW_ERR_NO_SUCH_CALL) as i64, -4098);
        assert_ne!(FW_ERR_USER_ADDRESS, -14);
        assert_ne!(FW_ERR_NO_SUCH_CALL, -38);
        assert_eq!(decode_result(20), 20);
    }
}
