use llvm::codegen::{ObjectCache, OptLevel, emit_object, emit_objects, emit_objects_cached};

fn lowered(src: &str) -> lir::instr::Module {
    let mut module = frontend::modules::ModuleGraph::from_source(src).unwrap();
    assert!(frontend::desugar::desugar(&mut module).is_empty());
    lir::lower::lower(&module).unwrap()
}

fn tiny_module() -> lir::instr::Module {
    use lir::instr::{Block, Function, LirType, Module, Terminator};
    let mut module = Module::default();
    for name in ["TinyA", "TinyB", "TinyMain"] {
        module.functions.push(Function {
            name: name.to_string(),
            params: Vec::new(),
            sig_params: Vec::new(),
            ret: LirType::I64,
            throws: false,
            is_unsafe: false,
            method_self: false,
            is_pub: false,
            is_closure: false,
            locals: Vec::new(),
            blocks: vec![Block { instrs: Vec::new(), term: Terminator::Ret(Vec::new()) }],
        });
    }
    module
}

fn is_elf(bytes: &[u8]) -> bool {
    bytes.len() > 4 && bytes[0] == 0x7f && bytes[1] == b'E' && bytes[2] == b'L' && bytes[3] == b'F'
}

fn contains(bytes: &[u8], s: &str) -> bool {
    bytes.windows(s.len()).any(|w| w == s.as_bytes())
}

fn index_of(lir: &lir::instr::Module, name: &str) -> usize {
    lir.functions.iter().position(|f| f.name == name).unwrap()
}

#[test]
fn single_unit_matches_emit_object() {
    let src = "fn Add(a: Int, b: Int): Int { return a + b; } fn Main(): Int { return Add(2, 3); }";
    let lir = lowered(src);
    let single = emit_object(&lir, "cgu_single", "Main", OptLevel::Dev, None).unwrap();
    assert!(is_elf(&single));
    let one = emit_objects(&lir, "cgu_single", "Main", OptLevel::Dev, None, None, 1).unwrap();
    assert_eq!(one.len(), 1);
    assert_eq!(one[0], single);
    let zero = emit_objects(&lir, "cgu_single", "Main", OptLevel::Dev, None, None, 0).unwrap();
    assert_eq!(zero.len(), 1);
    assert_eq!(zero[0], single);
}

#[test]
fn below_threshold_falls_back_to_single_object() {
    let lir = tiny_module();
    assert!(lir.functions.len() < 64);
    let single = emit_object(&lir, "cgu_tiny", "TinyMain", OptLevel::Dev, None).unwrap();
    let out = emit_objects(&lir, "cgu_tiny", "TinyMain", OptLevel::Dev, None, None, 8).unwrap();
    assert_eq!(out.len(), 1);
    assert_eq!(out[0], single);
}

#[test]
fn multi_unit_objects_are_valid_elf() {
    let mut src = String::new();
    for i in 0..200 {
        src.push_str(&format!("fn CguH{i:03}(): Int {{ return {i}; }} "));
    }
    src.push_str("fn Main(): Int { return CguH000() + CguH199(); } ");
    let lir = lowered(&src);
    assert!(lir.functions.len() >= 64);
    let names: Vec<String> = lir.functions.iter().map(|f| f.name.clone()).collect();
    let chunk = names.len().div_ceil(2);
    let main_unit = index_of(&lir, "Main") / chunk;
    let low_unit = index_of(&lir, "CguH000") / chunk;
    assert_ne!(low_unit, main_unit, "test needs Main and CguH000 in different units");

    let first = emit_objects(&lir, "cgu_multi", "Main", OptLevel::Dev, None, None, 2).unwrap();
    assert_eq!(first.len(), 2);
    for obj in &first {
        assert!(!obj.is_empty());
        assert!(is_elf(obj));
    }
    assert_ne!(first[0], first[1]);
    assert!(contains(&first[0], "rnx_entry_main"));
    for (i, name) in names.iter().enumerate() {
        assert!(contains(&first[i / chunk], name), "unit misses {name}");
    }
    let main_unit = index_of(&lir, "Main") / chunk;
    assert!(contains(&first[main_unit], "CguH000"));
    assert!(contains(&first[main_unit], "CguH199"));

    let second = emit_objects(&lir, "cgu_multi", "Main", OptLevel::Dev, None, None, 2).unwrap();
    assert_eq!(first, second);

    let wide = emit_objects(&lir, "cgu_multi", "Main", OptLevel::Dev, None, None, 8).unwrap();
    assert_eq!(wide.len(), 8);
    for obj in &wide {
        assert!(!obj.is_empty());
        assert!(is_elf(obj));
    }
}

#[test]
fn multi_unit_main_entry_renamed_everywhere() {
    let mut src = String::new();
    for i in 0..200 {
        src.push_str(&format!("fn CguH{i:03}(): Int {{ return {i}; }} "));
    }
    src.push_str("fn main(): Int { return CguH199(); } ");
    let lir = lowered(&src);
    assert!(lir.functions.len() >= 64);
    let chunk = lir.functions.len().div_ceil(2);
    let def_unit = index_of(&lir, "main") / chunk;
    assert_ne!(def_unit, 0, "test needs user main outside unit 0");

    let out = emit_objects(&lir, "cgu_main", "main", OptLevel::Dev, None, None, 2).unwrap();
    assert_eq!(out.len(), 2);
    for obj in &out {
        assert!(!obj.is_empty());
        assert!(is_elf(obj));
    }
    assert!(contains(&out[0], "rnx_entry_main"));
    assert!(contains(&out[def_unit], "__rnx_user_main"));
    assert!(contains(&out[0], "__rnx_user_main"));
}

fn big_module() -> lir::instr::Module {
    let mut src = String::new();
    for i in 0..200 {
        src.push_str(&format!("fn CguD{i:03}(): Int {{ return {i} * 3 + 1; }} "));
    }
    src.push_str("fn Main(): Int { return CguD000() + CguD199(); } ");
    let lir = lowered(&src);
    assert!(lir.functions.len() >= 64);
    lir
}

// Units reuse pooled contexts with one machine each; every repeated
// parallel emission must still produce byte-identical objects, on both
// the unoptimized and the O3 paths.
#[test]
fn shared_setup_byte_identical_across_runs() {
    let lir = big_module();
    let first = emit_objects(&lir, "cgu_det", "Main", OptLevel::Dev, None, None, 8).unwrap();
    assert_eq!(first.len(), 8);
    for obj in &first {
        assert!(is_elf(obj));
    }
    for _ in 0..2 {
        let again = emit_objects(&lir, "cgu_det", "Main", OptLevel::Dev, None, None, 8).unwrap();
        assert_eq!(again, first);
    }
    let rel_first = emit_objects(&lir, "cgu_det", "Main", OptLevel::Release, None, None, 8).unwrap();
    assert_eq!(rel_first.len(), 8);
    for obj in &rel_first {
        assert!(is_elf(obj));
    }
    let rel_again = emit_objects(&lir, "cgu_det", "Main", OptLevel::Release, None, None, 8).unwrap();
    assert_eq!(rel_again, rel_first);

    use std::cell::RefCell;
    use std::collections::HashMap;
    let map: RefCell<HashMap<String, Vec<u8>>> = RefCell::new(HashMap::new());
    let cache = ObjectCache {
        module_hash: "m",
        toolchain_hash: "t",
        lookup: &|k: &str| map.borrow().get(k).cloned(),
        store: &|k: &str, v: &[u8]| {
            map.borrow_mut().insert(k.to_string(), v.to_vec());
        },
    };
    let miss = emit_objects_cached(&lir, "cgu_det", "Main", OptLevel::Dev, None, None, 8, Some(&cache))
        .unwrap();
    assert_eq!(miss, first);
    let hit = emit_objects_cached(&lir, "cgu_det", "Main", OptLevel::Dev, None, None, 8, Some(&cache))
        .unwrap();
    assert_eq!(hit, first);
}

// Concurrent emissions share the process-wide context pool; pooled
// contexts are handed out owned, never aliased, so parallel builds
// agree byte-for-byte with a sequential one.
#[test]
fn concurrent_builds_share_pool_safely() {
    let lir = big_module();
    let seq = emit_objects(&lir, "cgu_race", "Main", OptLevel::Dev, None, None, 4).unwrap();
    std::thread::scope(|s| {
        let mut handles = Vec::new();
        for _ in 0..8 {
            handles.push(s.spawn(|| {
                emit_objects(&lir, "cgu_race", "Main", OptLevel::Dev, None, None, 4).unwrap()
            }));
        }
        for h in handles {
            assert_eq!(h.join().unwrap(), seq);
        }
    });
}
