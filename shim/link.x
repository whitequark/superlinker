ENTRY(_start)

PHDRS {
    load PT_LOAD;
    dynamic PT_DYNAMIC;
}

SECTIONS {
    shim_base = .;
    .text : {
        *(.text.entry)
        *(.text .text.*)
        *(.rodata .rodata.*)
        . = ALIGN(8);
    } : load
    shim_data = .;
    .dynamic : { *(.dynamic) } : dynamic
    /DISCARD/ : { *(.dynsym .gnu.hash .hash .dynstr) }
}
