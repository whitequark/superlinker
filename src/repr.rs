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
    // ... to be continued?
}

#[derive(Debug, Clone)]
pub struct Relocation {
    pub offset: u64,
    pub target: RelocationTarget,
}

#[derive(Debug, Clone)]
pub struct Image {
    pub machine: u16, // ELF machine
    pub alignment: u64, // integer that is a power of 2
    pub segments: Vec<LoadSegment>, // sorted in ascending order
    pub symbols: Vec<Symbol>,
    pub relocations: Vec<Relocation>,
    pub dependencies: Vec<String>, // requests images by name
    pub image_name: Option<String>, // requested via dependencies
    pub entry: u64,
}

impl Image {
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
                RelocationTarget::Symbol { .. } => (),
                RelocationTarget::Base { ref mut addend } =>
                    *addend += offset as i64,
            }
        }
        self.entry += offset;
    }

    pub fn merge_into(mut self, target: &mut Image) {
        // Check that the two images can be merged.
        assert!(self.machine == target.machine);
        assert!(self.alignment == target.alignment);
        // Relocate this image to be fully above the target.
        let (_target_begin, target_end) = target.segment_bounds();
        eprintln!("merge_into: rebasing source image by +{:#x}", target_end);
        self.rebase(target_end);
        // Merge this image's segments.
        target.segments.append(&mut self.segments);
        // Index the target image's symbol table.
        let mut target_symbol_map = HashMap::new();
        for (symbol_index, symbol) in target.symbols.iter().enumerate() {
            if target_symbol_map.insert(symbol.name.clone(), symbol_index).is_some() {
                panic!("Duplicate symbol {} in target image", symbol.name.as_str());
            }
        }
        // Merge symbols.
        for source_symbol in self.symbols.into_iter() {
            let symbol_name = source_symbol.name.to_owned();
            let target_symbol = target_symbol_map.get(&symbol_name).map(|index| &mut target.symbols[*index]);
            match (source_symbol, target_symbol) {
                (source_symbol, None) => {
                    eprintln!("merge_into: adding new symbol {}", &symbol_name);
                    target.symbols.push(source_symbol)
                }
                (source_symbol @ Symbol { scope: SymbolScope::Global, .. },
                 Some(target_symbol @ &mut Symbol { scope: SymbolScope::Import, .. })) => {
                    eprintln!("merge_into: using global symbol {} to resolve import", &symbol_name);
                    target_symbol.scope = source_symbol.scope;
                    target_symbol.kind = source_symbol.kind;
                    target_symbol.value = source_symbol.value;
                },
                (source_symbol @ Symbol { scope: SymbolScope::Global, .. },
                 Some(target_symbol @ &mut Symbol { scope: SymbolScope::Weak, value: 0, .. })) => {
                    eprintln!("merge_into: using global symbol {} to resolve missing weak symbol", &symbol_name);
                    target_symbol.scope = source_symbol.scope;
                    target_symbol.kind = source_symbol.kind;
                    target_symbol.value = source_symbol.value;
                },
                (source_symbol, Some(target_symbol @ &mut Symbol { .. })) if symbol_name == "_init" || symbol_name == "_fini" => {
                    if self.image_name.as_deref() == Some("libc.so") {
                        eprintln!("merge_into: forcing special symbol {} to come from libc", &symbol_name);
                        target_symbol.scope = SymbolScope::Global;
                        target_symbol.kind = source_symbol.kind;
                        target_symbol.value = source_symbol.value;
                    } else {
                        eprintln!("merge_into: ignoring special symbol {}", &symbol_name)
                    }
                }
                (source_symbol, Some(target_symbol)) if &source_symbol == target_symbol => (),
                (source_symbol, Some(target_symbol)) => {
                    panic!("Cannot merge source symbol {:?} into target symbol {:?}",
                        source_symbol, target_symbol)
                }
            }
        }
        // Merge relocations. Relocations can never be removed, even if they refer to the self.
        target.relocations.append(&mut self.relocations);
        // Merge dependencies.
        let mut target_dependency_set = HashSet::new();
        for target_dependency in target.dependencies.iter() {
            target_dependency_set.insert(target_dependency.clone());
        }
        for source_dependency in self.dependencies.into_iter() {
            if target_dependency_set.insert(source_dependency.clone()) {
                eprintln!("merge_into: adding new dependency {:?}", source_dependency);
            }
        }
        if let Some(source_image_name) = self.image_name.as_ref() {
            if target_dependency_set.remove(source_image_name) {
                eprintln!("merge_into: removing extinguished dependency {:?}", &source_image_name);
            }
        }
        target.dependencies = target_dependency_set.into_iter().collect::<Vec<_>>();
    }
}
