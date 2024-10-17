use elf::abi::*;
use elf::{endian::AnyEndian, ElfBytes};

fn main() {
    let entry_file = std::env::args().nth(1).expect("Usage: $0 <file.elf>");

    let path = std::path::PathBuf::from(entry_file);
    let file_data = std::fs::read(path).expect("Could not read file");
    let slice = file_data.as_slice();
    let file = ElfBytes::<AnyEndian>::minimal_parse(slice).expect("Could not parse ELF");

    for segment in file.segments().unwrap() {
        match segment.p_type {
            PT_PHDR => {
                // PT_PHDR indicates exactly where in the image program headers are. I (whitequark)
                // seem to recall its main use to be in exception handling, for the unwinder to be
                // able to locate the tables pointed to by PT_GNU_EH_FRAME. We don't need to do
                // anything with the existing PT_PHDR segment, but we may have to produce our own
                // for unwinding to work. (The API used by the unwinder is `dl_iterate_phdr`. I'm
                // unsure how statically linked C++ applications unwind, but presumably there is
                // some kind of symbol known by the unwinder and the linker.)
                //
                // As far as I know, whenever there is PT_PHDR in the image, there must be also
                // a corresponding PT_LOAD that makes sure the program headers are actually mapped.
                // See also the `PHDRS` linker script directive:
                // https://ftp.gnu.org/old-gnu/Manuals/ld-2.9.1/html_node/ld_23.html
            }
            PT_INTERP => {
                let data = file.segment_data(&segment).expect("Could not get data for PT_INTERP");
                let data = std::ffi::CStr::from_bytes_with_nul(data).expect("Expected PT_INTERP to be a null terminated string");
                let data = data.to_str().expect("Could not convert PT_INTERP path to string");
                let interp_path = std::path::PathBuf::from(data);
                println!("found PT_INTERP! path={:?}", interp_path);
            }
            PT_LOAD => {
                // Loadable section. Could contain ELF headers, executable code, read-only data,
                // read-write data, PLT, GOT, string and symbol tables, relocations, and so on.
                // The format does not provide any visibility into what the data inside is
                // (the static liner is supposed to prepare everything so that this isn't necessary;
                // information needed for dynamic linking lives in PT_DYNAMIC), it just tells us
                // where to load it to, where to get the contents, and which memory protection
                // options to use.
                println!("found PT_LOAD! segment={:?}", segment);
            }
            PT_DYNAMIC => {
                // Information for dynamic linking. That's us! We're the dynamic linker. The static
                // dynamic linker (aka superlinker, because linking isn't confusing enough and I
                // cannot  go along writing a linker without inventing a new, mostly superfluous
                // term). This segment contains a massive amount of information that overlaps with
                // the information in the section table; it is in many ways a copy of the section
                // table (which the dynamic linker should not look at) that is simplified or adapted
                // for runtime operation.
                let data = file.segment_data(&segment).expect("Could not get data for PT_DYNAMIC");
                let dynamic_table = elf::dynamic::DynamicTable::new(file.ehdr.endianness, file.ehdr.class, data);
                println!("found DT_DYNAMIC!");
                for dynamic_entry in dynamic_table {
                    println!("  entry d_tag={} d_val={:#x}",
                        elf::to_str::d_tag_to_str(dynamic_entry.d_tag).unwrap_or("<unknown>"),
                        dynamic_entry.d_val());
                }
            }
            PT_GNU_STACK => {
                // Controls whether the stack is executable, via the segment flags.
                if segment.p_flags == PF_R|PF_W {
                    println!("found PT_STACK! stack is non-executable")
                } else if segment.p_flags == PF_R|PF_W|PF_X {
                    println!("found PT_STACK! stack is executable")
                } else {
                    panic!("Flags for PT_STACK are not RW or RWX")
                }
            }
            PT_GNU_RELRO => {
                // Tells the dynamic linker to make certain ELF file structures read-only after
                // it is done with linking, to harden the memory image against exploits using
                // out-of-bounds writes to these structures. We can ignore this mitigation entirely.
            }
            p_type => {
                panic!("Unhandled segment type {:?}", elf::to_str::p_type_to_str(p_type))
            }
        }
    }
}
