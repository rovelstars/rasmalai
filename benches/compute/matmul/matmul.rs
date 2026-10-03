// matmul 256x256 integer matrices, ijk loop order.
const N: usize = 256;

fn main() {
    let mut a = vec![0i64; N * N];
    let mut b = vec![0i64; N * N];
    let mut c = vec![0i64; N * N];
    for r in 0..N {
        for k in 0..N {
            a[r * N + k] = ((r + k) % 64) as i64;
            b[r * N + k] = ((r * k) % 64) as i64;
        }
    }
    for x in 0..N {
        for y in 0..N {
            let mut s = 0i64;
            for z in 0..N {
                s += a[x * N + z] * b[z * N + y];
            }
            c[x * N + y] = s;
        }
    }
    let total: i64 = c.iter().sum();
    println!("RESULT checksum {}", total);
}
