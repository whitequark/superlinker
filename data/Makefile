test_exec.elf: test_exec.c libtest_dyn.so
	musl-gcc -fPIC -fPIE -o $@ $^
libtest_dyn.so: test_dyn.c
	musl-gcc -fPIC -shared -o $@ $^
clean:
	rm -f *.elf *.so
