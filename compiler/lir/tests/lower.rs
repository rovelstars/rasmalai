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
    let dir = std::env::temp_dir().join(format!("rnx-lir-thr-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
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
    let _ = std::fs::remove_dir_all(&dir);
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
