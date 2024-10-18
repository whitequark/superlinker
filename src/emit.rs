use object::elf::*;
use object::write::elf::{Class, FileHeader, ProgramHeader, Rel, SectionHeader, Sym, Writer};

use crate::repr::*;

fn make_static_str(s: impl AsRef<str>) -> &'static str {
    s.as_ref().to_owned().leak()
}

pub fn emit_elf(image: &Image) -> object::write::Result<Vec<u8>> {
    #[derive(Debug)]
    struct LoadSectionOut {
        index: object::write::elf::SectionIndex,
        name: object::write::StringId,
        mode: LoadMode,
        size: u64,
        addr: u64,
        load: bool,
    }

    #[derive(Debug)]
    #[allow(unused)]
    struct DynamicSymbolOut {
        index: object::write::elf::SymbolIndex,
        name: object::write::StringId,
        hash: u32,
    }

    let (class, is_rela, interp);
    if image.machine == object::elf::EM_X86_64 {
        class   = Class { is_64: true };
        is_rela = true;
        interp  = c"/lib/ld-musl-x86_64.so.1";
    } else {
        panic!("Unhandled machine: {}", image.machine)
    }

    let mut elf_data = Vec::new();
    let mut obj_writer = Writer::new(object::Endianness::Little, class.is_64, &mut elf_data);

    // Reserve space for file and program headers.
    // These are the things the dynamic linker cares about.
    obj_writer.reserve_file_header();
    let obj_phdr_offset = obj_writer.reserved_len();
    let phdr_count =
        /* PT_PHDR */1
        + /* PT_INTERP */1
        + /* PT_LOAD for ELF file and program headers */1
        + /* PT_LOAD for PT_DYNAMIC, etc */1
        + /* PT_DYNAMIC */1
        + /* PT_LOAD[..] */image.segments.len();
    obj_writer.reserve_program_headers(phdr_count as u32);
    let obj_interp_offset = obj_writer.reserve(interp.to_bytes_with_nul().len(), 1);
    let obj_headers_end = obj_writer.reserved_len();

    // Reserve space for dynamic linker information.
    // This is the stuff the dynamic linker *really* cares about.
    let mut out_needful = Vec::new();
    for dependency in image.dependencies.iter() {
        out_needful.push(obj_writer.add_dynamic_string(dependency.as_ref()));
    }
    let mut out_dynsyms = Vec::new();
    for symbol in image.symbols.iter() {
        let index = obj_writer.reserve_dynamic_symbol_index();
        let name = obj_writer.add_dynamic_string(symbol.name.as_ref());
        let hash = object::elf::hash(symbol.name.as_ref());
        out_dynsyms.push(DynamicSymbolOut { index, name, hash });
    }
    obj_writer.reserve(0, image.alignment as usize);
    let dynamic_count =
        /* DT_NEEDED */image.dependencies.len()
        + /* DT_STRTAB */1
        + /* DT_STRSZ */1
        + /* DT_SYMENT */1
        + /* DT_SYMTAB */1
        + /* DT_HASH */1
        + /* DT_REL(A) */1
        + /* DT_REL(A)SZ */1
        + /* DT_REL(A)ENT */1
        + /* DT_NULL */1;
    let obj_dynamic_offset = obj_writer.reserve_dynamic(dynamic_count);
    let obj_dynstr_offset = obj_writer.reserve_dynstr();
    let obj_dynstr_length = obj_writer.dynstr_len();
    let obj_dynsym_offset = obj_writer.reserve_dynsym();
    let hash_bucket_count = 4; // TODO: chosen at random
    let hash_index_base = 1; // null symbol
    let hash_chain_count = hash_index_base + out_dynsyms.len() as u32;
    let obj_hash_offset = obj_writer.reserve_hash(hash_bucket_count, hash_chain_count);
    let obj_reloc_offset = obj_writer.reserve_relocations(image.relocations.len(), is_rela);
    let obj_dynamic_end = obj_writer.reserved_len();

    // Reserve space for section headers.
    // This is the stuff that `objdump` cares about. Yes, even if there is a perfectly valid PT_DYNAMIC, it will look
    // for `.dynamic`/`.dynsym`/etc.
    obj_writer.reserve_null_section_index();
    obj_writer.reserve_shstrtab_section_index();
    obj_writer.reserve_dynamic_section_index();
    obj_writer.reserve_dynstr_section_index();
    let obj_dynsym_section_index = obj_writer.reserve_dynsym_section_index();
    obj_writer.reserve_hash_section_index();
    let _obj_reloc_dyn_section_index = obj_writer.reserve_section_index();
    let obj_reloc_dyn_section_name = obj_writer.add_section_name(if is_rela { b".rela.dyn" } else { b".rel.dyn" });
    let mut out_load_sections = Vec::new();
    for (segment_index, segment) in image.segments.iter().enumerate() {
        let mut make_section = |name, size, addr, load| {
            let index = obj_writer.reserve_section_index();
            let name = obj_writer.add_section_name(make_static_str(name).as_ref());
            out_load_sections.push(LoadSectionOut { index, name, mode: segment.mode, size, addr, load })
        };
        // A segment can be only partially mapped from disk, i.e. in the case of `p_filesz != 0 && p_filesz < p_memsz`.
        // Sections are either fully mapped or fully unmapped. Thus, we need to split the segment into two sections
        // to make this case work. (Remember that this is _still_ only for objdump.)
        let dataful_name = format!("image.{}.{}", segment_index, match segment.mode {
            LoadMode::ReadOnly => "ro",
            LoadMode::ReadWrite => "rw",
            LoadMode::ReadExecute => "rx",
        });
        let dataless_name = format!("image.{}.rwz", segment_index);
        if segment.data.len() as u64 == segment.size {
            make_section(dataful_name, segment.data.len() as u64, segment.addr, /*load=*/true);
        } else if segment.data.len() == 0 {
            make_section(dataless_name, segment.size, segment.addr, /*load=*/false);
        } else {
            make_section(dataful_name, segment.data.len() as u64, segment.addr, /*load=*/true);
            make_section(dataless_name, segment.size - segment.data.len() as u64,
                segment.addr + segment.data.len() as u64, /*load=*/false);
        }
    }
    obj_writer.reserve_shstrtab();
    obj_writer.reserve_section_headers();

    // Reserve space for image segments.
    let image_file_offset = obj_writer.reserve(0, image.alignment as usize);
    for segment in image.segments.iter() {
        assert!(segment.data.len() as u64 <= segment.size);
        obj_writer.reserve_until(image_file_offset + segment.addr as usize + segment.size as usize);
    }

    // Write file and program headers.
    obj_writer.write_file_header(&FileHeader {
        os_abi: 0,
        abi_version: 0,
        e_type: ET_DYN,
        e_machine: image.machine,
        e_entry: image_file_offset as u64 + image.entry,
        e_flags: 0,
    })?;
    // We use a 1:1 mapping between file offsets and virtual addresses (before rebasing). This is already how many
    // shared objects are laid out. It also simplifies both internal bookkeeping and debugging.
    let mut write_program_header = |type_, flags, offset, size, align| {
        obj_writer.write_program_header(&ProgramHeader {
            p_type: type_,
            p_flags: flags,
            p_offset: offset as u64,
            p_vaddr: offset as u64,
            p_paddr: offset as u64,
            p_filesz: size as u64,
            p_memsz: size as u64,
            p_align: align,
        })
    };
    // musl uses the difference between AT_PHDR and PT_PHDR to find out where the application is loaded, if it
    // is mapped by the kernel. Omitting this program header causes it to explode in a really amusing way.
    // As of Linux 6.10, the kernel always maps the application, and then if it has an interpreter, maps that too
    // and runs its entry point instead of the application's.
    write_program_header(PT_PHDR, PF_R,
        obj_phdr_offset, class.program_header_size() * phdr_count, class.align() as u64);
    // Kernel uses PT_INTERP to find out which interpreter to load.
    write_program_header(PT_INTERP, PF_R,
        obj_interp_offset, interp.to_bytes_with_nul().len(), /*align=*/1);
    // The ELF program headers must be loaded in order for the interpreter to be able to parse the file. Although
    // it is not required by the ABI to load the file headers, it's easier to do that anyway. (Most Linux binaries
    // do load them.)
    write_program_header(PT_LOAD, PF_R,
        0, obj_headers_end, image.alignment);
    // The ELF dynamic information must be loaded too, for the same reasons. The PT_DYNAMIC program header points
    // to the beginning of this information, which contains the dynamic table, and is followed by the entities
    // that are referenced by the table. These are mapped read-write since the interpreter modifies them in-place.
    write_program_header(PT_DYNAMIC, PF_R | PF_W,
        obj_dynamic_offset, class.dyn_size() * dynamic_count, class.align() as u64);
    write_program_header(PT_LOAD, PF_R | PF_W,
        obj_dynamic_offset, obj_dynamic_end - obj_dynamic_offset, class.align() as u64);
    // The image segments are loaded as-is. In the segments, `segment.size` could be bigger than `segment.data`, with
    // the remainder zeroed on load. Such a segment would be typically the last one. For our purposes this is
    // undesirable and we pad everything to the memory size.
    for segment in image.segments.iter() {
        let obj_flags = match segment.mode {
            LoadMode::ReadOnly => PF_R,
            LoadMode::ReadWrite => PF_R | PF_W,
            LoadMode::ReadExecute => PF_R | PF_X,
        };
        write_program_header(PT_LOAD, obj_flags,
            image_file_offset + segment.addr as usize, segment.size as usize, image.alignment);
    }

    // Write dynamic linker information.
    obj_writer.write(interp.to_bytes_with_nul());
    obj_writer.pad_until(obj_dynamic_offset);
    for out_needed in out_needful {
        obj_writer.write_dynamic_string(DT_NEEDED, out_needed); // do the needful
    }
    obj_writer.write_dynamic(DT_STRTAB, obj_dynstr_offset as u64);
    obj_writer.write_dynamic(DT_STRSZ, obj_dynstr_length as u64);
    obj_writer.write_dynamic(DT_SYMENT, class.sym_size() as u64);
    obj_writer.write_dynamic(DT_SYMTAB, obj_dynsym_offset as u64);
    obj_writer.write_dynamic(DT_HASH, obj_hash_offset as u64);
    obj_writer.write_dynamic(if is_rela { DT_RELA } else { DT_REL },
        obj_reloc_offset as u64);
    obj_writer.write_dynamic(if is_rela { DT_RELASZ } else { DT_RELSZ },
        (class.rel_size(is_rela) * image.relocations.len()) as u64);
    obj_writer.write_dynamic(if is_rela { DT_RELAENT } else { DT_RELENT },
        class.rel_size(is_rela) as u64);
    obj_writer.write_dynamic(DT_NULL, 0);
    obj_writer.write_dynstr();
    obj_writer.write_null_dynamic_symbol();
    for symbol in image.symbols.iter() {
        let obj_symtype = match symbol.kind {
            SymbolKind::Code => STT_FUNC,
            SymbolKind::Data => STT_OBJECT,
            SymbolKind::Unknown => STT_NOTYPE,
        };
        let obj_bind = match symbol.scope {
            SymbolScope::Local => STB_LOCAL,
            SymbolScope::Global => STB_GLOBAL,
            SymbolScope::Import => STB_GLOBAL,
            SymbolScope::Weak => STB_WEAK,
        };
        // In symbol tables, relocations must be associated with a section, even in an executable or shared object
        // where the address of the section is unimportant. Nevertheless, find which section they belong to.
        let (obj_value, obj_section);
        if symbol.value == 0 {
            obj_value = 0;
            obj_section = None;
        } else {
            obj_value = image_file_offset as u64 + symbol.value;
            obj_section = out_load_sections.iter().find_map(|&LoadSectionOut { addr, size, index, .. }| {
                // Neither `symbol` nor `out_load_sections` are relocated by `image_file_offset` here.
                if symbol.value >= addr && symbol.value < addr + size { Some(index) } else { None }
            })
        };
        obj_writer.write_dynamic_symbol(&Sym {
            name: Some(obj_writer.get_dynamic_string(symbol.name.as_ref())),
            section: obj_section,
            st_info: (obj_bind << 4) | obj_symtype,
            st_other: 0,
            st_shndx: 0, // automatically filled in if `section` is specified
            st_value: obj_value,
            st_size: symbol.size,
        });
    }
    obj_writer.write_hash(hash_bucket_count, hash_chain_count, |index| {
        Some(out_dynsyms.get(index.checked_sub(hash_index_base)? as usize)?.hash)
    });
    obj_writer.write_align_relocation();
    for relocation in image.relocations.iter() {
        let (obj_sym, obj_symtype, obj_addend);
        if image.machine == object::elf::EM_X86_64 {
            match relocation.target.clone() {
                RelocationTarget::Symbol { symbol: symbol_name, addend } => {
                    obj_sym = image.symbols.iter().position(|symbol|
                        symbol.name == symbol_name).map(|index| index + 1).unwrap_or(0) as u32;
                    obj_symtype = R_X86_64_64;
                    obj_addend = addend;
                },
                RelocationTarget::Base { addend } => {
                    obj_sym = 0;
                    obj_symtype = R_X86_64_RELATIVE;
                    obj_addend = image_file_offset as i64 + addend;
                },
            }
        } else {
            unreachable!()
        }
        obj_writer.write_relocation(is_rela, &Rel {
            // In executables and shared libraries, relocations are applied at a virtual address.
            r_offset: image_file_offset as u64 + relocation.offset,
            r_sym: obj_sym,
            r_type: obj_symtype,
            r_addend: obj_addend,
        });
    }

    // Write section headers.
    obj_writer.write_shstrtab();
    obj_writer.write_null_section_header();
    obj_writer.write_shstrtab_section_header();
    obj_writer.write_dynamic_section_header(obj_dynamic_offset as u64);
    obj_writer.write_dynstr_section_header(obj_dynstr_offset as u64);
    obj_writer.write_dynsym_section_header(obj_dynsym_offset as u64, 1);
    obj_writer.write_hash_section_header(obj_hash_offset as u64);
    obj_writer.write_section_header(&SectionHeader {
        name: Some(obj_reloc_dyn_section_name),
        sh_type: if is_rela { SHT_RELA } else { SHT_REL },
        sh_flags: SHF_ALLOC as u64,
        sh_addr: obj_reloc_offset as u64,
        sh_offset: obj_reloc_offset as u64,
        sh_size: (class.rel_size(is_rela) * image.relocations.len()) as u64,
        sh_link: obj_dynsym_section_index.0,
        sh_info: 0,
        sh_addralign: class.rel_size(is_rela) as u64,
        sh_entsize: class.rel_size(is_rela) as u64,
    });
    for out_load_section in out_load_sections {
        let sh_flags = match out_load_section.mode {
            LoadMode::ReadOnly => SHF_ALLOC,
            LoadMode::ReadWrite => SHF_ALLOC | SHF_WRITE,
            LoadMode::ReadExecute => SHF_ALLOC | SHF_EXECINSTR,
        };
        obj_writer.write_section_header(&SectionHeader {
            name: Some(out_load_section.name),
            sh_type: if out_load_section.load { SHT_PROGBITS } else { SHT_NOBITS },
            sh_flags: sh_flags as u64,
            sh_addr: image_file_offset as u64 + out_load_section.addr,
            sh_offset: image_file_offset as u64 + out_load_section.addr,
            sh_size: out_load_section.size,
            sh_link: SHN_UNDEF as u32,
            sh_info: 0,
            sh_addralign: image.alignment,
            sh_entsize: 0,
        });
    }

    // Write image segments.
    for segment in image.segments.iter() {
        obj_writer.pad_until(image_file_offset + segment.addr as usize);
        obj_writer.write(segment.data.as_ref());
        obj_writer.pad_until(image_file_offset + segment.addr as usize + segment.size as usize);
    }

    // If the reserved amount and written amount are the same, the file is probably good.
    assert_eq!(obj_writer.reserved_len(), obj_writer.len());

    Ok(elf_data)
}
