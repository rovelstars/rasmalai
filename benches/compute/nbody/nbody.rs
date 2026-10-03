// nbody: 5 bodies, classic Benchmarks-Game units, 100k steps, dt = 0.01.
const PI: f64 = 3.141592653589793;
const SOLAR: f64 = 4.0 * PI * PI;
const DAYS: f64 = 365.24;

fn main() {
    let mut px: [f64; 5] = [0.0, 4.8414314424647209, 8.3433667182445799, 12.894369562139131, 15.3796971148509165];
    let mut py: [f64; 5] = [0.0, -1.1603200440274284, 4.1247985641243048, -15.1111514016986312, -25.9193146099879641];
    let mut pz: [f64; 5] = [0.0, -0.10362204447112311, -0.40352341711432138, -0.22330757889265573, 0.17925877295037118];
    let mut vx: [f64; 5] = [0.0, 0.0016600766427440369, -0.0027674251072686241, 0.0029646013756476162, 0.0026806777249038932];
    let mut vy: [f64; 5] = [0.0, 0.0076990111841974043, 0.0049985280123491724, 0.0023784717395948095, 0.001628241700382423];
    let mut vz: [f64; 5] = [0.0, -0.0000690460016972063, 0.00002304172975737639, -0.00002965895685402376, -0.00009515922545197159];
    let m: [f64; 5] = [SOLAR, 0.00095479193842432661 * SOLAR, 0.00028588598066613081 * SOLAR,
             0.00004366244043351563 * SOLAR, 0.00005151389302046611 * SOLAR];
    for i in 0..5 {
        vx[i] *= DAYS;
        vy[i] *= DAYS;
        vz[i] *= DAYS;
    }
    let mut ox = 0.0;
    let mut oy = 0.0;
    let mut oz = 0.0;
    for i in 1..5 {
        ox += vx[i] * m[i];
        oy += vy[i] * m[i];
        oz += vz[i] * m[i];
    }
    vx[0] = -ox / m[0];
    vy[0] = -oy / m[0];
    vz[0] = -oz / m[0];

    for _ in 0..100000 {
        for i in 0..5 {
            for j in (i + 1)..5 {
                let dx = px[i] - px[j];
                let dy = py[i] - py[j];
                let dz = pz[i] - pz[j];
                let d2 = dx * dx + dy * dy + dz * dz;
                let mag = 0.01 / (d2 * d2.sqrt());
                vx[i] -= dx * m[j] * mag;
                vy[i] -= dy * m[j] * mag;
                vz[i] -= dz * m[j] * mag;
                vx[j] += dx * m[i] * mag;
                vy[j] += dy * m[i] * mag;
                vz[j] += dz * m[i] * mag;
            }
        }
        for k in 0..5 {
            px[k] += 0.01 * vx[k];
            py[k] += 0.01 * vy[k];
            pz[k] += 0.01 * vz[k];
        }
    }

    let mut e = 0.0;
    for i in 0..5 {
        e += 0.5 * m[i] * (vx[i] * vx[i] + vy[i] * vy[i] + vz[i] * vz[i]);
        for j in (i + 1)..5 {
            let dx = px[i] - px[j];
            let dy = py[i] - py[j];
            let dz = pz[i] - pz[j];
            e -= m[i] * m[j] / (dx * dx + dy * dy + dz * dz).sqrt();
        }
    }
    println!("RESULT checksum {:.16e}", e);
}
