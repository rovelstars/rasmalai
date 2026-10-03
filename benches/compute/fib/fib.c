// fib(35): naive double recursion. Prints "RESULT checksum <n>".
#include <stdio.h>

static long fib(long n) {
    if (n < 2) return n;
    return fib(n - 1) + fib(n - 2);
}

int main(void) {
    printf("RESULT checksum %ld\n", fib(35));
    return 0;
}
