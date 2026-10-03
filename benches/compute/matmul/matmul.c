// matmul 256x256 integer matrices, ijk loop order.
#include <stdio.h>

#define N 256

static long a[N * N], b[N * N], c[N * N];

int main(void) {
    for (int r = 0; r < N; r++)
        for (int k = 0; k < N; k++) {
            a[r * N + k] = (r + k) % 64;
            b[r * N + k] = (r * k) % 64;
        }
    for (int x = 0; x < N; x++)
        for (int y = 0; y < N; y++) {
            long s = 0;
            for (int z = 0; z < N; z++) s += a[x * N + z] * b[z * N + y];
            c[x * N + y] = s;
        }
    long total = 0;
    for (int t = 0; t < N * N; t++) total += c[t];
    printf("RESULT checksum %ld\n", total);
    return 0;
}
