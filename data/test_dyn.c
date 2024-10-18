#include <stdio.h>

int def_in_dyn = 1234;
extern int def_in_exec;

void dyn_main() {
    puts("hello from dyn_main()!");
    printf("dyn: def_in_exec=%d def_in_dyn=%d\n", def_in_exec, def_in_dyn);
    def_in_dyn = 2;
    def_in_exec = 4;
    printf("dyn: def_in_exec=%d def_in_dyn=%d\n", def_in_exec, def_in_dyn);
    puts("goodbye from dyn_main()!");
}
