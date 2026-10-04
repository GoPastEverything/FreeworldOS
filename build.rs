use std::{env, path::PathBuf};

fn main() {
    let kernel = PathBuf::from(
        env::var_os("CARGO_BIN_FILE_KERNEL_kernel")
            .expect("kernel artifact path missing"),
    );
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR missing"));

    let bios = out_dir.join("freeworldos-bios.img");
    let uefi = out_dir.join("freeworldos-uefi.img");

    bootloader::BiosBoot::new(&kernel)
        .create_disk_image(&bios)
        .expect("failed to create BIOS image");

    bootloader::UefiBoot::new(&kernel)
        .create_disk_image(&uefi)
        .expect("failed to create UEFI image");

    println!("cargo:rustc-env=FREEWORLD_BIOS_IMAGE={}", bios.display());
    println!("cargo:rustc-env=FREEWORLD_UEFI_IMAGE={}", uefi.display());
}
