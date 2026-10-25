use std::fmt::Write as _;

fn main() {
    let out = std::env::var("OUT_DIR").unwrap();
    let path = std::path::Path::new(&out).join("calls.rs");
    let mut code = String::new();
    code.push_str("pub unsafe fn raw_call(addr: usize, ret_float: bool, n: usize, pat: u64, ints: &[u64], floats: &[f64]) -> (u64, f64) {\n");
    code.push_str("    match (ret_float, n, pat) {\n");
    for ret in [false, true] {
        for n in 0..=6usize {
            for pat in 0..(1u64 << n) {
                let prefix = if ret { "f" } else { "i" };
                let call = format!("c_{prefix}{n:02}_{pat:02x}(addr, ints.as_ptr(), floats.as_ptr())");
                if ret {
                    let _ = write!(code, "        ({ret}, {n}, {pat}) => unsafe {{ (0, {call}) }},\n");
                } else {
                    let _ = write!(code, "        ({ret}, {n}, {pat}) => unsafe {{ ({call}, 0.0) }},\n");
                }
            }
        }
    }
    code.push_str("        _ => (u64::MAX, f64::from_bits(u64::MAX)),\n    }\n}\n");
    for ret in [false, true] {
        for n in 0..=6usize {
            for pat in 0..(1u64 << n) {
                let prefix = if ret { "f" } else { "i" };
                let mut params = Vec::new();
                let mut args = Vec::new();
                let mut ic = 0usize;
                let mut fc = 0usize;
                for j in 0..n {
                    if (pat >> j) & 1 == 1 {
                        params.push("f64".to_string());
                        args.push(format!("*f.add({fc})"));
                        fc += 1;
                    } else {
                        params.push("u64".to_string());
                        args.push(format!("*i.add({ic})"));
                        ic += 1;
                    }
                }
                let ret_ty = if ret { "f64" } else { "u64" };
                let _ = write!(
                    code,
                    "#[allow(clippy::too_many_arguments)]\n#[allow(unused_variables)]\npub unsafe fn c_{prefix}{n:02}_{pat:02x}(addr: usize, i: *const u64, f: *const f64) -> {ret_ty} {{\n"
                );
                let _ = write!(
                    code,
                    "    unsafe {{\n    let p: unsafe extern \"C\" fn({}) -> {ret_ty} = core::mem::transmute(addr);\n",
                    params.join(", ")
                );
                let _ = write!(code, "    p({})\n    }}\n}}\n", args.join(", "));
            }
        }
    }
    std::fs::write(&path, code).unwrap();
    println!("cargo:rerun-if-changed=build.rs");
}
