use core::fmt::Debug;

use crate::{
    arch,
    debug::events,
};

pub fn run_result<E: Debug>(
    name: &'static str,
    test: impl FnOnce() -> Result<(), E>,
) {
    let name_hash = stable_name_hash(name);

    events::record(
        events::SELFTEST_BEGIN,
        events::SUBSYSTEM_SELFTEST,
        events::LEVEL_INFO,
        [name_hash, 0, 0, 0],
    );
    arch::serial::write_fmt(format_args!(
        "FreeWorldOS: SELFTEST BEGIN name={name}\n"
    ));

    match test() {
        Ok(()) => {
            events::record(
                events::SELFTEST_PASS,
                events::SUBSYSTEM_SELFTEST,
                events::LEVEL_INFO,
                [name_hash, 0, 0, 0],
            );
            arch::serial::write_fmt(format_args!(
                "FreeWorldOS: SELFTEST PASS name={name}\n"
            ));
        }
        Err(error) => {
            events::record(
                events::SELFTEST_FAIL,
                events::SUBSYSTEM_SELFTEST,
                events::LEVEL_ERROR,
                [name_hash, 0, 0, 0],
            );
            arch::serial::write_fmt(format_args!(
                "FreeWorldOS: SELFTEST FAIL name={name} error={error:?}\n"
            ));
            panic!("named self-test failed: {name}");
        }
    }
}

pub fn run_infallible(name: &'static str, test: impl FnOnce()) {
    let name_hash = stable_name_hash(name);

    events::record(
        events::SELFTEST_BEGIN,
        events::SUBSYSTEM_SELFTEST,
        events::LEVEL_INFO,
        [name_hash, 0, 0, 0],
    );
    arch::serial::write_fmt(format_args!(
        "FreeWorldOS: SELFTEST BEGIN name={name}\n"
    ));

    test();

    events::record(
        events::SELFTEST_PASS,
        events::SUBSYSTEM_SELFTEST,
        events::LEVEL_INFO,
        [name_hash, 0, 0, 0],
    );
    arch::serial::write_fmt(format_args!(
        "FreeWorldOS: SELFTEST PASS name={name}\n"
    ));
}

fn stable_name_hash(name: &str) -> u64 {
    // FNV-1a 64-bit. This is only a compact event correlation key; the serial
    // self-test line remains the human-readable source.
    let mut hash = 0xcbf29ce484222325u64;
    for byte in name.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}
