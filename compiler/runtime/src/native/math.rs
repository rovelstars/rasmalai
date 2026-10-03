use super::collections::*;
use super::io::*;

pub const CANONICAL_NAN_BITS: u64 = 0x7ff8_0000_0000_0000;
pub const CANONICAL_NAN_F32_BITS: u32 = 0x7fc0_0000;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_black_box_i64(v: i64) -> i64 {
    v
}

pub fn splitmix_next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E3779B97F4A7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^ (z >> 31)
}

pub fn xoshiro_next(s: &mut [u64; 4]) -> u64 {
    let result = s[0].wrapping_add(s[3]).rotate_left(23).wrapping_add(s[0]);
    let t = s[1] << 17;
    s[2] ^= s[0];
    s[3] ^= s[1];
    s[1] ^= s[2];
    s[0] ^= s[3];
    s[2] ^= t;
    s[3] = s[3].rotate_left(45);
    result
}

pub fn prng_seed_state(seed: u64) -> [u64; 4] {
    let mut mix = seed;
    let mut s = [splitmix_next(&mut mix), splitmix_next(&mut mix), splitmix_next(&mut mix), splitmix_next(&mut mix)];
    if s == [0, 0, 0, 0] {
        s[0] = 1;
    }
    s
}

fn prng_slots(arr: *mut u8) -> *mut u64 {
    if arr.is_null() || arr_len(arr) < 4 {
        unsafe {
            rnx_panic(b"prng state needs 4 slots\0".as_ptr(), "prng state needs 4 slots".len());
        }
    }
    arr_data(arr) as *mut u64
}

fn math1(bits: u64, f: impl FnOnce(f64) -> f64) -> u64 {
    f(f64::from_bits(bits)).to_bits()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_math_sqrt(bits: u64) -> u64 {
    math1(bits, f64::sqrt)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_math_sin(bits: u64) -> u64 {
    math1(bits, f64::sin)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_math_cos(bits: u64) -> u64 {
    math1(bits, f64::cos)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_math_tan(bits: u64) -> u64 {
    math1(bits, f64::tan)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_math_atan2(y_bits: u64, x_bits: u64) -> u64 {
    f64::from_bits(y_bits).atan2(f64::from_bits(x_bits)).to_bits()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_math_pow(base_bits: u64, exp_bits: u64) -> u64 {
    f64::from_bits(base_bits).powf(f64::from_bits(exp_bits)).to_bits()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_math_floor(bits: u64) -> u64 {
    math1(bits, f64::floor)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_math_ceil(bits: u64) -> u64 {
    math1(bits, f64::ceil)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_math_round(bits: u64) -> u64 {
    math1(bits, f64::round)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_math_log(bits: u64) -> u64 {
    math1(bits, f64::ln)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_float_nan() -> u64 {
    CANONICAL_NAN_BITS
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_float_to_bits(bits: u64) -> u64 {
    bits
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_float_from_bits(bits: u64) -> u64 {
    bits
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_float_fma(a_bits: u64, b_bits: u64, c_bits: u64) -> u64 {
    f64::from_bits(a_bits)
        .mul_add(f64::from_bits(b_bits), f64::from_bits(c_bits))
        .to_bits()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_crypto_random_u64() -> u64 {
    let mut f = std::fs::File::open("/dev/urandom").unwrap_or_else(|_| {
        unsafe {
            rnx_panic(b"no entropy source\0".as_ptr(), "no entropy source".len());
        }
    });
    let mut buf = [0u8; 8];
    std::io::Read::read_exact(&mut f, &mut buf).unwrap_or_else(|_| {
        unsafe {
            rnx_panic(b"entropy read failed\0".as_ptr(), "entropy read failed".len());
        }
    });
    u64::from_ne_bytes(buf)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_prng_seed(state_out: *mut u8, seed: u64) {
    let slots = prng_slots(state_out);
    let s = prng_seed_state(seed);
    unsafe {
        slots.write_unaligned(s[0]);
        slots.add(1).write_unaligned(s[1]);
        slots.add(2).write_unaligned(s[2]);
        slots.add(3).write_unaligned(s[3]);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_prng_next(state_inout: *mut u8) -> u64 {
    let slots = prng_slots(state_inout);
    let mut s = unsafe {
        [slots.read_unaligned(), slots.add(1).read_unaligned(), slots.add(2).read_unaligned(), slots.add(3).read_unaligned()]
    };
    let out = xoshiro_next(&mut s);
    unsafe {
        slots.write_unaligned(s[0]);
        slots.add(1).write_unaligned(s[1]);
        slots.add(2).write_unaligned(s[2]);
        slots.add(3).write_unaligned(s[3]);
    }
    out
}

#[cfg(test)]
mod prng_tests {
    use super::*;

    #[test]
    fn seed_is_deterministic() {
        assert_eq!(prng_seed_state(1337), prng_seed_state(1337));
        assert_ne!(prng_seed_state(1337), prng_seed_state(1338));
    }

    #[test]
    fn stream_is_deterministic() {
        let mut a = prng_seed_state(42);
        let mut b = prng_seed_state(42);
        for _ in 0..16 {
            assert_eq!(xoshiro_next(&mut a), xoshiro_next(&mut b));
        }
    }

    #[test]
    fn stream_varies_and_uses_high_bits() {
        let mut s = prng_seed_state(7);
        let mut seen_high = false;
        let mut prev = xoshiro_next(&mut s);
        for _ in 0..64 {
            let v = xoshiro_next(&mut s);
            assert_ne!(v, prev);
            seen_high |= (v ^ prev) >> 63 != 0;
            prev = v;
        }
        assert!(seen_high);
    }

    #[test]
    fn raw_pointer_roundtrip() {
        unsafe {
            let a = rnx_array_new(4, 8);
            assert!(!a.is_null());
            for _ in 0..4 {
                rnx_array_push(a, 0, 8);
            }
            rnx_prng_seed(a, 1337);
            let x1 = rnx_prng_next(a);
            let x2 = rnx_prng_next(a);
            rnx_prng_seed(a, 1337);
            assert_eq!(rnx_prng_next(a), x1);
            assert_eq!(rnx_prng_next(a), x2);
            let mut s = prng_seed_state(1337);
            assert_eq!(xoshiro_next(&mut s), x1);
            rnx_release_array(a, 8, None);
        }
    }

    #[test]
    fn crypto_runs() {
        unsafe {
            let _ = rnx_crypto_random_u64();
        }
    }
}

#[cfg(test)]
mod math_tests {
    use super::*;

    fn f(bits: u64) -> f64 {
        f64::from_bits(bits)
    }

    #[test]
    fn scalar_identities() {
        unsafe {
            assert_eq!(f(rnx_math_sqrt(16.0f64.to_bits())), 4.0);
            assert_eq!(f(rnx_math_pow(2.0f64.to_bits(), 3.0f64.to_bits())), 8.0);
            assert_eq!(f(rnx_math_floor(2.7f64.to_bits())), 2.0);
            assert_eq!(f(rnx_math_ceil(2.3f64.to_bits())), 3.0);
            assert_eq!(f(rnx_math_round(2.5f64.to_bits())), 3.0);
            assert!((f(rnx_math_sin(0.0f64.to_bits()))).abs() < 1e-12);
            assert!((f(rnx_math_cos(0.0f64.to_bits())) - 1.0).abs() < 1e-12);
            assert!((f(rnx_math_tan(0.0f64.to_bits()))).abs() < 1e-12);
            assert!((f(rnx_math_atan2(1.0f64.to_bits(), 1.0f64.to_bits())) - std::f64::consts::FRAC_PI_4).abs() < 1e-12);
            assert!(f(rnx_math_sqrt((-1.0f64).to_bits())).is_nan());
        }
    }
}
