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
    let output_filename = std::path::PathBuf::from(std::env::args().nth(1).expect("Usage: $0 <output.elf> <input.elf>"));
    let input_filename = std::path::PathBuf::from(std::env::args().nth(2).expect("Usage: $0 <output.elf> <input.elf>"));
    let input_filename2 = std::path::PathBuf::from(std::env::args().nth(3).expect("Usage: $0 <output.elf> <input.elf>"));

    let input_data = std::fs::read(&input_filename).expect("Could not read input file");
    let input_soname = input_filename.file_name().and_then(|name| name.to_str());
    let mut image = parse::parse_elf::<AnyEndian>(&input_data[..], input_soname).expect("Could not parse input file");

    let input_data2 = std::fs::read(&input_filename2).expect("Could not read input file 2");
    let input_soname2 = input_filename2.file_name().and_then(|name| name.to_str());
    let image2 = parse::parse_elf::<AnyEndian>(&input_data2[..], input_soname2).expect("Could not parse input file 2");

    image2.merge_into(&mut image);

    let new_file_data = emit::emit_elf(&image).expect("Could not emit output file");
    std::fs::write(&output_filename, new_file_data).expect("Could not write output file");
    make_executable(&output_filename).expect("Could not make output file executable");
}
