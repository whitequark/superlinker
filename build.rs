use elf::{abi::*, endian::AnyEndian, ElfBytes};
use std::{env, process::Command};

const TARGETS: &[&str] = &["x86_64-unknown-none"];

const DT_RELR: i64 = 36;

fn segment_data(path: &str) -> Vec<u8> {
    let bytes = std::fs::read(path).expect("Reading file");
    let elf: ElfBytes<AnyEndian> = ElfBytes::minimal_parse(&bytes).expect("Parsing file");
    let dynamic = elf
        .dynamic()
        .expect("Parsing dynamic segment")
        .expect("File should have dynamic segment");

    assert!(
        !dynamic
            .iter()
            .any(|d| [DT_REL, DT_RELA, DT_RELR].contains(&d.d_tag)),
        "Shim should have no relocations"
    );
    let segments = elf.segments().expect("Parsing segments");
    let seg = segments.get(0).expect("Get first segment");
    assert!(seg.p_type == PT_LOAD, "First segment should be PT_LOAD");
    let data = elf.segment_data(&seg).expect("Getting segment data");
    data.to_vec()
}

fn main() {
    println!("cargo::rerun-if-changed=shim");

    let out_dir = env::var("OUT_DIR").unwrap();
    let call_rustc = |target| {
        let status = Command::new("rustc")
            .arg(format!("--target={target}"))
            .arg("-Copt-level=s")
            .arg("-Cpanic=abort")
            .arg("-Crelocation-model=pic")
            .arg("-Clink-args=-pie")
            .arg("-Clink-args=-Tshim/link.x")
            .arg(format!("--out-dir={out_dir}/shim/{target}"))
            .arg("--crate-name=shim")
            .arg("shim/main.rs")
            .env("RUSTC_BOOTSTRAP", "1")
            .status()
            .unwrap();
        assert!(status.success());
    };

    for target in TARGETS {
        call_rustc(target);
        let elf_file = format!("{out_dir}/shim/{target}/shim");
        eprintln!("Building {elf_file}");
        let data = segment_data(&elf_file);
        eprintln!("Processing {elf_file}");
        let out_path = format!("{out_dir}/shim/{target}/shim.bin");
        std::fs::write(out_path, data).expect("Writing segment data");
    }
}
