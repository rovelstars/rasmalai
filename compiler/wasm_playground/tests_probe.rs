#[test]
fn probe() {
    let src = "import { Vec4f } from \"@std/simd\";\nfn main(): Int { let v = Vec4f(1.0, 2.0, 3.0, 4.0); return 1; }";
    let (w, e) = frontend::check::check_source_all(src);
    println!("warnings={} errors={}", w.len(), e.len());
    for d in e.iter().chain(w.iter()) { println!("{d}"); }
}
