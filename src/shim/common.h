#define AT_BASE     0x7
#define AT_ENTRY    0x9
#define AT_PHNUM    0x5

// Keep in sync with src/emit.rs crate::emit::make_shim
#define USER_ENTRY_REL      (__SIZEOF_POINTER__ * 0)
#define INTERP_ENTRY_REL    (__SIZEOF_POINTER__ * 1)
#define INTERP_BASE_REL     (__SIZEOF_POINTER__ * 2)
#define INTERP_PHNUM        (__SIZEOF_POINTER__ * 3)
