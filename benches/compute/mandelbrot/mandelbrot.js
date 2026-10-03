// mandelbrot 400x400, escape iterations capped at 1000.
let total = 0;
for (let py = 0; py < 400; py++) {
    const y0 = (py * 3.0 / 399.0) - 1.5;
    for (let px = 0; px < 400; px++) {
        const x0 = (px * 3.0 / 399.0) - 2.0;
        let zx = 0.0, zy = 0.0, iter = 0;
        while (iter < 1000) {
            const zx2 = zx * zx, zy2 = zy * zy;
            if (zx2 + zy2 > 4.0) break;
            zy = 2.0 * zx * zy + y0;
            zx = zx2 - zy2 + x0;
            iter++;
        }
        total += iter;
    }
}
console.log('RESULT checksum', total);
