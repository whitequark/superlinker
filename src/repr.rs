use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadMode {
    ReadOnly,
    ReadWrite,
    ReadExecute,
}

#[derive(Debug, Clone)]
pub struct LoadSegment {
    pub addr: u64, // virtual address, relative to object base
    pub size: u64, // size in virtual memory
    pub data: Vec<u8>, // data to load at [addr..addr+size); can be smaller than size in virtual memory
    pub mode: LoadMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    Code,
    Data,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolScope {
    Local,
    Global,
    Import,
    Weak,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    pub scope: SymbolScope,
    pub value: u64,
    pub size: u64,
    pub abs: bool,
}

#[derive(Debug, Clone)]
pub enum RelocationTarget {
    // R_X86_64_64
    // R_X86_64_GLOB_DAT
    // R_X86_64_JUMP_SLOT
    // = S + A
    Symbol { symbol: String, addend: i64 },
    // R_X86_64_RELATIVE
    // = B + A
    Base { addend: i64 },
    // R_X86_64_COPY
    Copy { symbol: String },
    // R_X86_64_NONE
    None,
    // ... to be continued?

    ElfSpecific(u32), // any that doesn't need and/or can't be portably processed
}

#[derive(Debug, Clone)]
pub struct Relocation {
    pub offset: u64,
    pub target: RelocationTarget,
}

#[derive(Debug, Clone)]
pub enum Interpreter {
    Absent,
    External(String),
    Internal { base: u64, entry: u64, segments: usize },
}

#[derive(Debug, Clone)]
pub struct Image {
    pub machine: u16, // ELF machine
    pub alignment: u64, // integer that is a power of 2
    pub segments: Vec<LoadSegment>, // sorted in ascending order
    pub tls_image: Option<Vec<u8>>,
    pub symbols: Vec<Symbol>,
    pub relocations: Vec<Relocation>,
    pub initializers: Vec<u64>,
    pub finalizers: Vec<u64>,
    pub dependencies: Vec<String>, // requests images by name
    pub image_names: Vec<String>, // requested via dependencies
    pub interpreter: Interpreter,
    pub entry: u64,
}

impl Image {
    pub fn display_image_name(&self) -> &str {
        self.image_names.first().map(|name| &name[..]).unwrap_or("<unnamed>")
    }

    pub fn segment_bounds(&self) -> (u64, u64) {
        match (self.segments.first(), self.segments.last()) {
            (Some(first), Some(last)) =>
                (first.addr, ((last.addr + last.size - 1) | (self.alignment - 1)) + 1),
            _ => (0, 0)
        }
    }

    pub fn rebase(&mut self, offset: u64) {
        assert!(offset % self.alignment == 0, "Rebase offset must be aligned");
        for segment in self.segments.iter_mut() {
            segment.addr += offset;
        }
        for symbol in self.symbols.iter_mut() {
            // The intermediate representation currently doesn't include absolute symbols.
            if symbol.value != 0 {
                symbol.value += offset;
            }
        }
        for relocation in self.relocations.iter_mut() {
            relocation.offset += offset;
            match relocation.target {
                RelocationTarget::Base { ref mut addend } =>
                    *addend += offset as i64,
                RelocationTarget::Symbol { .. } |
                RelocationTarget::Copy { .. } |
                RelocationTarget::None |
                RelocationTarget::ElfSpecific(_) => ()
            }
        }
        for initializer in self.initializers.iter_mut() {
            *initializer += offset;
        }
        for finalizer in self.finalizers.iter_mut() {
            *finalizer += offset;
        }
        match self.interpreter {
            Interpreter::Absent | Interpreter::External(_) => (),
            Interpreter::Internal { ref mut base, ref mut entry, .. } => {
                *base += offset;
                *entry += offset;
            },
        }
        self.entry += offset;
    }

    pub fn merge_into(mut self, target: &mut Image) {
        // Check that the two images can be merged.
        assert!(self.machine == target.machine);
        assert!(self.alignment == target.alignment);
        eprintln!("merge_into: merging source image {} into target image {}",
            self.display_image_name(), target.display_image_name());
        // Relocate this image to be fully above the target.
        let (_target_begin, target_end) = target.segment_bounds();
        eprintln!("merge_into: rebasing source image by +{:#x}", target_end);
        self.rebase(target_end);
        // Merge this image's segments.
        target.segments.append(&mut self.segments);
        if self.tls_image.is_some() {
            if target.tls_image.is_none() {
                target.tls_image = self.tls_image.take();
            } else {
                panic!("Merging TLS images is not implemented");
            }
        }
        match (&self.interpreter, &mut target.interpreter) {
            (Interpreter::Absent, Interpreter::Absent) |
            (Interpreter::Absent, Interpreter::External(..)) => {
                // Merging executable + library or library + library
                self.merge_dynamic(target);
            }
            (source_interpreter @ Interpreter::Internal { .. },
             target_interpreter @ Interpreter::External(_)) => {
                // Merging interpreter + executable
                eprintln!("merge_into: embedding the source image into target object as its interpreter");
                *target_interpreter = source_interpreter.clone();
            }
            (source_interpreter, target_interpreter) =>
                panic!("Cannot merge source object with interpreter {:?} into target object with interpreter {:?}",
                    source_interpreter, target_interpreter)
        }
    }

    fn merge_dynamic(mut self, target: &mut Image) {
        // Index the target image's symbol table.
        let mut target_symbol_map = HashMap::new();
        for (symbol_index, symbol) in target.symbols.iter().enumerate() {
            if target_symbol_map.insert(symbol.name.clone(), symbol_index).is_some() {
                panic!("Duplicate symbol {:?} in target image", symbol.name.as_str());
            }
        }
        // Merge symbols.
        let mut apply_copy_relocs_later = Vec::new();
        for source_symbol in self.symbols.into_iter() {
            let symbol_name = source_symbol.name.to_owned();
            let target_symbol = target_symbol_map.get(&symbol_name).map(|index| &mut target.symbols[*index]);
            match (source_symbol, target_symbol) {
                (source_symbol, None) => {
                    // eprintln!("merge_into: adding new symbol {:?}", &symbol_name);
                    target_symbol_map.insert(symbol_name.clone(), target.symbols.len());
                    target.symbols.push(source_symbol);
                }
                (_source_symbol @ Symbol { scope: SymbolScope::Weak, value: 0, .. },
                 Some(_target_symbol @ &mut Symbol { scope: SymbolScope::Weak, value: 0, .. })) => (),
                (_source_symbol @ Symbol { scope: SymbolScope::Weak, value: 0, .. },
                 Some(_target_symbol @ &mut Symbol { scope: SymbolScope::Weak, .. })) => {
                    eprintln!("merge_into: replacing source weak symbol {:?} with target weak symbol", &symbol_name);
                }
                (source_symbol @ Symbol { scope: SymbolScope::Weak, .. },
                 Some(target_symbol @ &mut Symbol { scope: SymbolScope::Weak, value: 0, .. })) => {
                    eprintln!("merge_into: using source weak symbol {:?} to resolve target missing weak symbol", &symbol_name);
                    target_symbol.scope = source_symbol.scope;
                    target_symbol.kind = source_symbol.kind;
                    target_symbol.value = source_symbol.value;
                }
                (source_symbol @ Symbol { scope: SymbolScope::Weak, .. },
                 Some(target_symbol @ &mut Symbol { scope: SymbolScope::Weak, .. })) => {
                    eprintln!("merge_into: using source weak symbol {:?} to resolve target missing weak symbol", &symbol_name);
                    target_symbol.scope = source_symbol.scope;
                    target_symbol.kind = source_symbol.kind;
                    target_symbol.value = source_symbol.value;
                }
                (source_symbol @ Symbol { scope: SymbolScope::Global | SymbolScope::Weak, .. },
                 Some(target_symbol @ &mut Symbol { scope: SymbolScope::Import, .. })) => {
                    eprintln!("merge_into: using source symbol {:?} to resolve target import", &symbol_name);
                    target_symbol.scope = source_symbol.scope;
                    target_symbol.kind = source_symbol.kind;
                    target_symbol.value = source_symbol.value;
                },
                (_source_symbol @ Symbol { scope: SymbolScope::Import, .. },
                 Some(_target_symbol @ &mut Symbol { scope: SymbolScope::Global | SymbolScope::Weak, .. })) => {
                    eprintln!("merge_into: using target symbol {:?} to resolve source import", &symbol_name);
                },
                (source_symbol @ Symbol { scope: SymbolScope::Global, .. },
                 Some(target_symbol @ &mut Symbol { scope: SymbolScope::Weak, value: 0, .. })) => {
                    eprintln!("merge_into: using source global symbol {:?} to resolve target missing weak symbol", &symbol_name);
                    target_symbol.scope = source_symbol.scope;
                    target_symbol.kind = source_symbol.kind;
                    target_symbol.value = source_symbol.value;
                },
                (Symbol { scope: SymbolScope::Weak, value: 0, .. },
                 Some(&mut Symbol { scope: SymbolScope::Global, .. })) => {
                    eprintln!("merge_into: using target global symbol {:?} to resolve source missing weak symbol", &symbol_name);
                },
                (source_symbol, Some(target_symbol @ &mut Symbol { .. }))
                        if symbol_name == "_init" || symbol_name == "_fini" => {
                    if self.image_names.iter().find(|name| **name == "libc.so").is_some() {
                        eprintln!("merge_into: forcing target special symbol {:?} to come from libc", &symbol_name);
                        target_symbol.scope = SymbolScope::Global;
                        target_symbol.kind = source_symbol.kind;
                        target_symbol.value = source_symbol.value;
                    } else {
                        eprintln!("merge_into: ignoring source special symbol {:?}", &symbol_name)
                    }
                }
                (source_symbol @ Symbol { scope: SymbolScope::Global, kind: SymbolKind::Data, .. },
                 Some(target_symbol @ &mut Symbol { scope: SymbolScope::Global, kind: SymbolKind::Data, .. }))
                        if source_symbol.size == target_symbol.size => {
                    eprintln!("merge_into: replacing source global data symbol {:?} with the same target global data symbol", &symbol_name);
                    for (reloc_index, reloc) in target.relocations.iter().enumerate() {
                        if let Relocation { target: RelocationTarget::Copy { symbol: copy_symbol_name }, .. } = &reloc {
                            if symbol_name == *copy_symbol_name {
                                apply_copy_relocs_later.push((reloc_index, source_symbol.clone()));
                            }
                        }
                    }
                },
                (source_symbol, Some(target_symbol)) if &source_symbol == target_symbol => (),
                (source_symbol, Some(target_symbol)) => {
                    panic!("Cannot merge source symbol {:?} into target symbol {:?}",
                        source_symbol, target_symbol)
                }
            }
        }
        // Apply copy relocations, if any were triggered.
        for (reloc_index, source_symbol) in apply_copy_relocs_later.into_iter() {
            let target_reloc = &mut target.relocations[reloc_index];
            eprintln!("merge_into: applying copy relocation for symbol {:?}: copying {:#x}{:+#x} => {:#x}",
                &source_symbol.name, source_symbol.value, source_symbol.size, target_reloc.offset);
            let source_data = target.segments.iter().find_map(|segment| {
                if source_symbol.value >= segment.addr &&
                        source_symbol.value + source_symbol.size <= segment.addr + segment.size {
                    let range_begin = (source_symbol.value - segment.addr) as usize;
                    let range_end = (source_symbol.value - segment.addr + source_symbol.size) as usize;
                    if let Some(data) = segment.data.get(range_begin..range_end) {
                        Some(data.to_owned())
                    } else {
                        Some(vec![0; source_symbol.size as usize])
                    }
                } else {
                    None
                }
            }).expect("Failed to find source segment for copy relocation");
            for segment in target.segments.iter_mut() {
                if target_reloc.offset >= segment.addr &&
                        target_reloc.offset + source_symbol.size <= segment.addr + segment.size {
                    let range_begin = (target_reloc.offset - segment.addr) as usize;
                    let range_end = (target_reloc.offset - segment.addr + source_symbol.size) as usize;
                    if segment.data.len() < range_end {
                        segment.data.resize(range_end, 0);
                    }
                    segment.data.get_mut(range_begin..range_end)
                        .expect("Failed to slice target data for copy relocation")
                        .copy_from_slice(&source_data);
                }
            }
            target_reloc.target = RelocationTarget::None;
        }
        // Merge relocations. Relocations can never be removed, even if they refer to the self.
        target.relocations.append(&mut self.relocations);
        // Merge initializers and finalizers.
        target.initializers.append(&mut self.initializers);
        // Merge dependencies.
        let mut target_dependency_set = HashSet::new();
        for target_dependency in target.dependencies.iter() {
            target_dependency_set.insert(target_dependency.clone());
        }
        for source_dependency in self.dependencies.into_iter() {
            if target.image_names.iter().find(|&image_name| *image_name == source_dependency).is_some() { continue }
            if target_dependency_set.insert(source_dependency.clone()) {
                eprintln!("merge_into: adding new dependency {:?}", source_dependency);
            }
        }
        for source_image_name in self.image_names.iter() {
            if target_dependency_set.remove(source_image_name) {
                eprintln!("merge_into: removing extinguished dependency {:?}", &source_image_name);
            }
        }
        target.dependencies = target_dependency_set.into_iter().collect::<Vec<_>>();
        // Merge image names.
        target.image_names.append(&mut self.image_names);
    }
}
