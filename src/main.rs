use ovmf_prebuilt::{Arch, FileType, Prebuilt, Source};
use std::{env, process::{exit, Command}};

fn main() {
    let mode = env::args().nth(1).unwrap_or_else(|| "uefi".into());
    let bios = env!("FREEWORLD_BIOS_IMAGE");
    let uefi = env!("FREEWORLD_UEFI_IMAGE");

    let mut cmd = Command::new("qemu-system-x86_64");
    cmd.args(["-serial", "mon:stdio", "-display", "none", "-no-reboot", "-no-shutdown", "-m", "256M"]);

    match mode.as_str() {
        "bios" => {
            cmd.args(["-drive", &format!("format=raw,file={bios}")]);
        }
        "uefi" => {
            let prebuilt = Prebuilt::fetch(Source::LATEST, "target/ovmf")
                .expect("failed to fetch OVMF");
            let code = prebuilt.get_file(Arch::X64, FileType::Code);
            let vars = prebuilt.get_file(Arch::X64, FileType::Vars);

            cmd.arg("-drive")
                .arg(format!("format=raw,file={uefi}"));
            cmd.arg("-drive").arg(format!(
                "if=pflash,format=raw,unit=0,file={},readonly=on",
                code.display()
            ));
            cmd.arg("-drive").arg(format!(
                "if=pflash,format=raw,unit=1,file={},snapshot=on",
                vars.display()
            ));
        }
        _ => {
            eprintln!("usage: cargo run -- [uefi|bios]");
            exit(2);
        }
    }

    let status = cmd.status().expect("failed to launch qemu-system-x86_64");
    exit(status.code().unwrap_or(1));
}
