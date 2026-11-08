use lir::instr::{Instr, Local};

fn compile(src: &str) -> lir::instr::Module {
    let mut m = frontend::parser::Parser::parse_module(src).unwrap_or_else(|e| panic!("{e}"));
    let d = frontend::desugar::desugar(&mut m);
    assert!(d.is_empty(), "{d:?}");
    let out = lir::lower::lower(&mut m).unwrap_or_else(|e| panic!("{e}"));
    let v = lir::verify::verify(&out);
    assert!(v.is_empty(), "{v:?}");
    out
}

fn optimize(mut m: lir::instr::Module) -> lir::instr::Module {
    lir::opt::optimize_lir(&mut m, 1, "Main");
    let v = lir::verify::verify(&m);
    assert!(v.is_empty(), "{v:?}");
    m
}

fn main_func(m: &lir::instr::Module) -> &lir::instr::Function {
    m.functions.iter().find(|f| f.name == "Main").expect("Main")
}

fn releases(func: &lir::instr::Function) -> Vec<Local> {
    func.blocks
        .iter()
        .flat_map(|b| b.instrs.iter())
        .filter_map(|i| match i {
            Instr::Release { obj, .. } => Some(*obj),
            _ => None,
        })
        .collect()
}

fn objnew_of(func: &lir::instr::Function, class: &str) -> Vec<Local> {
    func.blocks
        .iter()
        .flat_map(|b| b.instrs.iter())
        .filter_map(|i| match i {
            Instr::ObjNew { dst, .. } => Some(*dst),
            _ => None,
        })
        .filter(|d| {
            matches!(
                func.locals.get(*d as usize),
                Some(lir::instr::LirType::Obj(n)) if n.contains(class)
            )
        })
        .collect()
}

fn getfield_dsts(func: &lir::instr::Function) -> Vec<Local> {
    func.blocks
        .iter()
        .flat_map(|b| b.instrs.iter())
        .filter_map(|i| match i {
            Instr::GetField { dst, .. } | Instr::GetFieldByName { dst, .. } => Some(*dst),
            _ => None,
        })
        .collect()
}

fn defines(ins: &Instr, local: Local) -> bool {
    match ins {
        Instr::Call { dsts, err, .. } => {
            dsts.contains(&local) || *err == Some(local)
        }
        Instr::Copy { dst, .. }
        | Instr::Cast { dst, .. }
        | Instr::GetField { dst, .. }
        | Instr::GetFieldByName { dst, .. }
        | Instr::ObjNew { dst, .. } => *dst == local,
        _ => false,
    }
}

fn def_count(func: &lir::instr::Function, local: Local) -> usize {
    func.blocks
        .iter()
        .flat_map(|b| b.instrs.iter())
        .filter(|ins| defines(ins, local))
        .count()
}

const NESTED: &str = r#"class D {
    let x: Int = 1
}
class F {
    let d: D
    init(d: D) { this.d = d }
}
fn Main(): Int {
    let i = 0
    while i < 3 {
        new F(new D())
        i += 1
    }
    return 0
}
"#;

#[test]
fn nested_inner_temp_gets_a_release() {
    let m = optimize(compile(NESTED));
    let main = main_func(&m);
    let inner = objnew_of(main, "D");
    assert!(!inner.is_empty(), "inner ObjNew lost");
    let rel = releases(main);
    for d in inner {
        assert!(rel.contains(&d), "inner temp {d} never released");
    }
}

const RETURNED: &str = r#"class Outer {
    let a: Int = 1
    deinit { print("outer-drop") }
}
fn makeOuter(): Outer {
    print("making")
    return new Outer()
}
fn Main(): Int {
    let o = makeOuter()
    let r = o.a
    print("out")
    return r
}
"#;

#[test]
fn returned_temp_has_no_early_release() {
    let m = optimize(compile(RETURNED));
    let made = m
        .functions
        .iter()
        .find(|f| f.name == "makeOuter")
        .expect("makeOuter missing");
    let rel = releases(made);
    let obj = objnew_of(made, "Outer");
    assert!(!obj.is_empty(), "Outer ObjNew lost");
    for o in obj {
        assert!(
            !rel.contains(&o),
            "returned temp {o} released before return"
        );
    }
}

const PINNED: &str = r#"class Buf {
    let handle: Int = 7
    fn address(): Int {
        print("addr")
        return this.handle
    }
    deinit { print("buf-drop") }
}
fn sink(k: Int): Int { return k }
fn Main(): Int {
    let b = new Buf()
    let k = b.address()
    return sink(k)
}
"#;

#[test]
fn int_derived_buffer_temp_is_pinned() {
    let m = optimize(compile(PINNED));
    let main = main_func(&m);
    let bufs = objnew_of(main, "Buf");
    assert!(!bufs.is_empty(), "Buf ObjNew lost");
    // The buffer is only read to derive an Int, and function-scope
    // temporaries have no scope release, so any Release here would be the
    // death pass freeing a buffer whose derived address may still be live
    // (the real ByteBuffer.address case). Nothing in Main may be released.
    assert!(
        releases(main).is_empty(),
        "death pass released despite a live derived address: {:?}",
        releases(main)
    );
}

const RECEIVER: &str = r#"class Leaf {
    let n: Int = 0
    fn value(): Int { return this.n }
}
class Holder {
    let leaf: Leaf
    init() {
        let l = new Leaf()
        this.leaf = l
    }
}
fn Main(): Int {
    let h = new Holder()
    if h.leaf.value() == 0 {
        return 1
    }
    return 0
}
"#;

#[test]
fn field_load_temps_are_never_released() {
    let m = optimize(compile(RECEIVER));
    let main = main_func(&m);
    let rel = releases(main);
    let loads = getfield_dsts(main);
    assert!(!loads.is_empty(), "no field loads found");
    for g in loads {
        if def_count(main, g) != 1 {
            continue;
        }
        assert!(
            !rel.contains(&g),
            "field load temp {g} released by the death pass"
        );
    }
}
