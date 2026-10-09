use lir::instr::{ArithOp, Instr};
use runtime::machine::Machine;
use runtime::value::Value;

fn lowered(src: &str) -> lir::instr::Module {
    let mut module = frontend::modules::ModuleGraph::from_source(src).expect("parse");
    assert!(frontend::desugar::desugar(&mut module).is_empty());
    lir::lower::lower(&module).expect("lower")
}

const VERSIONED_SRC: &str = "fn Main(n: Int): Int {\n    let arr: Array<Int> = [10, 20, 30, 40];\n    let s = 0;\n    let i = 0;\n    while i < n {\n        s = s + arr[i];\n        i = i + 1;\n    }\n    return s;\n}\n";

fn has_bool_bitand(module: &lir::instr::Module) -> bool {
    for f in &module.functions {
        let mut cmp_defs = std::collections::BTreeSet::new();
        for b in &f.blocks {
            for ins in &b.instrs {
                if let Instr::Cmp { dst, .. } = ins {
                    cmp_defs.insert(*dst);
                }
            }
        }
        for b in &f.blocks {
            for ins in &b.instrs {
                if let Instr::Arith {
                    op: ArithOp::BitAnd,
                    lhs,
                    rhs,
                    ..
                } = ins
                {
                    if cmp_defs.contains(lhs) && cmp_defs.contains(rhs) {
                        return true;
                    }
                }
            }
        }
    }
    false
}

#[test]
fn bce_guard_conjunction_runs_on_interpreter() {
    let mut module = lowered(VERSIONED_SRC);
    lir::opt::optimize_lir(&mut module, 1, "Main");
    let diags = lir::verify::verify(&module);
    assert!(diags.is_empty(), "{diags:?}");
    assert!(
        has_bool_bitand(&module),
        "BCE versioning did not fire; repro shape absent"
    );
    let leaked: &'static lir::instr::Module = Box::leak(Box::new(module));
    let mut machine = Machine::new(leaked);
    match machine.call("Main", vec![Value::Int(4)]) {
        Ok(v) => assert_eq!(v, Value::Int(100)),
        Err(e) => panic!("interpreter failed on BCE guard conjunction: {e:?}"),
    }
}

#[test]
fn bce_guard_slow_path_still_traps() {
    let mut module = lowered(VERSIONED_SRC);
    lir::opt::optimize_lir(&mut module, 1, "Main");
    assert!(has_bool_bitand(&module), "repro shape absent");
    let leaked: &'static lir::instr::Module = Box::leak(Box::new(module));
    let mut machine = Machine::new(leaked);
    match machine.call("Main", vec![Value::Int(10)]) {
        Ok(v) => panic!("expected out-of-bounds trap, got {v:?}"),
        Err(runtime::machine::ExecError::Throw(_)) => {}
        Err(runtime::machine::ExecError::Fatal(m)) => {
            panic!("slow path must trap, not fatal: {m}")
        }
    }
}
