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
    let output_filename = std::path::PathBuf::from(std::env::args().nth(1).expect("Usage: $0 <output.elf> <input.elf> <merge.elf>..."));
    let input_filename = std::path::PathBuf::from(std::env::args().nth(2).expect("Usage: $0 <output.elf> <input.elf> <merge.elf>..."));
    let merge_filenames = std::env::args().skip(3).map(|arg| std::path::PathBuf::from(arg));

    let input_data = std::fs::read(&input_filename).expect("Could not read input file");
    let input_soname = input_filename.file_name().and_then(|name| name.to_str());
    let mut input_image = parse::parse_elf::<AnyEndian>(
        &input_data[..], input_filename.to_str().unwrap(), input_soname).expect("Could not parse input file");

    for merge_filename in merge_filenames {
        let merge_data = std::fs::read(&merge_filename).expect("Could not read merge file");
        let merge_soname = merge_filename.file_name().and_then(|name| name.to_str());
        let merge_image = parse::parse_elf::<AnyEndian>(
            &merge_data[..], merge_filename.to_str().unwrap(), merge_soname).expect("Could not parse merge file");
        merge_image.merge_into(&mut input_image);
    }

    let output_data = emit::emit_elf(&input_image).expect("Could not emit output file");
    std::fs::write(&output_filename, output_data).expect("Could not write output file");
    make_executable(&output_filename).expect("Could not make output file executable");
}
