MAKEFLAGS += --no-builtin-rules --no-builtin-variables

suffix =

AS = clang --target=$*-none-elf -c
OBJCOPY = rust-objcopy
OBJDUMP = rust-objdump

targets := $(patsubst %.S,%,$(wildcard *.S))

.PHONY: all
all: $(targets:%=%$(suffix).bin) $(targets:%=%$(suffix).o)

%$(suffix).o: %.S common.h
	$(AS) -o $@ $<

%.bin: %.o
	$(OBJCOPY) --output-target binary $< $@

.PHONY: check
check:
	make all suffix=.chk
	@set -e ; \
		for t in $(targets); do \
		echo check $$t.bin is reproducible ; \
		cmp $$t.bin $$t.chk.bin ; \
		echo check $$t.chk.o is relocation-less ; \
		! rust-objdump -r x86_64.o | grep 'R_' ; \
		done

.PHONY: clean
clean:
	rm -f *.chk.* *.o
