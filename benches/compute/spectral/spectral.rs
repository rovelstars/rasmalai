// spectral-norm N=300, 10 power iterations on AtA.
const N: usize = 300;

fn a_elem(i: usize, j: usize) -> f64 {
    let w = (i + j) * (i + j + 1) / 2 + i + 1;
    1.0 / w as f64
}

fn av(x: &[f64], y: &mut [f64]) {
    for i in 0..N {
        let mut s = 0.0;
        for j in 0..N {
            s += a_elem(i, j) * x[j];
        }
        y[i] = s;
    }
}

fn atv(x: &[f64], y: &mut [f64]) {
    for i in 0..N {
        let mut s = 0.0;
        for j in 0..N {
            s += a_elem(j, i) * x[j];
        }
        y[i] = s;
    }
}

fn main() {
    let mut u = vec![1.0f64; N];
    let mut v = vec![0.0f64; N];
    let mut tmp = vec![0.0f64; N];
    for _ in 0..10 {
        av(&u, &mut tmp);
        atv(&tmp, &mut v);
        av(&v, &mut tmp);
        atv(&tmp, &mut u);
    }
    let mut vbv = 0.0;
    let mut vv = 0.0;
    for j in 0..N {
        vbv += u[j] * v[j];
        vv += v[j] * v[j];
    }
    println!("RESULT checksum {:.16e}", (vbv / vv).sqrt());
}
