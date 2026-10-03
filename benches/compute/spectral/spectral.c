// spectral-norm N=300, 10 power iterations on AtA.
#include <math.h>
#include <stdio.h>

#define N 300

static double a_elem(int i, int j) {
    long w = (long)(i + j) * (i + j + 1) / 2 + i + 1;
    return 1.0 / (double)w;
}

static void av(double *x, double *y) {
    for (int i = 0; i < N; i++) {
        double s = 0.0;
        for (int j = 0; j < N; j++) s += a_elem(i, j) * x[j];
        y[i] = s;
    }
}

static void atv(double *x, double *y) {
    for (int i = 0; i < N; i++) {
        double s = 0.0;
        for (int j = 0; j < N; j++) s += a_elem(j, i) * x[j];
        y[i] = s;
    }
}

int main(void) {
    static double u[N], v[N], tmp[N];
    for (int i = 0; i < N; i++) { u[i] = 1.0; v[i] = 0.0; tmp[i] = 0.0; }
    for (int k = 0; k < 10; k++) { av(u, tmp); atv(tmp, v); av(v, tmp); atv(tmp, u); }
    double vBv = 0.0, vv = 0.0;
    for (int j = 0; j < N; j++) { vBv += u[j] * v[j]; vv += v[j] * v[j]; }
    printf("RESULT checksum %.16g\n", sqrt(vBv / vv));
    return 0;
}
