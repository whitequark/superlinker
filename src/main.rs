use elf::endian::AnyEndian;

mod repr;
mod parse;
mod emit;

fn make_executable<P: AsRef<std::path::Path>>(path: P) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let mut perms = std::fs::metadata(&path)?.permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&path, perms)?;
    Ok(())
}

fn main() {
    let output_filename = std::env::args().nth(1).expect("Usage: $0 <output.elf> <input.elf>");
    let input_filename = std::env::args().nth(2).expect("Usage: $0 <output.elf> <input.elf>");

    let input_data = std::fs::read(&input_filename).expect("Could not read input file");
    // let file = ElfBytes::<NativeEndian>::minimal_parse(&input_data).expect("Could not parse ELF");
    let mut image = parse::parse_elf::<AnyEndian>(&input_data[..]).expect("Could not parse input file");

    image.rebase(image.alignment * 5);

    let new_file_data = emit::emit_elf(&image).expect("Could not emit output file");
    std::fs::write(&output_filename, new_file_data).expect("Could not write output file");
    make_executable(&output_filename).expect("Could not make output file executable");
}
