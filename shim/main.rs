#![feature(auto_traits)]
#![feature(decl_macro)]
#![feature(intrinsics)]
#![feature(lang_items)]
#![feature(no_core)]
#![feature(rustc_attrs)]
#![allow(internal_features)]
#![no_core]
#![no_std]
#![no_main]
#![no_builtins]

#[repr(C)]
struct ShimData {
    user_entry_rel: usize,
    interp_entry_rel: usize,
    interp_base_rel: usize,
    interp_phnum: usize,
}

#[repr(C)]
struct Auxv {
    tag: usize,
    value: usize,
}

const AT_BASE: usize = 0x7;
const AT_ENTRY: usize = 0x9;
const AT_PHNUM: usize = 0x5;

unsafe fn find_auxv(mut stack: *mut usize) -> *mut Auxv {
    let argc = *stack;

    // Skip argc, arguments, terminating NULL
    stack = offset(stack, wrapping_add(argc, 2));

    // Skip environment
    while *stack != 0 {
        stack = offset(stack, 1);
    }

    // Skip terminating NULL
    stack = offset(stack, 1);

    stack as _
}

#[no_mangle] // Make disassembly slightly easier to read
#[deny(dead_code, reason = "If you see this it means the architecture is unsupported")]
unsafe extern "C" fn shim_main(stack: *mut usize, data: &ShimData, base: usize) -> usize {
    let mut auxv = find_auxv(stack);

    loop {
        let a: &mut Auxv = &mut *auxv;
        auxv = offset(auxv, 1);

        match a.tag {
            0 => break,
            AT_BASE => a.value = wrapping_add(data.interp_base_rel, base),
            AT_ENTRY => a.value = wrapping_add(data.user_entry_rel, base),
            AT_PHNUM => a.value = data.interp_phnum,
            _ => {}
        }
    }

    wrapping_add(data.interp_entry_rel, base)
}

#[cfg(target_arch = "x86_64")]
global_asm!(
    r#"
    .pushsection .text.entry

    .global _start
_start:
    mov rbp, rsp // Save initial stack pointer

    // Args for shim_main
    mov rdi, rsp
    lea rsi, [rip + shim_data]
    lea rdx, [rip + shim_base]

    and rsp, -16 // Align stack for function call
    call {shim_main}

    mov rsp, rbp // Restore stack
    jmp rax // Jump to interp_entry

    .popsection
    "#,
    shim_main = sym shim_main
);

// >>> Here be dragons <<<

mod intrinsic {
    use super::*;

    extern "rust-intrinsic" {
        #[rustc_safe_intrinsic]
        #[rustc_nounwind]
        pub(super) fn wrapping_add<T: Copy>(a: T, b: T) -> T;

        #[rustc_nounwind]
        pub(super) fn offset<Ptr, Delta>(dst: Ptr, offset: Delta) -> Ptr;
    }
}

unsafe fn offset<T>(dst: *mut T, offset: usize) -> *mut T {
    intrinsic::offset(dst, offset)
}

fn wrapping_add(a: usize, b: usize) -> usize {
    intrinsic::wrapping_add(a, b)
}

#[rustc_builtin_macro]
pub macro global_asm() {}

#[lang = "sized"]
trait Sized {}

#[lang = "receiver"]
trait Receiver {}
impl<T: ?Sized> Receiver for &T {}

#[lang = "freeze"]
auto trait Freeze {}

#[lang = "copy"]
trait Copy {}
impl<T> Copy for *mut T {}
impl Copy for usize {}
impl Copy for bool {}

#[allow(dead_code)] // Spurious
#[lang = "eq"]
trait PartialEq<Rhs: ?Sized = Self> {
    fn eq(&self, other: &Rhs) -> bool;
    fn ne(&self, other: &Rhs) -> bool;
}

impl PartialEq for usize {
    fn eq(&self, other: &usize) -> bool {
        *self == *other
    }
    fn ne(&self, other: &usize) -> bool {
        *self != *other
    }
}
