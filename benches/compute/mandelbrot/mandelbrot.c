// mandelbrot 400x400, escape iterations capped at 1000.
#include <stdio.h>

int main(void) {
    long total = 0;
    for (int py = 0; py < 400; py++) {
        double y0 = (py * 3.0 / 399.0) - 1.5;
        for (int px = 0; px < 400; px++) {
            double x0 = (px * 3.0 / 399.0) - 2.0;
            double zx = 0.0, zy = 0.0;
            int iter = 0;
            while (iter < 1000) {
                double zx2 = zx * zx;
                double zy2 = zy * zy;
                if (zx2 + zy2 > 4.0) break;
                zy = 2.0 * zx * zy + y0;
                zx = zx2 - zy2 + x0;
                iter++;
            }
            total += iter;
        }
    }
    printf("RESULT checksum %ld\n", total);
    return 0;
}
