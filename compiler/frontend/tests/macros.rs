#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

use diagnostics::Code;
use frontend::ast as A;
use frontend::macros::{
    self, ComptimeRunner, ComptimeValue, MacroOptions, ITER_CAP, NODE_CAP,
};
use frontend::parser::Parser;
use std::cell::RefCell;

fn parse(src: &str) -> A::Module {
    Parser::parse_module(src).unwrap_or_else(|e| panic!("{e}"))
}

fn opts_in(dir: &std::path::Path) -> MacroOptions {
    MacroOptions::new(dir.to_path_buf())
}

fn tmp() -> crate::scratch::Scratch {
    crate::scratch::dir("fe-macros")
}

fn expand_a(src: &str, dir: &std::path::Path) -> (A::Module, Vec<diagnostics::Diagnostic>) {
    let mut m = parse(src);
    let mut opts = opts_in(dir);
    let diags = macros::expand_phase_a(&mut m, &mut opts);
    (m, diags)
}

fn hard_errors(diags: &[diagnostics::Diagnostic]) -> Vec<&diagnostics::Diagnostic> {
    diags.iter().filter(|d| !d.code.is_warning()).collect()
}

fn has_macro(m: &A::Module, name: &str) -> bool {
    let text = format!("{m:?}");
    text.contains(&format!("Macro {{ name: \"{name}\""))
}

fn count_macros(m: &A::Module, name: &str) -> usize {
    let text = format!("{m:?}");
    text.matches(&format!("Macro {{ name: \"{name}\"")).count()
}

struct FakeRunner {
    calls: RefCell<Vec<(String, Vec<ComptimeValue>)>>,
    method_src: String,
}

impl FakeRunner {
    fn with_method(src: &str) -> FakeRunner {
        FakeRunner {
            calls: RefCell::new(Vec::new()),
            method_src: src.to_string(),
        }
    }
}

impl ComptimeRunner for FakeRunner {
    fn call(
        &self,
        fn_name: &str,
        _fn_span: diagnostics::Span,
        args: Vec<ComptimeValue>,
        _use_span: diagnostics::Span,
    ) -> Result<ComptimeValue, diagnostics::Diagnostic> {
        self.calls.borrow_mut().push((fn_name.to_string(), args));
        Ok(ComptimeValue::Array(vec![ComptimeValue::Str(
            self.method_src.clone(),
        )]))
    }
}

#[test]
fn phase_a_vec_bracket_expands() {
    let dir = tmp();
    let (m, diags) = expand_a("fn Main(): Int { let v = vec![1, 2, 3]; return v[0]; }", &dir);
    assert!(hard_errors(&diags).is_empty(), "{diags:?}");
    assert!(!has_macro(&m, "vec"));
    let text = format!("{m:?}");
    assert!(text.contains("Array"), "{text}");
}

#[test]
fn phase_a_vec_paren_expands() {
    let dir = tmp();
    let (m, diags) = expand_a("fn Main(): Int { let v = vec!(1); return v[0]; }", &dir);
    assert!(hard_errors(&diags).is_empty(), "{diags:?}");
    assert!(!has_macro(&m, "vec"));
    let text = format!("{m:?}");
    assert!(text.contains("Array"), "{text}");
}

#[test]
fn phase_a_unknown_macro_untouched() {
    let dir = tmp();
    let (m, diags) = expand_a("fn Main(): Int { return Foo!(1); }", &dir);
    assert!(hard_errors(&diags).is_empty(), "{diags:?}");
    assert!(has_macro(&m, "Foo"));
}

#[test]
fn phase_a_embed_bytes_splices_ints() {
    let dir = tmp();
    std::fs::write(dir.join("icon.bin"), [7u8, 8, 9]).unwrap();
    let (m, diags) = expand_a(
        "fn Main(): Int { let b = EmbedBytes!(\"icon.bin\"); return b[1]; }",
        &dir,
    );
    assert!(hard_errors(&diags).is_empty(), "{diags:?}");
    assert!(!has_macro(&m, "EmbedBytes"));
    let text = format!("{m:?}");
    assert!(text.contains("Int(7)") && text.contains("Int(8)") && text.contains("Int(9)"), "{text}");
}

#[test]
fn phase_a_embed_cache_key_includes_file_hash() {
    let dir = tmp();
    std::fs::write(dir.join("a.bin"), [1u8]).unwrap();
    let mut opts = opts_in(&dir);
    let mut m1 = parse("fn Main(): Int { let b = EmbedBytes!(\"a.bin\"); return b[0]; }");
    let d1 = macros::expand_phase_a(&mut m1, &mut opts);
    assert!(hard_errors(&d1).is_empty(), "{d1:?}");
    let first = format!("{m1:?}");
    assert!(first.contains("Int(1)"), "{first}");
    let cached = opts.cache.len();

    std::fs::write(dir.join("a.bin"), [2u8]).unwrap();
    let mut m2 = parse("fn Main(): Int { let b = EmbedBytes!(\"a.bin\"); return b[0]; }");
    let d2 = macros::expand_phase_a(&mut m2, &mut opts);
    assert!(hard_errors(&d2).is_empty(), "{d2:?}");
    let second = format!("{m2:?}");
    assert!(second.contains("Int(2)"), "{second}");
    assert!(opts.cache.len() > cached);

    std::fs::write(dir.join("a.bin"), [2u8]).unwrap();
    let before = opts.cache.len();
    let mut m3 = parse("fn Main(): Int { let b = EmbedBytes!(\"a.bin\"); return b[0]; }");
    let d3 = macros::expand_phase_a(&mut m3, &mut opts);
    assert!(hard_errors(&d3).is_empty(), "{d3:?}");
    assert_eq!(opts.cache.len(), before);
    assert_eq!(format!("{m3:?}"), second);
}

#[test]
fn phase_a_embed_rejects_missing_absolute_escape() {
    let dir = tmp();
    let (_, missing) = expand_a("fn Main(): Int { return EmbedBytes!(\"nope.bin\")[0]; }", &dir);
    assert!(missing.iter().any(|d| d.code == Code::E122), "{missing:?}");
    let (_, abs) = expand_a("fn Main(): Int { return EmbedBytes!(\"/etc/hostname\")[0]; }", &dir);
    assert!(abs.iter().any(|d| d.code == Code::E122), "{abs:?}");
    let (_, esc) = expand_a("fn Main(): Int { return EmbedBytes!(\"../outside.bin\")[0]; }", &dir);
    assert!(esc.iter().any(|d| d.code == Code::E122), "{esc:?}");
}

#[test]
fn phase_a_embed_nonliteral_rejected() {
    let dir = tmp();
    let (_, diags) = expand_a(
        "fn Main(): Int { let p = \"a.bin\"; return EmbedBytes!(p)[0]; }",
        &dir,
    );
    assert!(diags.iter().any(|d| d.code == Code::E122), "{diags:?}");
}

#[test]
fn phase_a_span_attribution_is_use_site() {
    let dir = tmp();
    let src = "fn Main(): Int { return vec![5][0]; }";
    let mut m = parse(src);
    let use_span = match &m.decls[0].node {
        A::Decl::Fn(f) => match &f.body {
            A::FnBody::Block(b) => match &b.stmts[0].node {
                A::Stmt::Return(Some(e)) => match &e.node {
                    A::Expr::Index { base, .. } => base.span,
                    _ => panic!("shape"),
                },
                _ => panic!("shape"),
            },
            _ => panic!("shape"),
        },
        _ => panic!("shape"),
    };
    let mut opts = opts_in(&dir);
    let diags = macros::expand_phase_a(&mut m, &mut opts);
    assert!(hard_errors(&diags).is_empty(), "{diags:?}");
    match &m.decls[0].node {
        A::Decl::Fn(f) => match &f.body {
            A::FnBody::Block(b) => match &b.stmts[0].node {
                A::Stmt::Return(Some(e)) => match &e.node {
                    A::Expr::Index { base, .. } => assert_eq!(base.span, use_span),
                    _ => panic!("shape"),
                },
                _ => panic!("shape"),
            },
            _ => panic!("shape"),
        },
        _ => panic!("shape"),
    }
}

#[test]
fn phase_a_nested_macros_converge() {
    let dir = tmp();
    let (m, diags) = expand_a("fn Main(): Int { let v = vec![vec![1][0], vec![2][0]]; return v[0]; }", &dir);
    assert!(hard_errors(&diags).is_empty(), "{diags:?}");
    assert_eq!(count_macros(&m, "vec"), 0);
}

#[test]
fn phase_a_node_cap_trips() {
    let dir = tmp();
    let mut items = String::new();
    for i in 0..(NODE_CAP + 1) {
        if i > 0 {
            items.push_str(", ");
        }
        items.push_str("1");
    }
    let src = format!("fn Main(): Int {{ let v = vec![{items}]; return v[0]; }}");
    let (_, diags) = expand_a(&src, &dir);
    assert!(diags.iter().any(|d| d.code == Code::E119), "{diags:?}");
}

#[test]
fn phase_a_iter_cap_is_eight() {
    assert_eq!(ITER_CAP, 8);
}

#[test]
fn static_assert_pass_erases_to_true() {
    let dir = tmp();
    let mut m = parse("fn Main(): Int { StaticAssert!(1 + 1 == 2, \"math\"); return 1; }");
    let mut opts = opts_in(&dir);
    let diags = macros::expand_phase_b(&mut m, &mut opts, None);
    assert!(hard_errors(&diags).is_empty(), "{diags:?}");
    assert!(!has_macro(&m, "StaticAssert"));
}

#[test]
fn static_assert_fail_reports_e123_with_span() {
    let dir = tmp();
    let mut m = parse("fn Main(): Int { StaticAssert!(1 == 2, \"boom\"); return 1; }");
    let mut opts = opts_in(&dir);
    let diags = macros::expand_phase_b(&mut m, &mut opts, None);
    let errs = hard_errors(&diags);
    assert_eq!(errs.len(), 1);
    assert_eq!(errs[0].code, Code::E123);
    assert!(errs[0].message.contains("boom"), "{}", errs[0].message);
    assert!(errs[0].span.is_some());
}

#[test]
fn static_assert_nonconst_is_e123() {
    let dir = tmp();
    let mut m = parse("fn Main(x: Int): Int { StaticAssert!(x, \"no\"); return 1; }");
    let mut opts = opts_in(&dir);
    let diags = macros::expand_phase_b(&mut m, &mut opts, None);
    assert!(diags.iter().any(|d| d.code == Code::E123), "{diags:?}");
}

#[test]
fn comptime_const_splice_in_static() {
    let dir = tmp();
    let mut m = parse("const N: Int = CompTime!(6 * 7); fn Main(): Int { return N; }");
    let mut opts = opts_in(&dir);
    let diags = macros::expand_phase_b(&mut m, &mut opts, None);
    assert!(hard_errors(&diags).is_empty(), "{diags:?}");
    match &m.decls[0].node {
        A::Decl::Const { value, .. } => assert!(matches!(value.node, A::Expr::Int(42)), "{:?}", value.node),
        _ => panic!("shape"),
    }
}

#[test]
fn comptime_unknown_fn_is_e122() {
    let dir = tmp();
    let mut m = parse("const N: Int = CompTime!(Missing(1)); fn Main(): Int { return N; }");
    let mut opts = opts_in(&dir);
    let diags = macros::expand_phase_b(&mut m, &mut opts, None);
    assert!(diags.iter().any(|d| d.code == Code::E122), "{diags:?}");
}

#[test]
fn derive_clone_debug_struct() {
    let dir = tmp();
    let mut m = parse("#[Derive(Clone, Debug)] struct Pair { let a: Int; let b: Int } fn Main(): Int { return 0; }");
    let mut opts = opts_in(&dir);
    let diags = macros::expand_phase_b(&mut m, &mut opts, None);
    assert!(hard_errors(&diags).is_empty(), "{diags:?}");
    match &m.decls[0].node {
        A::Decl::Struct { members, attrs, .. } => {
            let names: Vec<String> = members
                .iter()
                .filter_map(|x| match &x.node {
                    A::ClassMember::Method(f) => Some(f.name.clone()),
                    _ => None,
                })
                .collect();
            assert!(names.contains(&"clone".to_string()), "{names:?}");
            assert!(names.contains(&"debug".to_string()), "{names:?}");
            assert!(attrs.is_empty(), "{attrs:?}");
        }
        _ => panic!("shape"),
    }
}

#[test]
fn derive_collision_is_e121_at_existing_member() {
    let dir = tmp();
    let mut m = parse("#[Derive(Clone)] struct S { let a: Int fn clone(): S { return S(this.a); } } fn Main(): Int { return 0; }");
    let mut opts = opts_in(&dir);
    let diags = macros::expand_phase_b(&mut m, &mut opts, None);
    let err = diags.iter().find(|d| d.code == Code::E121).expect("{diags:?}");
    assert!(err.message.contains("derive at"), "{}", err.message);
    assert!(err.span.is_some());
}

#[test]
fn derive_unknown_name_fails_closed() {
    let dir = tmp();
    let mut m = parse("#[Derive(Foo)] struct S { let a: Int } fn Main(): Int { return 0; }");
    let mut opts = opts_in(&dir);
    let diags = macros::expand_phase_b(&mut m, &mut opts, None);
    assert!(diags.iter().any(|d| d.code == Code::E108 && d.message.contains("Foo")), "{diags:?}");
}

#[test]
fn derive_serde_is_out_of_scope() {
    let dir = tmp();
    let mut m = parse("#[Derive(Serde)] struct S { let a: Int } fn Main(): Int { return 0; }");
    let mut opts = opts_in(&dir);
    let diags = macros::expand_phase_b(&mut m, &mut opts, None);
    assert!(diags.iter().any(|d| d.code == Code::E108 && d.message.contains("Serde")), "{diags:?}");
}

#[test]
fn closed_namespace_rejects_unknown_attr() {
    let dir = tmp();
    let mut m = parse("#[Bogus] struct S { let a: Int } fn Main(): Int { return 0; }");
    let mut opts = opts_in(&dir);
    let diags = macros::expand_phase_b(&mut m, &mut opts, None);
    assert!(diags.iter().any(|d| d.code == Code::E108 && d.message.contains("Bogus")), "{diags:?}");
    let mut m = parse("#[Bogus] fn F(): Int { return 1; } fn Main(): Int { return F(); }");
    let mut opts = opts_in(&dir);
    let diags = macros::expand_phase_b(&mut m, &mut opts, None);
    assert!(diags.iter().any(|d| d.code == Code::E108 && d.message.contains("Bogus")), "{diags:?}");
}

#[test]
fn known_attrs_pass_closed_check() {
    let dir = tmp();
    let mut m = parse("#[Allow(CyclicReference)] struct S { let a: Int } #[Inline(Always)] fn F(): Int { return 1; } fn Main(): Int { return F(); }");
    let mut opts = opts_in(&dir);
    let diags = macros::expand_phase_b(&mut m, &mut opts, None);
    assert!(hard_errors(&diags).is_empty(), "{diags:?}");
}

#[test]
fn user_derive_runs_once_with_typeinfo() {
    let dir = tmp();
    let mut m = parse("#[CompTime] fn Describe(info: Array<String>): Array<String> { return info; } #[Derive(Describe)] struct Point { let x: Int; let y: String } fn Main(): Int { return 0; }");
    let mut opts = opts_in(&dir);
    let runner = FakeRunner::with_method("fn describe(): String { return \"Point\"; }");
    let diags = macros::expand_phase_b(&mut m, &mut opts, Some(&runner));
    assert!(hard_errors(&diags).is_empty(), "{diags:?}");
    assert_eq!(runner.calls.borrow().len(), 1);
    let (name, args) = &runner.calls.borrow()[0];
    assert_eq!(name, "Describe");
    assert_eq!(args.len(), 1);
    match &args[0] {
        ComptimeValue::Array(items) => {
            assert_eq!(items[0], ComptimeValue::Str("Point".to_string()));
            let mut rest: Vec<String> = items[1..]
                .iter()
                .map(|i| match i {
                    ComptimeValue::Str(s) => s.clone(),
                    _ => panic!("typeinfo"),
                })
                .collect();
            rest.sort();
            assert_eq!(rest, vec!["x:Int".to_string(), "y:String".to_string()]);
        }
        _ => panic!("typeinfo shape"),
    }
    let diags = macros::expand_phase_b(&mut m, &mut opts, Some(&runner));
    assert!(hard_errors(&diags).is_empty(), "{diags:?}");
    assert_eq!(runner.calls.borrow().len(), 1);
    match &m.decls[1].node {
        A::Decl::Struct { members, .. } => {
            assert!(members.iter().any(|x| matches!(&x.node, A::ClassMember::Method(f) if f.name == "describe")));
        }
        _ => panic!("shape"),
    }
}

#[test]
fn hygiene_renames_generated_locals() {
    let dir = tmp();
    let mut m = parse("#[CompTime] fn Describe(info: Array<String>): Array<String> { return info; } #[Derive(Describe)] struct S { let a: Int } fn Main(): Int { let tmp = 41; return tmp; }");
    let mut opts = opts_in(&dir);
    let runner = FakeRunner::with_method("fn m(): Int { let tmp = 1; return tmp; }");
    let diags = macros::expand_phase_b(&mut m, &mut opts, Some(&runner));
    assert!(hard_errors(&diags).is_empty(), "{diags:?}");
    match &m.decls[1].node {
        A::Decl::Struct { members, .. } => match &members[1].node {
            A::ClassMember::Method(f) => match &f.body {
                A::FnBody::Block(b) => match &b.stmts[1].node {
                    A::Stmt::Return(Some(e)) => match &e.node {
                        A::Expr::Ident(n) => assert!(n.starts_with("__m"), "{n}"),
                        _ => panic!("shape"),
                    },
                    _ => panic!("shape"),
                },
                _ => panic!("shape"),
            },
            _ => panic!("shape"),
        },
        _ => panic!("shape"),
    }
}

#[test]
fn expansions_are_deterministic() {
    let dir = tmp();
    std::fs::write(dir.join("d.bin"), [3u8, 1, 2]).unwrap();
    let src = "#[Derive(Clone, Debug)] struct S { let a: Int } fn Main(): Int { let b = EmbedBytes!(\"d.bin\"); let v = vec![1]; StaticAssert!(2 > 1, \"x\"); return b[0] + v[0]; }";
    let run = || {
        let mut m = parse(src);
        let mut opts = opts_in(&dir);
        let d1 = macros::expand_phase_a(&mut m, &mut opts);
        assert!(hard_errors(&d1).is_empty(), "{d1:?}");
        let d2 = macros::expand_phase_b(&mut m, &mut opts, None);
        assert!(hard_errors(&d2).is_empty(), "{d2:?}");
        format!("{m:?}")
    };
    assert_eq!(run(), run());
}

#[test]
fn zero_cost_derive_free_build_never_runs_comptime() {
    macros::reset_comptime_runs();
    let dir = tmp();
    let mut m = parse("fn Main(): Int { let v = vec![1]; return v[0]; }");
    let mut opts = opts_in(&dir);
    let d1 = macros::expand_phase_a(&mut m, &mut opts);
    assert!(hard_errors(&d1).is_empty(), "{d1:?}");
    let d2 = macros::expand_phase_b(&mut m, &mut opts, None);
    assert!(hard_errors(&d2).is_empty(), "{d2:?}");
    assert_eq!(macros::comptime_runs(), 0);
}

#[test]
fn print_stays_prelude_function_not_macro() {
    let m = parse("fn Main(): Int { print(\"hi\"); return 0; }");
    match &m.decls[0].node {
        A::Decl::Fn(f) => match &f.body {
            A::FnBody::Block(b) => match &b.stmts[0].node {
                A::Stmt::Expr(e) => assert!(matches!(e.node, A::Expr::Call { .. }), "{:?}", e.node),
                _ => panic!("shape"),
            },
            _ => panic!("shape"),
        },
        _ => panic!("shape"),
    }
    let m = parse("fn Main(): Int { print(\"hi\"); return 0; }");
    let errs: Vec<_> = frontend::semantic::check(&m)
        .into_iter()
        .filter(|d| !d.code.is_warning())
        .collect();
    assert!(errs.is_empty(), "{errs:?}");
}

#[test]
fn lowercase_exception_is_only_vec() {
    let dir = tmp();
    let (m, diags) = expand_a("fn Main(): Int { return Vec!(1) + Foo!(2); }", &dir);
    assert!(hard_errors(&diags).is_empty(), "{diags:?}");
    assert!(has_macro(&m, "Vec"));
    assert!(has_macro(&m, "Foo"));
}

#[test]
fn expand_dump_writes_before_after() {
    let dir = tmp();
    let dump = dir.join("exp");
    let mut m = parse("fn Main(): Int { let v = vec![1]; StaticAssert!(true, \"x\"); return v[0]; }");
    let mut opts = opts_in(&dir);
    opts.dump_dir = Some(dump.clone());
    let d1 = macros::expand_phase_a(&mut m, &mut opts);
    assert!(hard_errors(&d1).is_empty(), "{d1:?}");
    let d2 = macros::expand_phase_b(&mut m, &mut opts, None);
    assert!(hard_errors(&d2).is_empty(), "{d2:?}");
    for f in ["phase-a.before", "phase-a.after", "phase-b.before", "phase-b.after"] {
        assert!(dump.join(f).is_file(), "{f}");
    }
}

#[test]
fn comptime_toolchain_is_versioned() {
    let tc = macros::toolchain();
    assert!(tc.starts_with("rasmalai-macros/"), "{tc}");
    assert!(macros::FUEL_STEPS == 1_000_000);
    assert!(macros::CALL_DEPTH_MAX == 64);
    assert!(macros::NODE_CAP == 10_000);
}
