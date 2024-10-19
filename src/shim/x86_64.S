#include "common.h"

    .intel_syntax noprefix

    .section .text
    .balign 8
start:
    mov rax, [rsp] // rax = argc
    lea rax, [rsp + 8 * rax + 16] // rax = start of envp

    // Skip environment
1:
    add rax, 8
    cmp dword ptr [rax - 8], 0
    jne 1b

    // rax = start of auxv

    lea rcx, [rip + start] // rcx = shim_base

    // Patch auxv
1:
    mov rdx, [rax]
    cmp rdx, 0 // End of auxv?
    je 3f

    // Set auxv AT_BASE to interp_base_rel + shim_base
    cmp rdx, AT_BASE
    jne 2f
    mov rdi, [rip + end + INTERP_BASE_REL]
    add rdi, rcx
    mov [rax + 8], rdi
2:

    // Set auxv AT_ENTRY to user_entry + shim_base
    cmp rdx, AT_ENTRY
    jne 2f
    mov rdi, [rip + end + USER_ENTRY_REL]
    add rdi, rcx
    mov [rax + 8], rdi
2:

    // Set auxv AT_PHNUM to interp_phnum
    cmp rdx, AT_PHNUM
    jne 2f
    mov rdi, [rip + end + INTERP_PHNUM]
    // Don't add shim_base
    mov [rax + 8], rdi
2:

    add rax, 16 // Check next auxv
    jmp 1b

3:
    // End patch auxv

    // Jump to interp_entry + shim_base
    mov rax, [rip + end + INTERP_ENTRY_REL]
    add rax, rcx
    jmp rax

    .balign 8
end:
    // Data part begins here
