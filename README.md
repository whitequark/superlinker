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
```

TODO: actual readme
