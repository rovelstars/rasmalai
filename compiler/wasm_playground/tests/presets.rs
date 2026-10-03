const HELLO: &str = "print(\"hello,\", \"world\");";
const SIMD: &str = "import { Vec4f } from \"@std/simd\";\n\nlet a = new Vec4f(1.0, 2.0, 3.0, 4.0);\nlet b = Vec4f.splat(2.0);\nlet c = a * b;\nprint(c.x(), c.y(), c.z(), c.w());\nprint(c.dot(b));";
const DEFER: &str = "defer { print(\"third\"); }\ndefer { print(\"second\"); }\nprint(\"first\");";
const MATCH: &str = "enum Shape { Circle(Float), Rect(Float, Float), Point }\n\nfn area(s: Shape): Float {\n    switch s {\n        case .Circle(r): return 3.14 * r * r;\n        case .Rect(w, h): return w * h;\n        case .Point: return 0.0;\n    }\n}\n\nprint(area(Shape.Circle(2.0)));\nprint(area(Shape.Rect(3.0, 4.0)));";

#[test]
fn playground_presets_check_clean() {
    for src in [HELLO, SIMD, DEFER, MATCH] {
        assert_eq!(wasm_playground::check(src), String::new(), "{src}");
    }
}

#[test]
fn playground_presets_run() {
    let out = wasm_playground::run(HELLO);
    assert!(out.contains("hello, world"), "{out}");
    assert!(out.contains("=> 0"), "{out}");
    let out = wasm_playground::run(SIMD);
    assert!(out.contains("2.0 4.0 6.0 8.0"), "{out}");
    assert!(out.contains("40.0"), "{out}");
    let out = wasm_playground::run(DEFER);
    assert_eq!(out, "first\nsecond\nthird\n=> 0", "{out}");
    let out = wasm_playground::run(MATCH);
    assert!(out.contains("12.56"), "{out}");
    assert!(out.contains("12.0"), "{out}");
}

#[test]
fn std_import_unknown_module_reports_e108() {
    let out = wasm_playground::check("import { X } from \"@std/nope\";\nfn main(): Int { return 0; }");
    assert!(out.contains("E108"), "{out}");
}
