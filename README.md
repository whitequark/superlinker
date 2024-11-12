# ｓｕｐｅｒｌｉｎｋｅｒ

Superlinker is a tool that can combine executables and shared libraries into even larger products, just like object files are combined into executables and shared libraries. It works well enough for [building a self-contained Python distribution](#python) from off-the-shelf packages.

## Why?

Wouldn't it be funny if your entire OS image consisted of only one shared object that was *really*, ***really*** large?

## How?

Superlinker is structured essentially like a compiler whose inputs and outputs are interpreted programs (ELF `ET_DYN` PIE executables or shared libraries). Its frontend lifts an ELF `ET_DYN` object into an abstract and simple intermediate representation, and its backend lowers this representation back to an ELF `ET_DYN` object. While memory mappings (`PT_LOAD` segments) are retained essentially intact, none of the ELF headers are copied from the inputs to the outputs. Of the many possible transformations, the currently implemented one rebases and merges several ELF objects. This approach is quite robust.

Additionally, Superlinker is able to merge the dynamic linker itself into an executable, which enables transforming system-dependent executables into executables that run anywhere. (The resulting executable is still an `ET_DYN` object to retain the benefits of ASLR, but it has no load-time dependencies.) This is implemented with an executable shim that emulates the kernel ABI for `PT_INTERP` loaded objects, and so is not tied to a specific libc, but currently only tested with [musl libc][].

The intermediate representation features architecture-, target-, and (somewhat) format-independent model of loadable segments, relocations, symbols, and image interpreters, biased towards ELF without directly requiring it. The frontend and backend are currently ported to `amd64` only. Although not strictly required for functioning, section headers are emitted as a courtesy for `libbfd` based tools (e.g. `objdump`).

[musl libc]: https://musl-libc.org

## Use?

First, install [Rust][] and run `cargo build`.

```
Usage: ./target/debug/superlinker <output.elf> <target.elf> [<source1.elf> [<source2.elf> ...]]
```

[rust]: https://rust-lang.org/

## Show?

```
$ make -C data # prepare test files
$ ./data/test_exec.elf
Error loading shared library libtest_dyn.so: No such file or directory (needed by ./data/test_exec.elf)
Error relocating ./data/test_exec.elf: dyn_main: symbol not found
$ readelf -d ./data/test_exec.elf

Dynamic section at offset 0x2e00 contains 25 entries:
  Tag        Type                         Name/Value
 0x0000000000000001 (NEEDED)             Shared library: [libtest_dyn.so]
 0x0000000000000001 (NEEDED)             Shared library: [libc.so]
 0x000000000000000c (INIT)               0x1000
...
$ ./target/debug/superlinker merged.elf data/test_exec.elf data/libtest_dyn.so /lib/x86_64-linux-musl/libc.so
merge_into: merging source image "libtest_dyn.so" into target image "test_exec.elf"
merge_into: rebasing source image by +0x5000
merge_into: ignoring source special symbol _init
merge_into: using source global symbol dyn_main to resolve target import
merge_into: ignoring source special symbol _fini
merge_into: removing extinguished dependency "libtest_dyn.so"
merge_into: merging source image "libc.so" into target image "test_exec.elf"
merge_into: rebasing source image by +0xa000
merge_into: using source global symbol puts to resolve target import
merge_into: forcing target special symbol _init to come from libc
merge_into: forcing target special symbol _fini to come from libc
merge_into: using source global symbol __cxa_finalize to resolve target missing weak symbol
merge_into: using source global symbol __libc_start_main to resolve target import
merge_into: removing extinguished dependency "libc.so"
merge_into: embedding the source image into target object as its interpreter
$ ./merged.elf
hello from main()!
hello from dyn_main()!
$ readelf -d ./merged.elf

Dynamic section at offset 0x2000 contains 9 entries:
  Tag        Type                         Name/Value
 0x0000000000000005 (STRTAB)             0x20a0
 0x000000000000000a (STRSZ)              15767 (bytes)
 0x000000000000000b (SYMENT)             24 (bytes)
 0x0000000000000006 (SYMTAB)             0x5e38
 0x0000000000000004 (HASH)               0xfe70
 0x0000000000000007 (RELA)               0x11940
 0x0000000000000008 (RELASZ)             2640 (bytes)
 0x0000000000000009 (RELAENT)            24 (bytes)
 0x0000000000000000 (NULL)               0x0
```

## Flaws?

Although the core approach is sound, this implementation has flaws, most of which are fixable:

- All of the code continues to use the dynamic linking ABI, i.e. procedure calls go through PLT and global accesses go through GOT. This is the only flaw inherent to the approach.
- Executable and shared object formats are notoriously complex and this implementation is bound to have bugs.
    - Moreover, some of the more obscure features are not implemented rigorously or at all (e.g. symbol scoping, visibility, and versioning).
- All GOT and PLT optimizations are disabled. (This means that `DT_JMPREL`, `DT_PLTREL`, and `DT_PLTRELSZ` entries are stripped.)
    - PLT optimizations at least could be added back with additional work.
- Only the `global-dynamic` TLS model is supported.
- Only "Rela" relocations are implemented and tested, though "Rel" relocations would be trivial to add.
- `DT_GNU_HASH` is not supported, and the number of `DT_HASH` buckets is randomly fixed at 4.
- Although ASLR is supported (Superlinker only produces position independent executables), `PT_GNU_STACK` and `PT_GNU_RELRO` are not supported and stripped.
- Exception handling currently isn't supported, and `PT_GNU_EH_FRAME` is stripped.
- Some of the internal book-keeping probably has O(n²) complexity.

The implementation is less than a thousand lines long, written with portability in mind, and extensively commented, so it should not be too difficult to address most of these flaws. It should even run on Windows!

## Python?

Although tedious, it is possible to use Superlinker to build a fully self-contained Python distribution without source modifications or, in fact, touching source at all. First, link the combination of the Python executable, its dependencies, and essential modules. Using Alpine Linux 3.20 as the base distribution, run:

```
# apk add python3
$ ./superlinker py.elf /usr/bin/python3.12 /usr/lib/libpython3.12.so.1.0 \
    /usr/lib/python3.12/lib-dynload/math.cpython-312-x86_64-linux-musl.so \
    /usr/lib/python3.12/lib-dynload/binascii.cpython-312-x86_64-linux-musl.so \
    /usr/lib/python3.12/lib-dynload/zlib.cpython-312-x86_64-linux-musl.so \
    /usr/lib/python3.12/lib-dynload/array.cpython-312-x86_64-linux-musl.so \
    /usr/lib/python3.12/lib-dynload/_struct.cpython-312-x86_64-linux-musl.so \
    /usr/lib/python3.12/lib-dynload/_ctypes.cpython-312-x86_64-linux-musl.so \
    /usr/lib/python3.12/lib-dynload/readline.cpython-312-x86_64-linux-musl.so \
    /usr/lib/libreadline.so.8 /usr/lib/libncursesw.so.6 /usr/lib/libffi.so.8 \
    /lib/libz.so.1 /lib/ld-musl-x86_64.so.1
```

Python has a little known function where it can [treat a zip archive as if it was a directory][zipimport], which will come in handy when packaging the (portable subset of) standard library modules:

```
# apk add fastjar
$ fastjar 0cvf py.zip -C /usr/lib/python3.12/ .
```

Note the `0` (that's a zero) option for `fastjar`; Python loads compressed zip archives using its own `zipimport` standard library module, which means that it cannot be compressed when it is a part of a zip archive itself.

Even though Python has all of these modules linked into it, it's currently unaware of that, and an attempt to import any of them will fail. This can be solved with a little bit of Python code:

```
$ cat >sitecustomize.py <<END
import sys, importlib.machinery, importlib.util

class SoulSearchingMetaPathFinder:
    @staticmethod
    def find_spec(name, path, target=None):
        if path is None:
            spec = importlib.util.spec_from_loader(name,
                importlib.machinery.ExtensionFileLoader(name, sys.executable))
            try:
                spec.loader.create_module(spec)
                return spec
            except ImportError:
                return None

sys.meta_path.append(SoulSearchingMetaPathFinder)
END
$ fastjar 0uvf py.zip sitecustomize.py
```

The Python executable and the Python standard library are now ready to spend an eternity together:

```
$ cat py.elf py.zip >py.run
$ chmod +x py.run
```

The final touch this distribution needs is the `PYTHONPATH` environment variable:

```
$ PYTHONPATH=$(pwd)/py.run ./py.run
Could not find platform independent libraries <prefix>
Could not find platform dependent libraries <exec_prefix>
Python 3.12.7 (main, Oct  7 2024, 11:30:19) [GCC 13.2.1 20240309] on linux
Type "help", "copyright", "credits" or "license" for more information.
>>> import sys, zipfile
>>> print([zi.filename for zi in zipfile.ZipFile(sys.executable).filelist][:10])
['META-INF/', 'META-INF/MANIFEST.MF', './', '_collections_abc.py', 'socket.py', '__pycache__/', '__pycache__/heapq.cpython-312.pyc', '__pycache__/codecs.cpython-312.pyc', '__pycache__/shutil.cpython-312.pyc', '__pycache__/ssl.cpython-312.pyc']
>>> import zlib
>>> zlib.crc32(b"spam")
1138425661
```

[zipimport]: https://docs.python.org/3/library/zipimport.html

## Past?

If you like Superlinker, you might also enjoy [unfork][].

[unfork]: https://github.com/whitequark/unfork

## License?

[0-clause BSD](LICENSE-0BSD.txt).
