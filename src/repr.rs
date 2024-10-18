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

#[derive(Debug, Clone)]
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
    pub segments: Vec<LoadSegment>,
    pub symbols: Vec<Symbol>,
    pub needed: Vec<String>,
    pub relocations: Vec<Relocation>,
    pub entry: u64,
}
