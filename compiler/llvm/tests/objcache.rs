use llvm::codegen::{
    ObjectCache, OptLevel, cache_key, emit_object, emit_object_cached, emit_objects,
    emit_objects_cached, profile_str, unit_hash,
};
use std::cell::RefCell;
use std::collections::HashMap;

fn lowered(src: &str) -> lir::instr::Module {
    let mut module = frontend::modules::ModuleGraph::from_source(src).unwrap();
    assert!(frontend::desugar::desugar(&mut module).is_empty());
    lir::lower::lower(&module).unwrap()
}

struct MemStore {
    map: RefCell<HashMap<String, Vec<u8>>>,
    lookups: RefCell<usize>,
    stores: RefCell<usize>,
}

impl MemStore {
    fn new() -> Self {
        Self {
            map: RefCell::new(HashMap::new()),
            lookups: RefCell::new(0),
            stores: RefCell::new(0),
        }
    }
    fn reset_counts(&self) {
        *self.lookups.borrow_mut() = 0;
        *self.stores.borrow_mut() = 0;
    }
}

macro_rules! mem_cache {
    ($mem:expr, $m:expr, $t:expr) => {
        ObjectCache {
            module_hash: $m,
            toolchain_hash: $t,
            lookup: &|k: &str| {
                *$mem.lookups.borrow_mut() += 1;
                $mem.map.borrow().get(k).cloned()
            },
            store: &|k: &str, v: &[u8]| {
                *$mem.stores.borrow_mut() += 1;
                $mem.map.borrow_mut().insert(k.to_string(), v.to_vec());
            },
        }
    };
}

fn is_elf(bytes: &[u8]) -> bool {
    bytes.len() > 4 && bytes[0] == 0x7f && bytes[1] == b'E' && bytes[2] == b'L' && bytes[3] == b'F'
}

#[test]
fn profile_and_key_shape() {
    assert_eq!(profile_str(OptLevel::Dev), "dev");
    assert_eq!(profile_str(OptLevel::Release), "release");
    assert_eq!(cache_key("dev", "m", "t", "u"), "dev/m/t/u");
}

#[test]
fn unit_hash_stable_and_sensitive() {
    let src = "fn Add(a: Int, b: Int): Int { return a + b; } fn Main(): Int { return Add(2, 3); }";
    let lir = lowered(src);
    let a = unit_hash(&lir, None, "Main", OptLevel::Dev, None, None, true, "h");
    assert_eq!(a.len(), 64);
    assert_eq!(a, unit_hash(&lir, None, "Main", OptLevel::Dev, None, None, true, "h"));
    let other = lowered("fn Add(a: Int, b: Int): Int { return a - b; } fn Main(): Int { return Add(2, 3); }");
    assert_ne!(a, unit_hash(&other, None, "Main", OptLevel::Dev, None, None, true, "h"));
    assert_ne!(
        a,
        unit_hash(&lir, None, "Main", OptLevel::Release, None, None, true, "h")
    );
}

#[test]
fn single_cached_matches_fresh_and_skips_reemit() {
    let src = "fn Add(a: Int, b: Int): Int { return a + b; } fn Main(): Int { return Add(2, 3); }";
    let lir = lowered(src);
    let fresh = emit_object(&lir, "obj_single", "Main", OptLevel::Dev, None).unwrap();
    assert!(is_elf(&fresh));
    let mem = MemStore::new();
    let got = emit_object_cached(&lir, "obj_single", "Main", OptLevel::Dev, None, None, &mem_cache!(mem, "m", "t")).unwrap();
    assert_eq!(got, fresh);
    assert_eq!(*mem.lookups.borrow(), 1);
    assert_eq!(*mem.stores.borrow(), 1);
    mem.reset_counts();
    let again = emit_object_cached(&lir, "obj_single", "Main", OptLevel::Dev, None, None, &mem_cache!(mem, "m", "t")).unwrap();
    assert_eq!(again, fresh);
    assert_eq!(*mem.lookups.borrow(), 1);
    assert_eq!(*mem.stores.borrow(), 0);
}

#[test]
fn release_single_cached_matches_fresh() {
    let src = "fn Add(a: Int, b: Int): Int { return a + b; } fn Main(): Int { return Add(2, 3); }";
    let lir = lowered(src);
    let fresh = emit_object(&lir, "obj_rel", "Main", OptLevel::Release, None).unwrap();
    assert!(is_elf(&fresh));
    let mem = MemStore::new();
    let got = emit_object_cached(&lir, "obj_rel", "Main", OptLevel::Release, None, None, &mem_cache!(mem, "m", "t")).unwrap();
    assert_eq!(got, fresh);
}

#[test]
fn multi_unit_cached_matches_fresh() {
    let mut src = String::new();
    for i in 0..70 {
        src.push_str(&format!("fn CchH{i:03}(): Int {{ return {i}; }} "));
    }
    src.push_str("fn Main(): Int { return CchH000() + CchH069(); } ");
    let lir = lowered(&src);
    assert!(lir.functions.len() >= 64);
    let fresh = emit_objects(&lir, "obj_multi", "Main", OptLevel::Dev, None, None, 4).unwrap();
    assert_eq!(fresh.len(), 4);
    let mem = MemStore::new();
    let got = emit_objects_cached(&lir, "obj_multi", "Main", OptLevel::Dev, None, None, 4, Some(&mem_cache!(mem, "m", "t"))).unwrap();
    assert_eq!(got, fresh);
    assert_eq!(*mem.stores.borrow(), 4);
    mem.reset_counts();
    let again = emit_objects_cached(&lir, "obj_multi", "Main", OptLevel::Dev, None, None, 4, Some(&mem_cache!(mem, "m", "t"))).unwrap();
    assert_eq!(again, fresh);
    assert_eq!(*mem.lookups.borrow(), 4);
    assert_eq!(*mem.stores.borrow(), 0);
}

#[test]
fn full_miss_falls_back_to_emit() {
    let mut src = String::new();
    for i in 0..70 {
        src.push_str(&format!("fn CmsH{i:03}(): Int {{ return {i}; }} "));
    }
    src.push_str("fn Main(): Int { return CmsH000(); } ");
    let lir = lowered(&src);
    let fresh = emit_objects(&lir, "obj_miss", "Main", OptLevel::Dev, None, None, 4).unwrap();
    let mem = MemStore::new();
    let lookup_none = |_: &str| None;
    let sink = |_: &str, _: &[u8]| {};
    let cache = ObjectCache {
        module_hash: "m",
        toolchain_hash: "t",
        lookup: &lookup_none,
        store: &sink,
    };
    let got = emit_objects_cached(&lir, "obj_miss", "Main", OptLevel::Dev, None, None, 4, Some(&cache)).unwrap();
    assert_eq!(got, fresh);
    assert!(mem.map.borrow().is_empty());
}

#[test]
fn no_cache_is_emit_objects() {
    let src = "fn Main(): Int { return 1; }";
    let lir = lowered(src);
    let a = emit_objects(&lir, "obj_plain", "Main", OptLevel::Dev, None, None, 4).unwrap();
    let b = emit_objects_cached(&lir, "obj_plain", "Main", OptLevel::Dev, None, None, 4, None).unwrap();
    assert_eq!(a, b);
}
