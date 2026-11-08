#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

use lir::{lower, verify};

fn compile(src: &str) -> lir::instr::Module {
    let mut m = frontend::parser::Parser::parse_module(src).unwrap_or_else(|e| panic!("{e}"));
    let d = frontend::desugar::desugar(&mut m);
    assert!(d.is_empty(), "{d:?}");
    let out = lower::lower(&m).unwrap_or_else(|e| panic!("{e}"));
    let v = verify::verify(&out);
    assert!(v.is_empty(), "{v:?}");
    out
}

fn count_instrs(m: &lir::instr::Module, pred: impl Fn(&lir::instr::Instr) -> bool) -> usize {
    m.functions
        .iter()
        .flat_map(|f| f.blocks.iter())
        .flat_map(|b| b.instrs.iter())
        .filter(|i| pred(i))
        .count()
}

#[test]
fn arithmetic_fn() {
    let m = compile("fn Add(a: Int, b: Int): Int { return a + b }");
    assert_eq!(m.functions.len(), 1);
    assert_eq!(m.functions[0].blocks.len(), 1);
}

#[test]
fn while_and_for() {
    let m = compile(
        "fn Sum(n: Int): Int { let t = 0 for i in 0..n { t += i } while t > 100 { t -= 10 } return t }",
    );
    assert!(m.functions[0].blocks.len() > 3);
}

#[test]
fn switch_try_defer() {
    let m = compile(
        r#"fn F(x: Int): Int {
  defer { print("out") }
  try { switch x { case 0..=5: return 1 default: return 2 } }
  catch (err) { return 0 }
}"#,
    );
    assert!(m.functions[0].blocks.len() >= 4);
}

#[test]
fn class_init_fields() {
    let m = compile(
        "class P { let x: Int let y: Int = 7 init(x: Int) { this.x = x } fn get(): Int { return this.x } }",
    );
    assert_eq!(m.classes.len(), 1);
    assert_eq!(m.classes[0].fields.len(), 2);
    assert!(m.fn_index.contains_key("P.init"));
    assert!(m.fn_index.contains_key("P.get"));
}

#[test]
fn closure_decay_lowes() {
    let m = compile(
        "class B { let cb: fn() init() { this.cb = fn decay(this) { return 1 } } }",
    );
    assert!(m.functions.iter().any(|f| f.name.contains("closure#")));
}

#[test]
fn e305_float_mix_fails() {
    let mut m =
        frontend::parser::Parser::parse_module("fn F(a: Float, b: FastFloat): Float { return a * b }")
            .unwrap();
    frontend::desugar::desugar(&mut m);
    let e = lower::lower(&m).expect_err("E305");
    assert_eq!(e.code.as_str(), "E305");
}

fn lower_err(src: &str) -> diagnostics::Code {
    let mut m = frontend::parser::Parser::parse_module(src).unwrap_or_else(|e| panic!("{e}"));
    let d = frontend::desugar::desugar(&mut m);
    assert!(d.is_empty(), "{d:?}");
    lower::lower(&m).expect_err("must fail").code
}

#[test]
fn record_value_type() {
    let m = compile("record Point(x: Float, y: Float) fn F(): Int { return 0 }");
    assert_eq!(m.classes.len(), 1);
    assert!(m.classes[0].is_struct);
    assert_eq!(m.classes[0].fields.len(), 2);
}

#[test]
fn enum_table() {
    let m = compile("enum Option<T> { None, Some(T) } fn F(): Int { return 0 }");
    let e = m.enums.iter().find(|e| e.name == "Option").unwrap();
    assert_eq!(e.variants.len(), 2);
    assert_eq!(e.variants[1].payload.len(), 1);
}

#[test]
fn enum_exhaustive_no_default() {
    compile(
        "enum Color { Red, Green } fn F(c: Color): Int { switch c { case .Red: return 1 case .Green: return 2 } }",
    );
}

#[test]
fn enum_default_skips_exhaustiveness() {
    compile(
        "enum Color { Red, Green, Blue } fn F(c: Color): Int { switch c { case .Red: return 1 default: return 0 } }",
    );
}

#[test]
fn enum_nonexhaustive_fails() {
    let code = lower_err(
        "enum Color { Red, Green } fn F(c: Color): Int { switch c { case .Red: return 1 } }",
    );
    assert_eq!(code, diagnostics::Code::E108);
}

#[test]
fn enum_arity_checked() {
    let code = lower_err("enum Option<T> { None, Some(T) } fn F(): Int { let x = Some(1, 2) return 0 }");
    assert_eq!(code, diagnostics::Code::E108);
}

#[test]
fn tuple_for_lowes() {
    let m = compile(
        "fn F(p: Array): Int { let t = 0 for (a, b) in p { t += 1 } return t }",
    );
    assert!(m.functions[0].blocks.len() > 3);
}

#[test]
fn tuple_for_over_range_fails() {
    let code = lower_err("fn F(): Int { for (a, b) in 0..10 { } return 0 }");
    assert_eq!(code, diagnostics::Code::E108);
}

#[test]
fn e201_unsafe_call() {
    let code = lower_err("unsafe fn R(): Int { return 1 } fn F(): Int { return R() }");
    assert_eq!(code, diagnostics::Code::E201);
}

#[test]
fn e201_allowed_in_unsafe() {
    compile("unsafe fn R(): Int { return 1 } fn F(): Int { unsafe { return R() } }");
    compile("unsafe fn R(): Int { return 1 } unsafe fn S(): Int { return R() }");
}

#[test]
fn e202_address_of() {
    let code = lower_err("fn F(): Int { let x = 1 let p = &x return 0 }");
    assert_eq!(code, diagnostics::Code::E202);
}

#[test]
fn unsafe_pointer_arith_compiles() {
    compile("fn F(): Int { unsafe { let x = 1 let p = &x let q = p + 1 return 0 } }");
}

#[test]
fn pointer_mul_rejected() {
    let code = lower_err("fn F(): Int { unsafe { let x = 1 let p = &x let q = p * 2 return 0 } }");
    assert_eq!(code, diagnostics::Code::E108);
}

#[test]
fn e202_pointer_param_arith() {
    let code = lower_err("fn F(p: Pointer): Int { let q = p + 1 return 0 }");
    assert_eq!(code, diagnostics::Code::E202);
}

const SECRET: &str = "class Point { let x: Int; private let secret: Int; init(x: Int, s: Int) { this.x = x; this.secret = s; } fn get(): Int { return this.secret; } }";

#[test]
fn internal_access_needs_no_modifier() {
    compile(&format!("{SECRET} fn Main(): Int {{ let pt = new Point(1, 2); return pt.x; }}"));
}

#[test]
fn private_read_inside_class_ok() {
    compile(&format!("{SECRET} fn Main(): Int {{ let pt = new Point(1, 2); return pt.get(); }}"));
}

#[test]
fn private_field_read_outside_fails() {
    let code = lower_err(&format!("{SECRET} fn Main(): Int {{ let pt = new Point(1, 2); return pt.secret; }}"));
    assert_eq!(code, diagnostics::Code::E203);
}

#[test]
fn private_field_write_outside_fails() {
    let code = lower_err(&format!("{SECRET} fn Main(): Int {{ let pt = new Point(1, 2); pt.secret = 9; return 0; }}"));
    assert_eq!(code, diagnostics::Code::E203);
}

#[test]
fn private_method_call_outside_fails() {
    let code = lower_err("class A { private fn hid(): Int { return 1 } } fn Main(): Int { let a = new A(); return a.hid(); }");
    assert_eq!(code, diagnostics::Code::E203);
}

#[test]
fn private_method_call_inside_ok() {
    compile("class A { private fn hid(): Int { return 1 } fn show(): Int { return this.hid(); } } fn Main(): Int { let a = new A(); return a.show(); }");
}

#[test]
fn public_member_access_ok() {
    compile("class A { public let x: Int; } fn Main(): Int { let a = new A(); return a.x; }");
}

#[test]
fn array_literal_typed() {
    use lir::instr::{Instr, LirType};
    let m = compile("fn F(): Int { let a = [10, 20]; return 0; }");
    let mut saw_new = false;
    let mut pushes = 0;
    for b in &m.functions[0].blocks {
        for i in &b.instrs {
            match i {
                Instr::ArrayNew { dst: _, cap, elem_size, .. } => {
                    saw_new = true;
                    assert_eq!((*cap, *elem_size), (2, 8));
                }
                Instr::ArrayPush { elem_size, .. } => {
                    pushes += 1;
                    assert_eq!(*elem_size, 8);
                }
                _ => {}
            }
        }
    }
    assert!(saw_new && pushes == 2);
    assert!(matches!(
        m.functions[0].locals.iter().find(|t| matches!(t, LirType::Array(_))),
        Some(LirType::Array(inner)) if **inner == LirType::I64
    ));
}

#[test]
fn array_annotation_parameterized() {
    use lir::instr::LirType;
    let m = compile("fn F(a: Array<Int>): Int { return 0; }");
    assert!(matches!(&m.functions[0].params[0], LirType::Array(inner) if **inner == LirType::I64));
}

#[test]
fn array_length_and_push_lower() {
    use lir::instr::Instr;
    let m = compile("fn F(): Int { let a = [1]; a.push(2); return a.length; }");
    let mut saw_push = false;
    let mut saw_len = false;
    for b in &m.functions[0].blocks {
        for i in &b.instrs {
            match i {
                Instr::ArrayPush { .. } => saw_push = true,
                Instr::ArrayLen { .. } => saw_len = true,
                _ => {}
            }
        }
    }
    assert!(saw_push && saw_len);
}

#[test]
fn array_index_get_set() {
    use lir::instr::Instr;
    let m = compile("fn F(): Int { let a = [1, 2]; a[0] = 9; return a[0]; }");
    let mut saw_get = false;
    let mut saw_set = false;
    for b in &m.functions[0].blocks {
        for i in &b.instrs {
            match i {
                Instr::ArrayGet { elem_size, .. } => {
                    saw_get = true;
                    assert_eq!(*elem_size, 8);
                }
                Instr::ArraySet { elem_size, .. } => {
                    saw_set = true;
                    assert_eq!(*elem_size, 8);
                }
                _ => {}
            }
        }
    }
    assert!(saw_get && saw_set);
}

#[test]
fn array_dtor_synthesized_for_objects() {
    let m = compile("class B { let v: Int; init(v: Int) { this.v = v; } } fn F(): Int { let a = [new B(1)]; return 0; }");
    assert!(m.array_dtors.values().any(|id| m.functions[*id].name.starts_with("__arrdtor_")));
}

#[test]
fn array_no_dtor_for_ints() {
    let m = compile("fn F(): Int { let a = [1, 2]; return 0; }");
    assert!(m.array_dtors.is_empty());
}

#[test]
fn for_over_array_uses_len_check() {
    use lir::instr::{Instr, Terminator};
    let m = compile("fn F(a: Array<Int>): Int { let t = 0; for x in a { t = t + x; } return t; }");
    let mut saw_len = false;
    let mut saw_get = false;
    for b in &m.functions[0].blocks {
        for i in &b.instrs {
            match i {
                Instr::ArrayLen { .. } => saw_len = true,
                Instr::ArrayGet { .. } => saw_get = true,
                _ => {}
            }
        }
        let _ = &b.term as &Terminator;
    }
    assert!(saw_len && saw_get);
}

#[test]
fn float_strict_fast_casts_are_copies() {
    use lir::instr::{FloatKind, Instr, LirType};
    let m = compile("fn F(c: Float): Float { let f = c.asFast(); return f.asStrict(); }");
    let mut copies = 0;
    for b in &m.functions[0].blocks {
        for i in &b.instrs {
            if matches!(i, Instr::Copy { .. }) {
                copies += 1;
            }
        }
    }
    assert!(copies >= 2);
    assert!(matches!(
        m.functions[0].locals.as_slice(),
        _ if m.functions[0].locals.contains(&LirType::F64(FloatKind::Fast))
            && m.functions[0].locals.contains(&LirType::F64(FloatKind::Strict))
    ));
}

#[test]
fn float_int_conversions() {
    use lir::instr::{ConvertKind, FloatKind, Instr};
    let m = compile("fn F(x: Int, y: Float): Int { let f = Float(x); return Int(y); }");
    let mut saw_to_float = false;
    let mut saw_to_int = false;
    for b in &m.functions[0].blocks {
        for i in &b.instrs {
            match i {
                Instr::Convert { kind: ConvertKind::IntToFloat(FloatKind::Strict), .. } => saw_to_float = true,
                Instr::Convert { kind: ConvertKind::FloatToInt, .. } => saw_to_int = true,
                _ => {}
            }
        }
    }
    assert!(saw_to_float && saw_to_int);
}

#[test]
fn float_fast_mix_rejected() {
    assert_eq!(lower_err("fn F(a: Float, b: FastFloat): Float { return a + b }"), diagnostics::Code::E305);
}

#[test]
fn float_literal_method_base() {
    let m = compile("fn F(): Float { return 2.0.asFast(); }");
    assert!(m.functions[0].blocks.iter().flat_map(|b| b.instrs.iter()).any(|i| matches!(i, lir::instr::Instr::Copy { .. })));
}

#[test]
fn enum_dtor_synthesized_for_owned_payload() {
    let m = compile("enum Msg { Blank, Text(Str) } fn F(): Int { return 0; }");
    let ei = m.enums.iter().position(|e| e.name == "Msg").unwrap();
    let id = m.enums[ei].dtor.expect("enum with Str payload needs a dtor");
    assert!(m.functions[id].name.starts_with("__enum_dtor_"));
}

#[test]
fn enum_dtor_skipped_for_plain_payload() {
    let m = compile("enum Shape { None, Circle(Int) } fn F(): Int { return 0; }");
    let ei = m.enums.iter().position(|e| e.name == "Shape").unwrap();
    assert!(m.enums[ei].dtor.is_none());
}

#[test]
fn enum_tag_and_payload_lowered() {
    use lir::instr::Instr;
    let m = compile("enum Msg { Blank, Text(Str) } fn Len(m: Msg): Int { switch m { case .Blank: return 0; case .Text(t): return 1; } }");
    let f = m.functions.iter().find(|f| f.name == "Len").unwrap();
    assert!(f.blocks.iter().flat_map(|b| b.instrs.iter()).any(|i| matches!(i, Instr::EnumPayload { .. })));
    let ei = m.enums.iter().position(|e| e.name == "Msg").unwrap();
    let dtor = &m.functions[m.enums[ei].dtor.unwrap()];
    assert!(dtor.blocks.iter().flat_map(|b| b.instrs.iter()).any(|i| matches!(i, Instr::EnumTag { .. })));
}

#[test]
fn float_field_arith_stays_float() {
    use lir::instr::{ArithOp, Instr, NumKind};
    let m = compile("record Vec2(x: Float, y: Float) fn F(v: Vec2): Float { return v.x + v.y; }");
    let f = m.functions.iter().find(|f| f.name == "F").unwrap();
    let mut saw_float_add = false;
    for b in &f.blocks {
        for i in &b.instrs {
            if let Instr::Arith { op: ArithOp::Add, kind: NumKind::Float(_), .. } = i {
                saw_float_add = true;
            }
        }
    }
    assert!(saw_float_add);
}

#[test]
fn thread_spawn_join_lower() {
    use lir::instr::Instr;
    let dir = crate::scratch::dir("lir");
    std::fs::write(dir.join("main.rnx"), "fn worker(): Int { return 1; } fn Main(): Int { let h = Thread.spawn(worker); return h.join().unwrap(); }").unwrap();
    let g = frontend::modules::ModuleGraph::build(&dir.join("main.rnx")).unwrap_or_else(|e| panic!("{e}"));
    let mut m = g.resolve().unwrap_or_else(|e| panic!("{e}"));
    let d = frontend::desugar::desugar(&mut m);
    assert!(d.is_empty(), "{d:?}");
    let m = lower::lower(&m).unwrap_or_else(|e| panic!("{e}"));
    let f = m.functions.iter().find(|f| f.name == "Main").unwrap();
    let mut saw_spawn = false;
    let mut saw_enum = false;
    for b in &f.blocks {
        for i in &b.instrs {
            match i {
                Instr::ThreadSpawn { func, closure, .. } => {
                    saw_spawn = true;
                    assert!(closure.is_none());
                    assert_eq!(m.functions[*func].name, "worker");
                }
                Instr::EnumNew { .. } => saw_enum = true,
                _ => {}
            }
        }
    }
    assert!(saw_spawn && saw_enum);
}

fn lower_graph(src: &str, tag: &str) -> lir::instr::Module {
    let dir = crate::scratch::dir(tag);
    std::fs::write(dir.join("main.rnx"), src).unwrap();
    let g = frontend::modules::ModuleGraph::build(&dir.join("main.rnx"))
        .unwrap_or_else(|e| panic!("{e}"));
    let mut m = g.resolve().unwrap_or_else(|e| panic!("{e}"));
    let d = frontend::desugar::desugar(&mut m);
    assert!(d.is_empty(), "{d:?}");
    lower::lower(&m).unwrap_or_else(|e| panic!("{e}"))
}

fn main_dyns(m: &lir::instr::Module) -> Vec<String> {
    let f = m.functions.iter().find(|f| f.name == "Main").unwrap();
    f.blocks
        .iter()
        .flat_map(|b| b.instrs.iter())
        .filter_map(|i| match i {
            lir::instr::Instr::Call {
                target: lir::instr::CallTarget::Dyn { method, .. },
                ..
            } => Some(method.clone()),
            _ => None,
        })
        .collect()
}

fn main_array_locals(m: &lir::instr::Module) -> Vec<lir::instr::LirType> {
    let f = m.functions.iter().find(|f| f.name == "Main").unwrap();
    f.locals
        .iter()
        .filter(|t| matches!(t, lir::instr::LirType::Array(_)))
        .cloned()
        .collect()
}

#[test]
fn pushed_thread_array_element_join_is_static() {
    use lir::instr::LirType;
    let m = lower_graph(
        "fn worker(): Int { return 1; }\nfn Main(): Int {\n    let hs: Array<Thread> = [];\n    hs.push(Thread.spawn((): Int => worker()));\n    let r = hs[0].join();\n    return 0;\n}\n",
        "lir-thread-array",
    );
    assert!(main_dyns(&m).is_empty(), "array element join must not stay dynamic");
    assert!(
        main_array_locals(&m).contains(&LirType::Array(Box::new(LirType::I64))),
        "bare `Thread` names the raw handle: {:?}",
        main_array_locals(&m)
    );
}

#[test]
fn imported_thread_array_element_join_is_static() {
    use lir::instr::LirType;
    let m = lower_graph(
        "import { Thread } from \"@std/sync\";\nfn worker(): Int { return 1; }\nfn Main(): Int {\n    let hs: Array<Thread> = [];\n    hs.push(Thread.spawn(worker));\n    let r = hs[0].join();\n    return 0;\n}\n",
        "lir-imported-thread-array",
    );
    assert!(main_dyns(&m).is_empty(), "imported Thread array element join must not stay dynamic");
    assert!(
        main_array_locals(&m).contains(&LirType::Array(Box::new(LirType::Obj(
            "std.sync.Thread".to_string()
        )))),
        "imported `Thread` names the class: {:?}",
        main_array_locals(&m)
    );
}

#[test]
fn class_array_element_method_is_static() {
    use lir::instr::LirType;
    let m = lower_graph(
        "class Box { let n: Int = 0; fn get(): Int { return this.n; } }\nfn Main(): Int {\n    let xs: Array<Box> = [];\n    xs.push(new Box());\n    let i = 0;\n    return xs[i].get();\n}\n",
        "lir-class-array",
    );
    assert!(main_dyns(&m).is_empty(), "class array element method must not stay dynamic");
    assert!(
        main_array_locals(&m).contains(&LirType::Array(Box::new(LirType::Obj("Box".to_string())))),
        "{:?}",
        main_array_locals(&m)
    );
}

#[test]
fn untyped_empty_array_element_call_stays_dynamic() {
    let m = lower_graph(
        "class Box { let n: Int = 0; fn get(): Int { return this.n; } }\nfn Main(): Int {\n    let xs = [];\n    xs.push(new Box());\n    return xs[0].get();\n}\n",
        "lir-untyped-array",
    );
    assert_eq!(main_dyns(&m), vec!["get".to_string()], "bare `[]` stays `Array<Any>`");
}

#[test]
fn enum_eq_compares_tags() {
    use lir::instr::{CmpOp, Instr, NumKind};
    let m = compile("enum M { Read, Write } fn F(a: M, b: M): Bool { return a == b; }");
    let f = m.functions.iter().find(|f| f.name == "F").unwrap();
    let mut saw_tag = 0;
    let mut saw_cmp = false;
    for b in &f.blocks {
        for i in &b.instrs {
            match i {
                Instr::EnumTag { .. } => saw_tag += 1,
                Instr::Cmp { op: CmpOp::Eq, kind: NumKind::Int, .. } => saw_cmp = true,
                _ => {}
            }
        }
    }
    assert_eq!(saw_tag, 2);
    assert!(saw_cmp);
}

#[test]
fn void_fn_implicit_return_terminates_cleanly() {
    let m = compile("fn Main() { print(\"hi\") }");
    let f = m.functions.iter().find(|f| f.name == "Main").unwrap();
    assert!(!f.blocks.is_empty());
    assert!(f.blocks.iter().all(|b| matches!(b.term, lir::instr::Terminator::Ret(_))));
    assert!(f.blocks.iter().all(|b| !matches!(b.term, lir::instr::Terminator::Unreachable { .. })));
}

#[test]
fn throw_terminated_fn_has_no_synthetic_trap() {
    let m = compile("fn Main() { throw 0 }");
    let f = m.functions.iter().find(|f| f.name == "Main").unwrap();
    assert!(f.blocks.iter().all(|b| !matches!(b.term, lir::instr::Terminator::Unreachable { .. })));
}

#[test]
fn missing_return_on_value_fn_fails_compile() {
    let code = lower_err("fn F(): Int { let x = 1 }");
    assert_eq!(code, diagnostics::Code::E108);
}

#[test]
fn value_fn_both_branches_return_compiles() {
    let m = compile("fn F(c: Bool): Int { if c { return 1 } else { return 2 } }");
    let f = m.functions.iter().find(|f| f.name == "F").unwrap();
    assert!(f.blocks.iter().all(|b| match &b.term {
        lir::instr::Terminator::Unreachable { span } => *span != lir::instr::UNKNOWN_SPAN,
        _ => true,
    }));
}

#[test]
fn verify_rejects_spanless_unreachable() {
    use lir::instr::{Block, Function, LirType, Terminator, UNKNOWN_SPAN};
    let f = Function {
        name: "F".to_string(),
        params: vec![],
        sig_params: vec![],
        ret: LirType::Void,
        throws: false,
        is_unsafe: false,
        method_self: false,
        is_pub: false,
        is_closure: false,
        locals: vec![],
        blocks: vec![Block { instrs: vec![], term: Terminator::Unreachable { span: UNKNOWN_SPAN } }],
    };
    let m = lir::instr::Module { functions: vec![f], ..Default::default() };
    let diags = verify::verify(&m);
    assert!(diags.iter().any(|d| d.code == diagnostics::Code::E108));
}

#[test]
fn nested_submodule_import_lowers_under_mangled_name() {
    let src = "import { statusCode, isSuccess } from \"@std/net/http\";\nfn Main(): Int { let code = statusCode(\"HTTP/1.1 200 OK\"); if (isSuccess(code)) { return code; } return 0 - 1; }\n";
    let mut m = frontend::modules::ModuleGraph::from_source(src).unwrap_or_else(|e| panic!("{e}"));
    let d = frontend::desugar::desugar(&mut m);
    assert!(d.is_empty(), "{d:?}");
    let out = lower::lower(&m).unwrap_or_else(|e| panic!("{e}"));
    let v = verify::verify(&out);
    assert!(v.is_empty(), "{v:?}");
    let names: Vec<&str> = out.functions.iter().map(|f| f.name.as_str()).collect();
    assert!(names.contains(&"std.net.http.statusCode"), "{names:?}");
    assert!(names.contains(&"std.net.http.isSuccess"), "{names:?}");
    assert!(names.contains(&"Main"), "{names:?}");
}

#[test]
fn block_void_statement_fails() {
    let code = lower_err(
        "class Box { fn addChild(c: Box) {} fn make(): Box { return this } fn noise(): Void {} } fn F(b: Box) { b.make() { b.noise() } }",
    );
    assert_eq!(code, diagnostics::Code::E108);
}

#[test]
fn block_add_child_statement_is_allowed() {
    compile(
        "class Box { fn addChild(c: Box) {} fn make(): Box { return this } } fn F(b: Box) { let c = b b.make() { b.addChild(c) } }",
    );
}

#[test]
fn collection_block_lowers_to_array() {
    let m = compile(
        r#"fn F(children: Array<Int>): Int { return children.length() }
fn Main(): Int {
  let n = F {
    1
    2
    for i in 0..3 { i }
  }
  return n
}"#,
    );
    let mut new = 0;
    let mut push = 0;
    for f in &m.functions {
        for b in &f.blocks {
            for i in &b.instrs {
                match i {
                    lir::instr::Instr::ArrayNew { .. } => new += 1,
                    lir::instr::Instr::ArrayPush { .. } => push += 1,
                    _ => {}
                }
            }
        }
    }
    assert!(new >= 1, "collection block allocates an array");
    assert!(push >= 3, "block elements and loop bodies push");
}

#[test]
fn collection_block_slots_lower() {
    let m = compile(
        r#"fn F(title: Array<Int>, children: Array<Int>): Int {
  return title.length() + children.length()
}
fn Main(): Int {
  let n = F {
    title: { 1 2 }
    3
  }
  return n
}"#,
    );
    let arrays = count_instrs(&m, |i| matches!(i, lir::instr::Instr::ArrayNew { .. }));
    let pushes = count_instrs(&m, |i| matches!(i, lir::instr::Instr::ArrayPush { .. }));
    assert_eq!(arrays, 2, "title and children each allocate an array");
    assert_eq!(pushes, 3, "two title slots plus one child");
}

#[test]
fn collection_block_rejects_wrong_element_type() {
    let mut m = frontend::parser::Parser::parse_module(
        r#"fn F(children: Array<String>): Int { return children.length() }
fn Main(): Int { let n = F { 1 } return n }"#,
    )
    .expect("parse");
    let d = frontend::desugar::desugar(&mut m);
    assert!(d.is_empty(), "{d:?}");
    assert!(lower::lower(&m).is_err(), "Int is not a String element");
}

#[test]
fn collection_block_rejects_return() {
    let mut m = frontend::parser::Parser::parse_module(
        r#"fn F(children: Array<Int>): Int { return children.length() }
fn Main(): Int { let n = F { return 1 } return n }"#,
    )
    .expect("parse");
    let d = frontend::desugar::desugar(&mut m);
    assert!(d.is_empty(), "{d:?}");
    assert!(lower::lower(&m).is_err(), "return cannot leave a collection block");
}

#[test]
fn collection_block_rejects_defer() {
    let mut m = frontend::parser::Parser::parse_module(
        r#"fn F(children: Array<Int>): Int { return children.length() }
fn Main(): Int { let n = F { defer { } 1 } return n }"#,
    )
    .expect("parse");
    let d = frontend::desugar::desugar(&mut m);
    assert!(d.is_empty(), "{d:?}");
    assert!(lower::lower(&m).is_err(), "defer cannot live in a collection block");
}

#[test]
fn collection_block_rejects_throw() {
    let mut m = frontend::parser::Parser::parse_module(
        r#"fn F(children: Array<Int>): Int { return children.length() }
fn Main(): Int { let n = F { throw 1 } return n }"#,
    )
    .expect("parse");
    let d = frontend::desugar::desugar(&mut m);
    assert!(d.is_empty(), "{d:?}");
    assert!(lower::lower(&m).is_err(), "throw cannot leave a collection block");
}

#[test]
fn collection_slots_use_their_own_element_type() {
    let m = compile(
        r#"fn F(title: Array<String>, children: Array<Int>): Int {
  return title.length() + children.length()
}
fn Main(): Int {
  let n = F {
    title: { "a" "b" }
    3
  }
  return n
}"#,
    );
    let arrays = count_instrs(&m, |i| matches!(i, lir::instr::Instr::ArrayNew { .. }));
    let pushes = count_instrs(&m, |i| matches!(i, lir::instr::Instr::ArrayPush { .. }));
    assert_eq!(arrays, 2, "title and children each allocate an array");
    assert_eq!(pushes, 3, "two title slots plus one child");
}

#[test]
fn collection_slot_rejects_wrong_element_type() {
    let mut m = frontend::parser::Parser::parse_module(
        r#"fn F(title: Array<String>, children: Array<Int>): Int {
  return title.length() + children.length()
}
fn Main(): Int { let n = F { title: { 1 } 2 } return n }"#,
    )
    .expect("parse");
    let d = frontend::desugar::desugar(&mut m);
    assert!(d.is_empty(), "{d:?}");
    assert!(lower::lower(&m).is_err(), "slot element must match its own array");
}

#[test]
fn collection_spread_rejects_boxed_any_into_concrete() {
    let mut m = frontend::parser::Parser::parse_module(
        r#"fn F(children: Array<String>): Int { return children.length() }
fn Main(): Int {
  let any: Array<Any> = [1]
  let n = F { ...any }
  return n
}"#,
    )
    .expect("parse");
    let d = frontend::desugar::desugar(&mut m);
    assert!(d.is_empty(), "{d:?}");
    assert!(lower::lower(&m).is_err(), "Array<Any> cannot be spread into Array<String>");
}

#[test]
fn collection_blocks_work_on_instance_and_this_methods() {
    let m = compile(
        r#"class C {
  fn make(children: Array<Int>): Int { return children.length() }
  fn run(): Int { return this.make { 1 2 } }
}
fn Main(): Int {
  let c = new C()
  let n = c.make { 1 2 3 }
  return n + c.run()
}"#,
    );
    assert!(m.functions.iter().any(|f| f.name.contains("C.make")));
}

#[test]
fn collection_slot_rejects_non_array_parameter() {
    let mut m = frontend::parser::Parser::parse_module(
        r#"fn F(title: String, children: Array<Int>): Int { return children.length() }
fn Main(): Int { let n = F { title: { "a" } 1 } return n }"#,
    )
    .expect("parse");
    let d = frontend::desugar::desugar(&mut m);
    assert!(d.is_empty(), "{d:?}");
    assert!(lower::lower(&m).is_err(), "a slot must name an array parameter");
}

#[test]
fn collection_slot_rejects_children_duplicate() {
    let mut m = frontend::parser::Parser::parse_module(
        r#"fn F(children: Array<Int>): Int { return children.length() }
fn Main(): Int { let n = F { children: { 1 } 2 } return n }"#,
    )
    .expect("parse");
    let d = frontend::desugar::desugar(&mut m);
    assert!(d.is_empty(), "{d:?}");
    assert!(lower::lower(&m).is_err(), "children cannot be both unnamed and named");
}

#[test]
fn collection_block_rejects_generic_array_parameter() {
    let mut m = frontend::parser::Parser::parse_module(
        r#"fn F<T>(children: Array<T>): Int { return children.length() }
fn Main(): Int { let n = F { 1 } return n }"#,
    )
    .expect("parse");
    let d = frontend::desugar::desugar(&mut m);
    assert!(d.is_empty(), "{d:?}");
    assert!(lower::lower(&m).is_err(), "generic collection arrays are rejected");
}

#[test]
fn collection_block_rejects_throwing_calls() {
    let mut m = frontend::parser::Parser::parse_module(
        r#"fn risky(): Int throws { throw 1 }
fn F(children: Array<Int>): Int { return children.length() }
fn Main(): Int { let n = F { risky() } return n }"#,
    )
    .expect("parse");
    let d = frontend::desugar::desugar(&mut m);
    assert!(d.is_empty(), "{d:?}");
    assert!(lower::lower(&m).is_err(), "throwing calls are rejected in collection blocks");
}

#[test]
fn collection_block_allows_non_throwing_local_closure() {
    let m = compile(
        r#"fn F(children: Array<Any>): Int { return children.length() }
fn Main(): Int {
  let f: fn(Int): Int = (x: Int): Int => x + 1
  let n = F { f(1) }
  return n
}"#,
    );
    assert!(m.functions.iter().any(|f| f.name.contains("F")));
}

#[test]
fn collection_block_generic_non_array_uses_legacy_child_sink() {
    let m = compile(
        r#"class Box {
  fn addChild(c: Box) {}
}
fn wrap<T>(v: T): Box { return new Box() }
fn Main(): Int {
  let b = wrap(1) { new Box() }
  return 0
}"#,
    );
    assert!(m.functions.iter().any(|f| f.name.contains("wrap")));
}

#[test]
fn collection_block_rejects_throwing_constructor() {
    let mut m = frontend::parser::Parser::parse_module(
        r#"class C { init() { throw 1 } }
fn F(children: Array<Int>): Int { return children.length() }
fn Main(): Int { let n = F { new C() } return n }"#,
    )
    .expect("parse");
    let d = frontend::desugar::desugar(&mut m);
    assert!(d.is_empty(), "{d:?}");
    assert!(lower::lower(&m).is_err(), "throwing constructors are rejected");
}

#[test]
fn collection_block_rejects_break_leaving_the_block() {
    let mut m = frontend::parser::Parser::parse_module(
        r#"fn F(children: Array<Int>): Int { return children.length() }
fn Main(): Int {
  while true {
    let n = F { break }
    return n
  }
  return 0
}"#,
    )
    .expect("parse");
    let d = frontend::desugar::desugar(&mut m);
    assert!(d.is_empty(), "{d:?}");
    assert!(lower::lower(&m).is_err(), "break cannot leave a collection block");
}

#[test]
fn collection_any_box_releases_temporaries() {
    let m = compile(
        r#"fn F(children: Array<Any>): Int { return children.length() }
fn Main(): Int {
  let n = F { 1 2 3 }
  return n
}"#,
    );
    let f = m
        .functions
        .iter()
        .find(|f| f.name.contains("Main"))
        .expect("Main lowered");
    let releases = f
        .blocks
        .iter()
        .flat_map(|b| b.instrs.iter())
        .filter(|i| matches!(i, lir::instr::Instr::Release { .. }))
        .count();
    assert!(releases >= 3, "boxed Any elements release their temporaries");
}

#[test]
fn collection_block_rejects_throwing_extension_method() {
    let mut m = frontend::parser::Parser::parse_module(
        r#"class Box { init() {} }
extension Box {
  fn boom(): Int throws { throw 1 }
}
fn F(children: Array<Int>): Int { return children.length() }
fn Main(): Int { let b = new Box(); let n = F { b.boom() }; return n }"#,
    )
    .expect("parse");
    let d = frontend::desugar::desugar(&mut m);
    assert!(d.is_empty(), "{d:?}");
    assert!(lower::lower(&m).is_err(), "throwing extensions are rejected");
}

#[test]
fn collection_block_allows_non_throwing_extension_method() {
    let m = compile(
        r#"class Box { init() {} }
extension Box {
  fn size(): Int { return 7 }
}
fn F(children: Array<Int>): Int { return children.length() }
fn Main(): Int { let b = new Box(); let n = F { b.size() }; return n }"#,
    );
    assert!(m.functions.iter().any(|f| f.name.contains("Main")));
}

#[test]
fn collection_block_rejects_throwing_optional_call() {
    let mut m = frontend::parser::Parser::parse_module(
        r#"class Box {
  init() {}
  fn boom(): Int throws { throw 1 }
}
fn F(children: Array<Int>): Int { return children.length() }
fn Main(): Int { let b: Box? = new Box(); let n = F { b?.boom() }; return n }"#,
    )
    .expect("parse");
    let d = frontend::desugar::desugar(&mut m);
    assert!(d.is_empty(), "{d:?}");
    assert!(lower::lower(&m).is_err(), "throwing optional calls are rejected");
}

#[test]
fn collection_block_rejects_throwing_super_call() {
    let mut m = frontend::parser::Parser::parse_module(
        r#"class Base {
  init() {}
  fn go(): Int throws { throw 1 }
}
class Sub extends Base {
  init() { super() }
  fn run(): Int { return 1 }
}
fn F(children: Array<Int>): Int { return children.length() }
fn Main(): Int { let s = new Sub(); let n = F { s.go() }; return n }"#,
    )
    .expect("parse");
    let d = frontend::desugar::desugar(&mut m);
    assert!(d.is_empty(), "{d:?}");
    assert!(lower::lower(&m).is_err(), "throwing inherited calls are rejected");
}

#[test]
fn collection_spread_lowers_into_the_target_array() {
    let m = compile(
        r#"fn F(children: Array<Int>): Int { return children.length() }
fn Main(): Int {
  let xs: Array<Int> = [1, 2]
  let n = F { ...xs 3 }
  return n
}"#,
    );
    let lens = count_instrs(&m, |i| matches!(i, lir::instr::Instr::ArrayLen { .. }));
    let pushes = count_instrs(&m, |i| matches!(i, lir::instr::Instr::ArrayPush { .. }));
    assert!(lens >= 1, "spread reads the source length");
    assert!(pushes >= 3, "spread loop and explicit element push into the block array");
}

#[test]
fn collection_block_rejects_throwing_interface_method() {
    let mut m = frontend::parser::Parser::parse_module(
        r#"interface Op { fn run(): Int throws; }
class Add : Op {
  init() {}
  fn run(): Int throws { throw 1 }
}
fn F(children: Array<Int>): Int { return children.length() }
fn Main(): Int { let o: Op = new Add(); let n = F { o.run() }; return n }"#,
    )
    .expect("parse");
    let d = frontend::desugar::desugar(&mut m);
    assert!(d.is_empty(), "{d:?}");
    assert!(lower::lower(&m).is_err(), "throwing interface calls are rejected");
}

#[test]
fn collection_block_allows_non_throwing_interface_method() {
    let m = compile(
        r#"interface Op { fn run(): Int; }
class Add : Op {
  init() {}
  fn run(): Int { return 1 }
}
fn F(children: Array<Int>): Int { return children.length() }
fn Main(): Int { let o: Op = new Add(); let n = F { o.run() }; return n }"#,
    );
    assert!(m.functions.iter().any(|f| f.name.contains("Main")));
}

#[test]
fn function_shadows_enum_variant_in_call_position() {
    let m = compile(
        r#"enum E { screen, other }
fn screen(a: Int, b: Int): Int { return a + b }
fn Main(): Int { return screen(1, 2) }"#,
    );
    assert!(m.functions.iter().any(|f| f.name == "screen"));
}

#[test]
fn bare_enum_variant_still_lowers() {
    let m = compile(
        r#"enum E { screen, other }
fn Main(): Int { let e: E = E.screen; return 0 }"#,
    );
    assert!(m.functions.iter().any(|f| f.name == "Main"));
}

#[test]
fn class_dtor_releases_owned_fields() {
    let m = compile(
        "class Inner { let n: Int = 1 } class Outer { let i: Inner = new Inner() } fn Main(): Int { let o = new Outer(); return 0 }",
    );
    let dtor = m
        .functions
        .iter()
        .find(|f| f.name == "__dtor_Outer")
        .expect("Outer dtor");
    let releases = dtor
        .blocks
        .iter()
        .flat_map(|b| b.instrs.iter())
        .filter(|i| matches!(i, lir::instr::Instr::ReleaseField { .. }))
        .count();
    assert_eq!(releases, 1, "Outer dtor must release its Inner field");
}
