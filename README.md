# ｓｕｐｅｒｌｉｎｋｅｒ

```
$ ./data/test_exec.elf
Error loading shared library libtest_dyn.so: No such file or directory (needed by ./data/test_exec.elf)
Error relocating ./data/test_exec.elf: dyn_main: symbol not found
$ ./target/debug/superlinker merged.elf ./data/test_exec.elf ./data/libtest_dyn.so
merge_into: rebasing source image by +0x5000
merge_into: ignoring special symbol _init
merge_into: using global symbol dyn_main to resolve import
merge_into: ignoring special symbol _fini
merge_into: removing extinguished dependency "libtest_dyn.so"
$ ./merged.elf
hello from main()!
hello from dyn_main()!
$ ./target/debug/superlinker merged2.elf merged.elf .../path/to/patched/libc.so
merge_into: rebasing source image by +0xc000
merge_into: adding new symbol y0f
...
merge_into: adding new symbol err
merge_into: adding new symbol recvmsg
merge_into: removing extinguished dependency "libc.so"
$ ./merged2.elf
hello from main()!
hello from dyn_main()!
$ readelf merged2.elf -d

Dynamic section at offset 0x1000 contains 9 entries:
  Tag        Type                         Name/Value
 0x0000000000000005 (STRTAB)             0x10a0
 0x000000000000000a (STRSZ)              15741 (bytes)
 0x000000000000000b (SYMENT)             24 (bytes)
 0x0000000000000006 (SYMTAB)             0x4e20
 0x0000000000000004 (HASH)               0xee40
 0x0000000000000007 (RELA)               0x10908
 0x0000000000000008 (RELASZ)             2640 (bytes)
 0x0000000000000009 (RELAENT)            24 (bytes)
 0x0000000000000000 (NULL)               0x0
```

TODO: actual readme
