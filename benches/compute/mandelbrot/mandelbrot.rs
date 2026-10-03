// mandelbrot 400x400, escape iterations capped at 1000.
fn main() {
    let mut total: i64 = 0;
    let mut py = 0;
    while py < 400 {
        let y0 = (py as f64 * 3.0 / 399.0) - 1.5;
        let mut px = 0;
        while px < 400 {
            let x0 = (px as f64 * 3.0 / 399.0) - 2.0;
            let mut zx = 0.0;
            let mut zy = 0.0;
            let mut iter: i64 = 0;
            while iter < 1000 {
                let zx2 = zx * zx;
                let zy2 = zy * zy;
                if zx2 + zy2 > 4.0 {
                    break;
                }
                zy = 2.0 * zx * zy + y0;
                zx = zx2 - zy2 + x0;
                iter += 1;
            }
            total += iter;
            px += 1;
        }
        py += 1;
    }
    println!("RESULT checksum {}", total);
}
