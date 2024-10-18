#include <stdio.h>

extern int def_in_dyn;
int def_in_exec = 5678;

void dyn_main();

int main() {
    puts("hello from main()!");
    printf("exec: def_in_exec=%d def_in_dyn=%d\n", def_in_exec, def_in_dyn);
    def_in_dyn = 1;
    def_in_exec = 3;
    printf("exec: def_in_exec=%d def_in_dyn=%d\n", def_in_exec, def_in_dyn);
    dyn_main();
    printf("exec: def_in_exec=%d def_in_dyn=%d\n", def_in_exec, def_in_dyn);
    puts("goodbye from main()!");
}
