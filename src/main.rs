use elf::endian::AnyEndian;

mod repr;
mod parse;

fn main() {
    let entry_file = std::env::args().nth(1).expect("Usage: $0 <file.elf>");
    let file_data = std::fs::read(entry_file).expect("Could not read file");
    // let file = ElfBytes::<NativeEndian>::minimal_parse(&file_data).expect("Could not parse ELF");
    let image = parse::parse_elf::<AnyEndian>(&file_data[..]);
    dbg!(image);
}
