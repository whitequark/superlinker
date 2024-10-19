use object::{elf::*, Endianness};
use object::write::elf::{Class, FileHeader, ProgramHeader, Rel, SectionHeader, Sym, Writer};

use crate::repr::*;

fn make_static_str(s: impl AsRef<str>) -> &'static str {
    s.as_ref().to_owned().leak()
}

fn make_shim(
    machine: u16,
    base: u64,
    interp_base: u64,
    interp_phdrs: usize,
    interp_entry: u64,
    user_entry: u64
) -> Vec<u8> {
    // When the interpreter is loaded by the kernel, the kernel communicates several key parameters to it through
    // the auxiliary vector; most importantly, AT_BASE, AT_ENTRY, and AT_PH*. For the dynamic loader to function,
    // AT_BASE must be set to its own ELF header (to which it maintains an internal PC-relative reference).
    // For the dynamic loader to jump to the executable after loading, AT_ENTRY must be set to the user entry point
    // (`e_entry` of the PIE). For the dynamic loader to relocate the executable, AT_PH* must be pointing to the user
    // executable's entry point.
    //
    // All of these parameters are configured by the kernel when it's launching an interpreter via the PT_INTERP
    // mechanism. However, if we link the interpreter in, the kernel will instead point these parameters to our
    // combined executable. Luckily, AT_PH* already have the right values, so the only modifications needed are
    // to AT_BASE (which *must* point to the `\x7FELF` of the interpreter) and AT_ENTRY (which must point to
    // the PIE entry point). Since we interpose this shim using the `e_entry` file header field, we must restore
    // the original `e_entry` by modifying `AT_ENTRY`.
    //
    // The shim consists of a code part (blob built from shim/ in build.rs and included) and data part (built here).
    // The code part must be completely position independent (no relocations) and padding to align the data part
    // must be included in the blob. The code part reads the data part using pc-relative addresses.

    macro_rules! shim_blob {
        ($target:expr) => {
            include_bytes!(concat!(env!("OUT_DIR"), "/shim/", $target, "/shim.bin"))
        };
    }

    let code = if machine == EM_X86_64 {
        shim_blob!("x86_64-unknown-none").to_vec()
    } else {
        panic!("Shim not implemented for machine: {:?}", machine)
    };

    let mut code = code.to_vec();

    // Append data part of shim

    // Keep in sync with shim/main.rs
    // TODO: Handle 32-bit stuff
    code.extend(user_entry.wrapping_sub(base).to_le_bytes());
    code.extend(interp_entry.wrapping_sub(base).to_le_bytes());
    code.extend(interp_base.wrapping_sub(base).to_le_bytes());
    code.extend((interp_phdrs as u64).to_le_bytes());

    code.resize(((code.len() - 1) | 0xff) + 1, 0); // pad to make it easier to edit in binja
    code
}

pub fn emit_elf(image: &Image) -> object::write::Result<Vec<u8>> {
    #[derive(Debug)]
    enum InterpreterOut {
        Path { bytes: Vec<u8> },
        Shim { base: u64, entry: u64, phdrs: usize, code_len: usize },
        None,
    }

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

    let (endian, class, is_rela);
    if image.machine == object::elf::EM_X86_64 {
        endian  = Endianness::Little;
        class   = Class { is_64: true };
        is_rela = true;
    } else {
        panic!("Unhandled machine: {}", image.machine)
    }

    let out_interp = match image.interpreter {
        Interpreter::External(ref path) => {
            let mut bytes = path.as_bytes().to_vec();
            bytes.push(0);
            InterpreterOut::Path { bytes }
        }
        Interpreter::Internal { base, entry, segments: phdrs } => {
            let code = make_shim(image.machine, 0, 0, 0, 0, 0); // can't resolve references yet
            InterpreterOut::Shim { base, entry, phdrs, code_len: code.len() }
        },
        Interpreter::Absent =>
            InterpreterOut::None
    };

    let mut out_gdb_script = Vec::new();
    out_gdb_script.extend_from_slice(b"\x04superlinker\n");
    out_gdb_script.extend_from_slice(br#"
initialized = False

def new_objfile_handler(event):
    global initialized
    if not initialized:
        initialized = True
        init_debugging()

def clear_objfiles_handler(event):
    global initialized
    initialized = False

gdb.events.new_objfile.connect(new_objfile_handler)
gdb.events.clear_objfiles.connect(clear_objfiles_handler)

def init_debugging():
    load_addr = int(gdb.parse_and_eval("&__elf_header"))
    print(f"Program loaded at: {load_addr:#x}")
"#);

    let mut elf_data = Vec::new();
    let mut obj_writer = Writer::new(endian, class.is_64, &mut elf_data);

    // Reserve space for file and program headers.
    // These are the things the dynamic linker cares about.
    obj_writer.reserve_file_header();
    let obj_phdr_offset = obj_writer.reserved_len();
    let interp_phdr_count = match &out_interp {
        InterpreterOut::Path { .. } => /* PT_INTERP */1,
        InterpreterOut::Shim { .. } => /* PT_LOAD */1,
        InterpreterOut::None => 0,
    };
    let phdr_count =
        /* PT_PHDR */1
        + /* PT_LOAD for ELF file and program headers */1
        + /* PT_INTERP or PT_LOAD for interpreter thunk */interp_phdr_count
        + /* PT_LOAD for PT_DYNAMIC, etc */1
        + /* PT_DYNAMIC */1
        + /* PT_LOAD[..] */image.segments.len();
    obj_writer.reserve_program_headers(phdr_count as u32);
    let obj_interp_offset = if let InterpreterOut::Path { bytes } = &out_interp {
        obj_writer.reserve(bytes.len(), 1)
    } else { 0 };
    let obj_headers_end = obj_writer.reserved_len();
    let obj_shim_offset = if let InterpreterOut::Shim { code_len, .. } = out_interp {
        obj_writer.reserve(code_len, image.alignment as usize)
    } else { 0 };

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
    {
        let symbol_name = b"__elf_header";
        let index = obj_writer.reserve_dynamic_symbol_index();
        let name = obj_writer.add_dynamic_string(symbol_name);
        let hash = object::elf::hash(symbol_name.as_ref());
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
        + /* DT_NULL */1
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
    let obj_shim_section_index_name = if let InterpreterOut::Shim { .. } = out_interp {
        Some((obj_writer.reserve_section_index(), obj_writer.add_section_name(b"shim")))
    } else { None };
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
    let _obj_gdb_script_section_index = obj_writer.reserve_section_index();
    let obj_gdb_script_section_name = obj_writer.add_section_name(b".debug_gdb_scripts");

    obj_writer.reserve_shstrtab();
    obj_writer.reserve_section_headers();

    // Reserve space for image segments.
    let image_file_offset = obj_writer.reserve(0, image.alignment as usize);
    for segment in image.segments.iter() {
        assert!(segment.data.len() as u64 <= segment.size);
        obj_writer.reserve_until(image_file_offset + segment.addr as usize + segment.size as usize);
    }

    // Reserve space for GDB script.
    for segment in image.segments.iter() {
        match segment {
            LoadSegment { addr: segment_addr, mode: LoadMode::ReadExecute, file: Some(ref filename), .. } => {
                let addr = image_file_offset as u64 + segment_addr;
                let script_line = format!("    gdb.execute(f'add-symbol-file {} {{load_addr + {}:#x}}')\n", filename, addr);
                out_gdb_script.extend_from_slice(script_line.as_bytes())
            }
            _ => ()
        }
    }
    let gdb_script_offset = obj_writer.reserve(out_gdb_script.len() + 1, 0);

    // Write file and program headers.
    let entry = match &out_interp {
        InterpreterOut::Path { .. } => image_file_offset as u64 + image.entry,
        InterpreterOut::Shim { .. } => obj_shim_offset as u64,
        InterpreterOut::None => 0,
    };
    obj_writer.write_file_header(&FileHeader {
        os_abi: 0,
        abi_version: 0,
        e_type: ET_DYN,
        e_machine: image.machine,
        e_entry: entry,
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
    // The ELF program headers must be loaded in order for the interpreter to be able to parse the file. Although
    // it is not required by the ABI to load the file headers, it's easier to do that anyway. (Most Linux binaries
    // do load them.)
    write_program_header(PT_LOAD, PF_R,
        0, obj_headers_end, image.alignment);
    match &out_interp {
        InterpreterOut::Path { bytes } =>
            // Kernel uses PT_INTERP to find out which interpreter to load.
            write_program_header(PT_INTERP, PF_R,
                obj_interp_offset, bytes.len(), /*align=*/1),
        InterpreterOut::Shim { code_len, .. } =>
            // Shim uses kernel ABI to bootstrap the built-in interpreter.
            write_program_header(PT_LOAD, PF_R | PF_X,
                obj_shim_offset, *code_len, /*align=*/image.alignment),
        InterpreterOut::None => ()
    }
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
    match &out_interp {
        InterpreterOut::Path { bytes } => {
            obj_writer.pad_until(obj_interp_offset);
            obj_writer.write(&bytes);
        }
        InterpreterOut::Shim { base: interp_base, entry: interp_entry, phdrs: interp_phdrs, code_len } => {
            let code = make_shim(
                image.machine,
                obj_shim_offset as u64,
                image_file_offset as u64 + *interp_base,
                *interp_phdrs,
                image_file_offset as u64 + *interp_entry,
                image_file_offset as u64 + image.entry
            );
            assert_eq!(code.len(), *code_len);
            obj_writer.pad_until(obj_shim_offset);
            obj_writer.write(&code);
        }
        InterpreterOut::None => (),
    }
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
        let (obj_value, obj_section, obj_shndx);
        if symbol.abs {
            obj_value = symbol.value;
            obj_section = None;
            obj_shndx = SHN_ABS;
        } else if symbol.value == 0 {
            obj_value = 0;
            obj_section = None;
            obj_shndx = 0;
        } else {
            obj_value = image_file_offset as u64 + symbol.value;
            obj_section = out_load_sections.iter().find_map(|&LoadSectionOut { addr, size, index, .. }| {
                // Neither `symbol` nor `out_load_sections` are relocated by `image_file_offset` here.
                if symbol.value >= addr && symbol.value < addr + size { Some(index) } else { None }
            });
            obj_shndx = 0;
        };
        obj_writer.write_dynamic_symbol(&Sym {
            name: Some(obj_writer.get_dynamic_string(symbol.name.as_ref())),
            section: obj_section,
            st_info: (obj_bind << 4) | obj_symtype,
            st_other: 0,
            st_shndx: obj_shndx, // automatically filled in if `section` is specified
            st_value: obj_value,
            st_size: symbol.size,
        });
    }
    obj_writer.write_dynamic_symbol(&Sym {
        name: Some(obj_writer.get_dynamic_string(b"__elf_header")),
        section: None,
        st_info: (STB_GLOBAL << 4) | STT_OBJECT,
        st_other: 0,
        st_shndx: 1,
        st_value: 0,
        st_size: 1,
    });
    obj_writer.write_hash(hash_bucket_count, hash_chain_count, |index| {
        Some(out_dynsyms.get(index.checked_sub(hash_index_base)? as usize)?.hash)
    });
    obj_writer.write_align_relocation();
    let find_symbol = |name|
        image.symbols.iter().position(|symbol| symbol.name == name).map(|index| index + 1).unwrap_or(0) as u32;
    for relocation in image.relocations.iter() {
        let (obj_reltype, obj_relsym, obj_addend);
        if image.machine == object::elf::EM_X86_64 {
            match relocation.target.clone() {
                RelocationTarget::Symbol { symbol: symbol_name, addend } => {
                    obj_reltype = R_X86_64_64;
                    obj_relsym = find_symbol(symbol_name);
                    obj_addend = addend;
                },
                RelocationTarget::Base { addend } => {
                    obj_reltype = R_X86_64_RELATIVE;
                    obj_relsym = 0;
                    obj_addend = image_file_offset as i64 + addend;
                },
                RelocationTarget::Copy { symbol: symbol_name } => {
                    obj_reltype = R_X86_64_COPY;
                    obj_relsym = find_symbol(symbol_name);
                    obj_addend = 0;
                },
                RelocationTarget::None => {
                    obj_reltype = R_X86_64_NONE;
                    obj_relsym = 0;
                    obj_addend = 0;
                }
            }
        } else {
            unreachable!()
        }
        obj_writer.write_relocation(is_rela, &Rel {
            // In executables and shared libraries, relocations are applied at a virtual address.
            r_offset: image_file_offset as u64 + relocation.offset,
            r_sym: obj_relsym,
            r_type: obj_reltype,
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
    if let InterpreterOut::Shim { code_len, .. } = out_interp {
        if let Some((_obj_shim_section_index, obj_shim_section_name)) = obj_shim_section_index_name {
            obj_writer.write_section_header(&SectionHeader {
                name: Some(obj_shim_section_name),
                sh_type: SHT_PROGBITS,
                sh_flags: SHF_ALLOC as u64,
                sh_addr: obj_shim_offset as u64,
                sh_offset: obj_shim_offset as u64,
                sh_size: code_len as u64,
                sh_link: SHN_UNDEF as u32,
                sh_info: 0,
                sh_addralign: class.align() as u64,
                sh_entsize: class.align() as u64,
            });
        } else { unreachable!() }
    }
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
    obj_writer.write_section_header(&SectionHeader {
        name: Some(obj_gdb_script_section_name),
        sh_type: SHT_PROGBITS,
        sh_flags: 0,
        sh_addr: 0,
        sh_offset: gdb_script_offset as u64,
        sh_size: (out_gdb_script.len() + 1) as u64,
        sh_link: SHN_UNDEF as u32,
        sh_info: 0,
        sh_addralign: 1,
        sh_entsize: 0,
    });

    // Write image segments.
    for segment in image.segments.iter() {
        obj_writer.pad_until(image_file_offset + segment.addr as usize);
        obj_writer.write(segment.data.as_ref());
        obj_writer.pad_until(image_file_offset + segment.addr as usize + segment.size as usize);
    }

    // Write GDB script.
    obj_writer.write(&out_gdb_script);
    obj_writer.write(&[0]);

    // If the reserved amount and written amount are the same, the file is probably good.
    assert_eq!(obj_writer.reserved_len(), obj_writer.len());

    Ok(elf_data)
}
