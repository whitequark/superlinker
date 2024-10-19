use elf::abi::*;
use elf::endian::EndianParse;
use elf::relocation::RelaIterator;
use elf::segment::ProgramHeader;
use elf::ElfBytes;

use crate::repr::*;

pub const DT_RELR: i64 = 36;
pub const DT_RELRSZ: i64 = 35;

pub fn parse_elf<E: EndianParse>(elf_data: &[u8], soname: Option<&str>) -> Result<Image, elf::parse::ParseError> {
    let elf_file = ElfBytes::<E>::minimal_parse(elf_data)?;
    let machine = elf_file.ehdr.e_machine;
    let elf_common = elf_file.find_common_data()?;
    let elf_segments = elf_file.segments().expect("No segments");
    let alignment = elf_segments
        .iter()
        .filter_map(|elf_segment| {
            if elf_segment.p_type == PT_LOAD { Some(elf_segment.p_align) } else { None }
        })
        .max()
        .unwrap_or(1);
    let segments = elf_segments
        .iter()
        .filter_map(|elf_segment| {
            if elf_segment.p_type == PT_LOAD {
                let addr = elf_segment.p_vaddr;
                let size = elf_segment.p_memsz;
                let data = elf_file.segment_data(&elf_segment)
                    .expect("No data for PT_LOAD")
                    .to_owned();
                let mode = if elf_segment.p_flags == PF_R {
                    LoadMode::ReadOnly
                } else if elf_segment.p_flags == PF_R | PF_W {
                    LoadMode::ReadWrite
                } else if elf_segment.p_flags == PF_R | PF_X {
                    LoadMode::ReadExecute
                } else {
                    panic!("Unknown segment flags: {}",
                        elf::to_str::p_flags_to_string(elf_segment.p_flags))
                };
                Some(LoadSegment { addr, size, data, mode })
            } else {
                None
            }

        })
        .collect::<Vec<_>>();
    let tls_image = elf_segments
        .iter()
        .find_map(|elf_segment| {
            if elf_segment.p_type == PT_TLS {
                let mut tls_image = vec![0; elf_segment.p_memsz as usize];
                let data = elf_file.segment_data(&elf_segment)
                    .expect("No data for PT_TLS")
                    .to_owned();
                tls_image[..data.len()].copy_from_slice(&data[..]);
                Some(tls_image)
            } else {
                None
            }
        });
    let elf_dynsyms = elf_common.dynsyms.as_ref().expect("No dynamic symbol table");
    let elf_dynsyms_strs = elf_common.dynsyms_strs.as_ref().expect("No dynamic symbol string table");
    let symbols = elf_dynsyms
        .clone()
        .into_iter()
        .skip(1)
        .filter_map(|elf_symbol| {
            // The type of the symbol can be `STT_NOTYPE` if it is a reference to a symbol that the static linker could
            // not discover at link time. This is independent of how the symbol was declared in C, i.e. `extern int a;`,
            // `extern int a(void);`, and `extern double a;` all become `STT_NOTYPE` when the symbol isn't resolved.
            // Weak symbols generally end up as `STT_NOTYPE`, unless defined in the same object.
            let elf_symtype = elf_symbol.st_symtype();
            if elf_symtype == STT_FUNC || elf_symtype == STT_OBJECT || elf_symtype == STT_NOTYPE {
                let name = elf_dynsyms_strs
                    .get(elf_symbol.st_name as usize)
                    .expect("Invalid symbol name")
                    .to_owned();
                let kind = if elf_symtype == STT_FUNC {
                    SymbolKind::Code
                } else if elf_symtype == STT_OBJECT {
                    SymbolKind::Data
                } else {
                    SymbolKind::Unknown
                };
                let value = elf_symbol.st_value;
                let scope = if elf_symbol.st_bind() == STB_GLOBAL {
                    if elf_symbol.is_undefined() {
                        SymbolScope::Import
                    } else {
                        SymbolScope::Global
                    }
                } else if elf_symbol.st_bind() == STB_WEAK {
                    SymbolScope::Weak
                } else if elf_symbol.st_bind() == STB_LOCAL {
                        SymbolScope::Local
                } else {
                    panic!("Unhandled symbol visibility: {}",
                        elf::to_str::st_bind_to_str(elf_symbol.st_bind()).unwrap_or("<unknown>"))
                };
                if elf_symbol.st_shndx == SHN_COMMON {
                    panic!("Unhandled special shndx {:#x}", elf_symbol.st_shndx);
                }
                let size = elf_symbol.st_size;
                Some(Symbol { name, kind, scope, value, size, abs: (elf_symbol.st_shndx == SHN_ABS) })
            } else if elf_symtype == STT_TLS {
                panic!("Unhangled STT_TLS symbol");
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    let elf_dynamic = elf_common.dynamic.map(|elf_dynamic| {
        elf_dynamic.into_iter().collect::<Vec<_>>()
    }).unwrap_or(Vec::new());
    let parse_elf_rela = |elf_rela_data| {
        RelaIterator::new(elf_file.ehdr.endianness, elf_file.ehdr.class, elf_rela_data)
            .map(|elf_rela| {
                let offset = elf_rela.r_offset;
                let target = if elf_file.ehdr.e_machine == EM_X86_64 {
                    let symbol = if elf_rela.r_sym == 0 { None } else {
                        let elf_symbol = elf_dynsyms
                            .get(elf_rela.r_sym as usize)
                            .expect("Invalid symbol reference in relocation");
                        let elf_symbol_name = elf_dynsyms_strs
                            .get(elf_symbol.st_name as usize)
                            .expect("Invalid symbol name in relocation");
                        Some(elf_symbol_name.to_owned())
                    };
                    // Both `R_X86_64_GLOB_DAT` and `R_X86_64_JUMP_SLOT` relocations can be expressed in terms of
                    // the more general and less optimized `R_X86_64_64` relocation, which is what the emitter is using.
                    if elf_rela.r_type == R_X86_64_64 {
                        RelocationTarget::Symbol {
                            symbol: symbol.expect("R_X86_64_64 requires a symbol"),
                            addend: elf_rela.r_addend
                        }
                    } else if elf_rela.r_type == R_X86_64_GLOB_DAT {
                        RelocationTarget::Symbol {
                            symbol: symbol.expect("R_X86_64_GLOB_DAT requires a symbol"),
                            addend: elf_rela.r_addend
                        }
                    } else if elf_rela.r_type == R_X86_64_JUMP_SLOT {
                        RelocationTarget::Symbol {
                            symbol: symbol.expect("R_X86_64_JUMP_SLOT requires a symbol"),
                            addend: elf_rela.r_addend
                        }
                    } else if elf_rela.r_type == R_X86_64_RELATIVE {
                        assert!(elf_rela.r_sym == 0, "R_X86_64_RELATIVE accepts no symbol");
                        RelocationTarget::Base { addend: elf_rela.r_addend }
                    } else if elf_rela.r_type == R_X86_64_COPY {
                        RelocationTarget::Copy {
                            symbol: symbol.expect("R_X86_64_COPY requires a symbol"),
                        }
                    } else if [R_X86_64_DTPMOD64].contains(&elf_rela.r_type) {
                        assert!(elf_rela.r_sym == 0, "Generic relocation mechanism accepts no symbol");
                        assert!(elf_rela.r_addend == 0, "Generic relocation mechanism accepts no addend");
                        RelocationTarget::ElfSpecific(elf_rela.r_type)
                    } else {
                        panic!("Unhandled relocation type: {}", elf_rela.r_type)
                    }
                } else {
                    panic!("Unhandled machine for RELA relocations: {}",
                        elf::to_str::e_machine_to_str(elf_file.ehdr.e_machine)
                        .unwrap_or("<unknown>"))
                };
                Relocation { offset, target }
            })
            .collect::<Vec<_>>()
    };
    let elf_dynamic_rela = elf_dynamic.iter().find_map(|elf_dyn| {
        if elf_dyn.d_tag == DT_RELA { Some(elf_dyn.clone().d_val() as usize) } else { None }
    });
    let elf_dynamic_relasz = elf_dynamic.iter().find_map(|elf_dyn| {
        if elf_dyn.d_tag == DT_RELASZ { Some(elf_dyn.clone().d_val() as usize) } else { None }
    });
    let elf_dynamic_pltrel = elf_dynamic.iter().find_map(|elf_dyn| {
        if elf_dyn.d_tag == DT_PLTREL { Some(elf_dyn.clone().d_val() as i64) } else { None }
    });
    let elf_dynamic_jmprel = elf_dynamic.iter().find_map(|elf_dyn| {
        if elf_dyn.d_tag == DT_JMPREL { Some(elf_dyn.clone().d_val() as usize) } else { None }
    });
    let elf_dynamic_pltrelsz = elf_dynamic.iter().find_map(|elf_dyn| {
        if elf_dyn.d_tag == DT_PLTRELSZ { Some(elf_dyn.clone().d_val() as usize) } else { None }
    });
    let elf_dynamic_relr = elf_dynamic.iter().find_map(|elf_dyn| {
        if elf_dyn.d_tag == DT_RELR { Some(elf_dyn.clone().d_val() as usize) } else { None }
    });
    let elf_dynamic_relrsz = elf_dynamic.iter().find_map(|elf_dyn| {
        if elf_dyn.d_tag == DT_RELRSZ { Some(elf_dyn.clone().d_val() as usize) } else { None }
    });
    let mut data_relocations = match (elf_dynamic_rela, elf_dynamic_relasz) {
        (Some(elf_dynamic_rela), Some(elf_dynamic_relasz)) =>
            parse_elf_rela(&elf_data[elf_dynamic_rela..elf_dynamic_rela + elf_dynamic_relasz]),
        (None, None) => Vec::new(),
        _ => panic!("Expected dynamic table to have both or neither of PT_RELA and PT_RELASZ")
    };
    let mut code_relocations = match (elf_dynamic_pltrel, elf_dynamic_jmprel, elf_dynamic_pltrelsz) {
        (Some(elf_dynamic_pltrel), Some(elf_dynamic_jmprel), Some(elf_dynamic_pltrelsz))
                if elf_dynamic_pltrel == DT_RELA => {
            let elf_jmprel_data = &elf_data[elf_dynamic_jmprel..elf_dynamic_jmprel + elf_dynamic_pltrelsz];
            if elf_dynamic_pltrel == DT_RELA {
                parse_elf_rela(elf_jmprel_data)
            // } else if elf_dynamic_pltrel == DT_REL {
            //     parse_elf_rel(elf_pltrel_data)
            } else {
                panic!("Unhandled PLT relocation type: {}",
                    elf::to_str::d_tag_to_str(elf_dynamic_pltrel)
                    .unwrap_or("<unknown>"));
            }
        }
        (None, None, None) => Vec::new(),
        _ => panic!("Expected dynamic table to have all or none of PT_PLTREL, PT_JMPREL, and PT_PLTRELSZ")
    };
    let mut relr_relocations = Vec::new();
    match (elf_dynamic_relr, elf_dynamic_relrsz) {
        (Some(elf_dynamic_relr), Some(elf_dynamic_relrsz)) => {
            let parse = E::from_ei_data(elf_data[EI_DATA]).unwrap();
            let mut segment_for_addend = None::<ProgramHeader>;
            let mut get_addend = |addr| {
                segment_for_addend = match segment_for_addend {
                    Some(segment) if addr >= segment.p_vaddr && addr < segment.p_vaddr + segment.p_memsz =>
                        segment_for_addend,
                    _ => elf_segments.iter().find(|segment|
                        addr >= segment.p_vaddr && addr < segment.p_vaddr + segment.p_memsz)
                };
                let mut file_offset = segment_for_addend
                    .map(|segment| addr + segment.p_offset - segment.p_vaddr)
                    .expect("Relr target outside of all segments") as usize;
                parse.parse_i64_at(&mut file_offset, elf_data).unwrap()
            };
            let mut push_relr = |addr|
                relr_relocations.push(Relocation {
                    offset: addr,
                    target: RelocationTarget::Base { addend: get_addend(addr) }
                });
            let elf_relr_data = &elf_data[elf_dynamic_relr..elf_dynamic_relr + elf_dynamic_relrsz];
            let mut offset = 0;
            let mut next_rel = 0;
            while offset < elf_relr_data.len() {
                let mut entry = parse.parse_u64_at(&mut offset, elf_relr_data).unwrap();
                if (entry & 1) == 0 {
                    push_relr(entry as u64);
                    next_rel = entry + 8;
                } else {
                    let mut iter_rel = next_rel;
                    while (entry & !1) != 0 {
                        entry >>= 1;
                        if entry & 1 == 1 {
                            push_relr(iter_rel as u64);
                        }
                        iter_rel += 8;
                    }
                    next_rel = next_rel + 8 * 63;
                }
            }
        }
        (None, None) => (),
        _ => panic!("Expected dynamic table to have all or none of DT_RELR and DT_RELRSZ")
    };
    let mut relocations = Vec::new();
    relocations.append(&mut relr_relocations); // ABI suggests processing Relr first
    relocations.append(&mut data_relocations);
    relocations.append(&mut code_relocations);
    let dependencies = elf_dynamic.iter().filter_map(|elf_dyn| {
        if elf_dyn.d_tag == DT_NEEDED {
            Some(elf_dynsyms_strs
                .get(elf_dyn.clone().d_val() as usize)
                .expect("Invalid DT_NEEDED name")
                .to_owned())
        } else {
            None
        }
    }).collect::<Vec<_>>();
    let elf_dynamic_init = elf_dynamic.iter().find_map(|elf_dyn| {
        if elf_dyn.d_tag == DT_INIT { Some(elf_dyn.clone().d_val() as usize) } else { None }
    });
    let elf_dynamic_init_array = elf_dynamic.iter().find_map(|elf_dyn| {
        if elf_dyn.d_tag == DT_INIT_ARRAY { Some(elf_dyn.clone().d_val() as usize) } else { None }
    });
    let elf_dynamic_init_arraysz = elf_dynamic.iter().find_map(|elf_dyn| {
        if elf_dyn.d_tag == DT_INIT_ARRAYSZ { Some(elf_dyn.clone().d_val() as usize) } else { None }
    });
    let elf_dynamic_fini = elf_dynamic.iter().find_map(|elf_dyn| {
        if elf_dyn.d_tag == DT_FINI { Some(elf_dyn.clone().d_val() as usize) } else { None }
    });
    let elf_dynamic_fini_array = elf_dynamic.iter().find_map(|elf_dyn| {
        if elf_dyn.d_tag == DT_FINI_ARRAY { Some(elf_dyn.clone().d_val() as usize) } else { None }
    });
    let elf_dynamic_fini_arraysz = elf_dynamic.iter().find_map(|elf_dyn| {
        if elf_dyn.d_tag == DT_FINI_ARRAYSZ { Some(elf_dyn.clone().d_val() as usize) } else { None }
    });
    let addend_to_unmap_at = |address| elf_segments.iter().find_map(|elf_segment| {
        if (address as u64) >= elf_segment.p_vaddr &&
                (address as u64) < elf_segment.p_vaddr + elf_segment.p_memsz {
            Some(elf_segment.p_offset as isize - elf_segment.p_vaddr as isize)
        } else {
            None
        }
    });
    let mut initializers = Vec::new();
    if let Some(init_func) = elf_dynamic_init { initializers.push(init_func as u64) }
    match (elf_dynamic_init_array, elf_dynamic_init_arraysz) {
        (Some(init_func_array), Some(init_func_array_sz)) => {
            let init_func_array = init_func_array.wrapping_add_signed(addend_to_unmap_at(init_func_array)
                .expect("DT_INIT_ARRAY not part of any segment"));
            let parse = E::from_ei_data(elf_data[EI_DATA]).unwrap();
            let elf_init_funcs = &elf_data[init_func_array..init_func_array + init_func_array_sz];
            let mut offset = 0;
            while offset < init_func_array_sz {
                initializers.push(parse.parse_u64_at(&mut offset, elf_init_funcs).unwrap())
            }
        }
        (None, None) => (),
        _ => panic!("Expected dynamic table to have both or neither of DT_INIT_ARRAY and DT_INIT_ARRAYSZ")
    }
    let mut finalizers = Vec::new();
    match (elf_dynamic_fini_array, elf_dynamic_fini_arraysz) {
        (Some(fini_func_array), Some(fini_func_array_sz)) => {
            let fini_func_array = fini_func_array.wrapping_add_signed(addend_to_unmap_at(fini_func_array)
                .expect("DT_FINI_ARRAY not part of any segment"));
            let parse = E::from_ei_data(elf_data[EI_DATA]).unwrap();
            let elf_fini_funcs = &elf_data[fini_func_array..fini_func_array + fini_func_array_sz];
            let mut offset = 0;
            while offset < fini_func_array_sz {
                finalizers.push(parse.parse_u64_at(&mut offset, elf_fini_funcs).unwrap())
            }
        }
        (None, None) => (),
        _ => panic!("Expected dynamic table to have both or neither of DT_FINI_ARRAY and DT_FINI_ARRAYSZ")
    }
    if let Some(init_func) = elf_dynamic_fini { finalizers.push(init_func as u64) }
    // TODO: DT_SONAME(s) must take priority
    let image_name = soname.map(|name| name.to_owned());
    let interpreter = elf_segments.iter().find_map(|elf_segment| {
        // If PT_INTERP exists, it specifies a path to the external interpreter.
        if elf_segment.p_type == PT_INTERP {
            let path = elf_file.segment_data(&elf_segment).ok()
                .and_then(|data| String::from_utf8(data[..data.len() - 1].to_owned()).ok())
                .expect("Invalid PT_INTERP path");
            Some(Interpreter::External(path))
        } else {
            None
        }
    }).unwrap_or_else(|| {
        if elf_file.ehdr.e_entry != 0 {
            // If PT_INTERP does not exist (and this is an ET_DYN), but there is an entry point, then this object is
            // its own interpreter. Record the values required to invoke it according to the kernel ABI later, once
            // we combine it with something to load.
            Interpreter::Internal {
                // Assume the PIE isn't prelinked to a weird address, which really shouldn't happen; it's a real pain
                // to try and figure out exactly what the base is supposed to be, since it doesn't explicitly appear in
                // any of the ELF structures.
                base: 0,
                entry: elf_file.ehdr.e_entry,
                // musl libc does some hair-raising manipulations with segments; namely, it uses padding around segment
                // data as Free Real Estate™ for its malloc, in both the dynamic linker itself (ld.so) as well as
                // whatever it's loading. While this works fine with normal kernel PT_INTERP logic, ours is pecularly
                // different in that the image of the interpreter overlaps the image of the loadee. As a result, musl's
                // dynamic loader causes its malloc to perform a 'double alloc', which is somehow even more destructive
                // than a double free. To avoid this, we're hiding the interpreter from itself by reducing the amount
                // of program headers available via `auxv[AT_PHNUM]`, which is the ABI-prescribed mechanism for ld.so
                // to find out how many it needs to relocate. The success of this requires ld.so to not look at
                // the actual ELF header for our binary, but since it doesn't even have a pointer to it, this should
                // all work just fine.
                segments: segments.len(),
            }
        } else {
            // Probably just a shared library.
            Interpreter::Absent
        }
    });
    let entry = elf_file.ehdr.e_entry;
    Ok(Image {
        machine,
        alignment,
        segments,
        tls_image,
        symbols,
        relocations,
        initializers,
        finalizers,
        dependencies,
        image_name,
        interpreter,
        entry,
    })
}
