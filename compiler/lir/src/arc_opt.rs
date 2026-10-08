use crate::instr::*;
use crate::licm::DominatorTree;
use crate::opt::instr_dst;
use std::collections::{BTreeMap, BTreeSet};

fn union_alias(alias: &mut BTreeMap<Local, Local>, a: Local, b: Local) {
    let ra = find_root(alias, a);
    let rb = find_root(alias, b);
    if ra != rb {
        alias.insert(ra, rb);
    }
}

fn find_root(alias: &BTreeMap<Local, Local>, mut x: Local) -> Local {
    while let Some(p) = alias.get(&x) {
        if *p == x {
            break;
        }
        x = *p;
    }
    x
}

fn collect_aliases(func: &Function) -> BTreeMap<Local, Local> {
    let mut alias: BTreeMap<Local, Local> = BTreeMap::new();
    for block in &func.blocks {
        for ins in &block.instrs {
            if let Instr::Copy {  dst, src , ..} | Instr::Cast { dst, src , ..} = ins {
                union_alias(&mut alias, *dst, *src);
            }
        }
    }
    alias
}

fn trivial_class(module: &Module, ty: &LirType) -> bool {
    match ty {
        LirType::Obj(name) => match module.class_index.get(name) {
            Some(ci) => match module.classes.get(*ci) {
                Some(c) => {
                    c.deinit.is_none()
                        && match c.dtor {
                            None => true,
                            Some(id) => is_trivial_dtor(module, id),
                        }
                }
                None => false,
            },
            None => false,
        },
        LirType::Enum(ei) => match module.enums.get(*ei) {
            Some(e) => match e.dtor {
                None => true,
                Some(id) => is_trivial_dtor(module, id),
            },
            None => false,
        },
        LirType::Array(_) => false,
        _ => true,
    }
}

fn is_trivial_dtor(module: &Module, id: usize) -> bool {
    let func = match module.functions.get(id) {
        Some(f) => f,
        None => return false,
    };
    func.blocks.iter().all(|b| {
        b.instrs.iter().all(|i| matches!(i, Instr::GenRefInvalidate { ..}))
            && matches!(b.term, Terminator::Ret(_) | Terminator::Br(_))
    })
}

fn local_type(func: &Function, local: Local) -> LirType {
    func.locals.get(local as usize).cloned().unwrap_or(LirType::Any)
}

fn tainted_roots(
    func: &Function,
    alias: &BTreeMap<Local, Local>,
) -> (BTreeSet<Local>, BTreeSet<Local>) {
    let mut genref: BTreeSet<Local> = BTreeSet::new();
    let mut returned: BTreeSet<Local> = BTreeSet::new();
    for block in &func.blocks {
        for ins in &block.instrs {
            if let Instr::GenRefOf {  obj , ..} = ins {
                genref.insert(find_root(alias, *obj));
            }
        }
        match &block.term {
            Terminator::Ret(v) => {
                for l in v {
                    returned.insert(find_root(alias, *l));
                }
            }
            Terminator::Throw { src, .. } => {
                returned.insert(find_root(alias, *src));
            }
            Terminator::Rethrow { err, .. } => {
                returned.insert(find_root(alias, *err));
            }
            _ => {}
        }
    }
    (genref, returned)
}

fn invalidate_barrier(active: &mut BTreeMap<Local, usize>, ins: &Instr) {
    match ins {
        Instr::Call { .. }
        | Instr::ThreadSpawn { .. }
        | Instr::PoolInit { .. }
        | Instr::PoolSubmit { .. }
        | Instr::PoolParallelFor { .. }
        | Instr::PoolJoin { .. }
        | Instr::PoolShutdown { .. }
        | Instr::Defer { .. } => active.clear(),
        Instr::SetField { obj, value, .. } | Instr::SetFieldByName { obj, value, .. } => {
            active.remove(obj);
            active.remove(value);
        }
        Instr::ArrayPush { value, .. } => {
            active.remove(value);
        }
        Instr::ArraySet { value, .. } => {
            active.remove(value);
        }
        Instr::ClosureNew { captures, .. } => {
            for c in captures {
                active.remove(c);
            }
        }
        _ => {
            if let Some(dst) = instr_dst(ins) {
                active.remove(&dst);
            }
        }
    }
}

fn eligible_locals(module: &Module, func: &Function) -> BTreeSet<Local> {
    let mut candidates: BTreeSet<Local> = BTreeSet::new();
    for block in &func.blocks {
        for ins in &block.instrs {
            if let Instr::Retain {  obj , ..} | Instr::Release { obj , ..} = ins {
                candidates.insert(*obj);
            }
        }
    }
    if candidates.is_empty() {
        return candidates;
    }
    let alias = collect_aliases(func);
    let (genref, returned) = tainted_roots(func, &alias);
    candidates
        .into_iter()
        .filter(|l| {
            let root = find_root(&alias, *l);
            if genref.contains(&root) {
                return false;
            }
            if returned.contains(&root) {
                return false;
            }
            trivial_class(module, &local_type(func, *l))
        })
        .collect()
}

fn elide_intra_block(blocks: &mut [Block], eligible: &BTreeSet<Local>) -> bool {
    let mut changed = false;
    for block in blocks.iter_mut() {
        let mut active: BTreeMap<Local, usize> = BTreeMap::new();
        let mut drop: BTreeSet<usize> = BTreeSet::new();
        for (ii, ins) in block.instrs.iter().enumerate() {
            match ins {
                Instr::Retain {  obj , ..} => {
                    if eligible.contains(obj) {
                        active.entry(*obj).or_insert(ii);
                    }
                }
                Instr::Release {  obj , ..} => {
                    if eligible.contains(obj) {
                        if let Some(ri) = active.remove(obj) {
                            drop.insert(ri);
                            drop.insert(ii);
                            changed = true;
                        }
                    }
                }
                _ => {}
            }
            invalidate_barrier(&mut active, ins);
        }
        if !drop.is_empty() {
            let mut kept: Vec<Instr> = Vec::with_capacity(block.instrs.len());
            for (ii, ins) in block.instrs.drain(..).enumerate() {
                if !drop.contains(&ii) {
                    kept.push(ins);
                }
            }
            block.instrs = kept;
        }
    }
    changed
}

struct EpilogueFacts {
    defs: BTreeMap<Local, usize>,
    escaped: BTreeSet<Local>,
    releases: BTreeMap<Local, Vec<(usize, usize)>>,
}

impl EpilogueFacts {
    fn collect(func: &Function) -> EpilogueFacts {
        let mut facts = EpilogueFacts {
            defs: BTreeMap::new(),
            escaped: BTreeSet::new(),
            releases: BTreeMap::new(),
        };
        for (bi, block) in func.blocks.iter().enumerate() {
            for (ii, ins) in block.instrs.iter().enumerate() {
                if let Some(dst) = instr_dst(ins) {
                    *facts.defs.entry(dst).or_insert(0) += 1;
                }
                if let Instr::Call {  dsts, err , ..} = ins {
                    for (i, d) in dsts.iter().enumerate() {
                        if dsts[..i].contains(d) {
                            continue;
                        }
                        *facts.defs.entry(*d).or_insert(0) += 1;
                    }
                    if let Some(e) = err {
                        if !dsts.contains(e) {
                            *facts.defs.entry(*e).or_insert(0) += 1;
                        }
                    }
                }
                match ins {
                    Instr::Call {  args , ..} => {
                        for a in args {
                            facts.escaped.insert(*a);
                        }
                    }
                    Instr::SetField {  value , ..} | Instr::SetFieldByName { value , ..} => {
                        facts.escaped.insert(*value);
                    }
                    Instr::ArraySet {  value , ..} => {
                        facts.escaped.insert(*value);
                    }
                    Instr::ArrayPush {  value , ..} => {
                        facts.escaped.insert(*value);
                    }
                    Instr::EnumNew {  payload , ..} => {
                        for p in payload {
                            facts.escaped.insert(*p);
                        }
                    }
                    Instr::ClosureNew {  captures , ..} => {
                        for c in captures {
                            facts.escaped.insert(*c);
                        }
                    }
                    Instr::GenRefOf {  obj , ..} => {
                        facts.escaped.insert(*obj);
                    }
                    Instr::AddrOf {  src , ..} => {
                        facts.escaped.insert(*src);
                    }
                    Instr::Release {  obj , ..} => {
                        facts.releases.entry(*obj).or_default().push((bi, ii));
                    }
                    _ => {}
                }
            }
            match &block.term {
                Terminator::Ret(v) => {
                    for l in v {
                        facts.escaped.insert(*l);
                    }
                }
                Terminator::Throw { src, .. } => {
                    facts.escaped.insert(*src);
                }
                Terminator::Rethrow { err, .. } => {
                    facts.escaped.insert(*err);
                }
                Terminator::BrErr { catch_bind, .. } => {
                    *facts.defs.entry(*catch_bind).or_insert(0) += 1;
                }
                _ => {}
            }
        }
        facts
    }
}

fn elide_epilogue(
    module: &mut Module,
    fi: usize,
    dom: &DominatorTree,
    eligible: &BTreeSet<Local>,
) -> bool {
    let entry_retains: Vec<(Local, usize)> = module.functions[fi].blocks
        .first()
        .map(|b| {
            b.instrs
                .iter()
                .enumerate()
                .filter_map(|(ii, ins)| match ins {
                    Instr::Retain {  obj , ..} => Some((*obj, ii)),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    if entry_retains.is_empty() {
        return false;
    }
    let mut changed = false;
    let mut facts: Option<EpilogueFacts> = None;
    for (local, _) in entry_retains {
        if !eligible.contains(&local) {
            continue;
        }
        if facts.is_none() {
            facts = Some(EpilogueFacts::collect(&module.functions[fi]));
        }
        let f = facts.as_ref().unwrap();
        if f.defs.get(&local).copied().unwrap_or(0) > 0 {
            continue;
        }
        let entry_count = module.functions[fi].blocks[0]
            .instrs
            .iter()
            .filter(|ins| matches!(ins, Instr::Retain {  obj , ..} if *obj == local))
            .count();
        if entry_count != 1 {
            continue;
        }
        if f.escaped.contains(&local) {
            continue;
        }
        let mut exits: Vec<(usize, usize)> = Vec::new();
        let mut ok = true;
        if let Some(rel) = f.releases.get(&local) {
            let mut at = 0;
            while at < rel.len() {
                let (bi, ii) = rel[at];
                let mut end = at + 1;
                while end < rel.len() && rel[end].0 == bi {
                    end += 1;
                }
                if matches!(module.functions[fi].blocks[bi].term, Terminator::Ret(_)) {
                    if end - at > 1 {
                        ok = false;
                        break;
                    }
                    if !dom.dominates(0, bi) {
                        ok = false;
                        break;
                    }
                    exits.push((bi, ii));
                }
                at = end;
            }
        }
        if !ok || exits.is_empty() {
            continue;
        }
        {
            let func = &mut module.functions[fi];
            for (bi, ii) in exits.iter().rev() {
                func.blocks[*bi].instrs.remove(*ii);
            }
            func.blocks[0].instrs.retain(|ins| {
                !matches!(ins, Instr::Retain {  obj , ..} if *obj == local)
            });
        }
        changed = true;
        facts = None;
    }
    changed
}

pub fn eliminate_redundant_arc(
    module: &mut Module,
    fi: usize,
    dom: &DominatorTree,
) -> bool {
    let eligible = eligible_locals(module, &module.functions[fi]);
    if eligible.is_empty() {
        return false;
    }
    let mut changed = false;
    {
        let func = &mut module.functions[fi];
        changed |= elide_intra_block(&mut func.blocks, &eligible);
    }
    changed |= elide_epilogue(module, fi, dom, &eligible);
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arc_func() -> Function {
        Function {
            name: "f".to_string(),
            params: Vec::new(),
            sig_params: Vec::new(),
            ret: LirType::I64,
            throws: false,
            is_unsafe: false,
            method_self: false,
            is_pub: false,
            is_closure: false,
            locals: vec![LirType::Obj("C".to_string()), LirType::I64],
            blocks: vec![Block {
                instrs: vec![
                    Instr::Retain {span: UNKNOWN_SPAN,  obj: 0 },
                    Instr::GetField {span: UNKNOWN_SPAN,  dst: 1, obj: 0, field: 0 },
                    Instr::Release {span: UNKNOWN_SPAN,  obj: 0 },
                ],
                term: Terminator::Ret(vec![1]),
            }],
        }
    }

    fn test_module(func: Function) -> Module {
        let mut module = Module::default();
        module.classes.push(ClassDesc {
            name: "C".to_string(),
            type_params: Vec::new(),
            is_struct: false,
            fields: vec![FieldDesc {
                name: "x".to_string(),
                ty: LirType::I64,
                private: false,
                owner: "C".to_string(),
            }],
            field_index: BTreeMap::from([("x".to_string(), 0)]),
            methods: BTreeMap::new(),
            deinit: None,
            dtor: None,
            ifaces: Vec::new(),
            parent: None,
        });
        module.functions.push(func);
        module.fn_index.insert("f".to_string(), 0);
        module.class_index.insert("C".to_string(), 0);
        module
    }

    fn dom_of(module: &Module) -> DominatorTree {
        crate::licm::compute_dominators(&module.functions[0])
    }

    #[test]
    fn pairs_cancelled_intra_block() {
        let mut module = test_module(arc_func());
        let dom = dom_of(&module);
        assert!(eliminate_redundant_arc(&mut module, 0, &dom));
        assert!(module.functions[0].blocks[0]
            .instrs
            .iter()
            .all(|i| !matches!(i, Instr::Retain { ..} | Instr::Release { ..})));
    }

    #[test]
    fn call_barrier_preserves_pair() {
        let mut module = test_module(Function {
            name: "f".to_string(),
            params: Vec::new(),
            sig_params: Vec::new(),
            ret: LirType::I64,
            throws: false,
            is_unsafe: false,
            method_self: false,
            is_pub: false,
            is_closure: false,
            locals: vec![LirType::Obj("C".to_string()), LirType::I64],
            blocks: vec![
                Block { instrs: Vec::new(), term: Terminator::Br(1) },
                Block {
                    instrs: vec![
                        Instr::Retain {span: UNKNOWN_SPAN,  obj: 0 },
                        Instr::Call {span: UNKNOWN_SPAN, 
                            dsts: vec![],
                            err: None,
                            target: CallTarget::Builtin("print".to_string()),
                            args: Vec::new(),
                        },
                        Instr::GetField {span: UNKNOWN_SPAN,  dst: 1, obj: 0, field: 0 },
                        Instr::Release {span: UNKNOWN_SPAN,  obj: 0 },
                    ],
                    term: Terminator::Ret(vec![1]),
                },
            ],
        });
        let dom = dom_of(&module);
        assert!(!eliminate_redundant_arc(&mut module, 0, &dom));
    }

    #[test]
    fn epilogue_pair_removed() {
        let mut module = test_module(Function {
            name: "f".to_string(),
            params: Vec::new(),
            sig_params: Vec::new(),
            ret: LirType::I64,
            throws: false,
            is_unsafe: false,
            method_self: false,
            is_pub: false,
            is_closure: false,
            locals: vec![LirType::Obj("C".to_string()), LirType::I64],
            blocks: vec![
                Block {
                    instrs: vec![Instr::Retain {span: UNKNOWN_SPAN,   obj: 0 }],
                    term: Terminator::Br(1),
                },
                Block {
                    instrs: vec![
                        Instr::GetField {span: UNKNOWN_SPAN,  dst: 1, obj: 0, field: 0 },
                        Instr::Release {span: UNKNOWN_SPAN,  obj: 0 },
                    ],
                    term: Terminator::Ret(vec![1]),
                },
            ],
        });
        let dom = dom_of(&module);
        assert!(eliminate_redundant_arc(&mut module, 0, &dom));
        for b in &module.functions[0].blocks {
            assert!(b.instrs.iter().all(|i| !matches!(
                i,
                Instr::Retain { ..} | Instr::Release { ..}
            )));
        }
    }

    fn mk_func(locals: Vec<LirType>, blocks: Vec<Block>) -> Function {
        Function {
            name: "f".to_string(),
            params: Vec::new(),
            sig_params: Vec::new(),
            ret: LirType::I64,
            throws: false,
            is_unsafe: false,
            method_self: false,
            is_pub: false,
            is_closure: false,
            locals,
            blocks,
        }
    }

    fn obj_locals(n: usize, extra: usize) -> Vec<LirType> {
        let mut v = vec![LirType::Obj("C".to_string()); n];
        v.extend(std::iter::repeat(LirType::I64).take(extra));
        v
    }

    fn retain(obj: Local) -> Instr {
        Instr::Retain { span: UNKNOWN_SPAN, obj }
    }

    fn release(obj: Local) -> Instr {
        Instr::Release { span: UNKNOWN_SPAN, obj }
    }

    fn getf(dst: Local, obj: Local) -> Instr {
        Instr::GetField { span: UNKNOWN_SPAN, dst, obj, field: 0 }
    }

    fn builtin_call() -> Instr {
        Instr::Call {
            span: UNKNOWN_SPAN,
            dsts: vec![],
            err: None,
            target: CallTarget::Builtin("print".to_string()),
            args: Vec::new(),
        }
    }

    fn count_arc(func: &Function) -> (usize, usize) {
        let mut retains = 0;
        let mut releases = 0;
        for b in &func.blocks {
            for i in &b.instrs {
                match i {
                    Instr::Retain { .. } => retains += 1,
                    Instr::Release { .. } => releases += 1,
                    _ => {}
                }
            }
        }
        (retains, releases)
    }

    fn dump_case(name: &str, func: Function) -> String {
        let mut module = test_module(func);
        let (rb, lb) = count_arc(&module.functions[0]);
        let dom = dom_of(&module);
        let changed = eliminate_redundant_arc(&mut module, 0, &dom);
        let (ra, la) = count_arc(&module.functions[0]);
        let mut out = format!(
            "=== {name} ===\nchanged={changed} retains={rb}->{ra} releases={lb}->{la}\n"
        );
        for (bi, b) in module.functions[0].blocks.iter().enumerate() {
            out.push_str(&format!("block {bi} term={:?}\n", b.term));
            for ins in &b.instrs {
                out.push_str(&format!("  {ins:?}\n"));
            }
        }
        out
    }

    fn corpus_dump() -> String {
        let mut out = String::new();
        let mut dense_instrs = Vec::new();
        for i in 0..6u32 {
            dense_instrs.push(retain(i));
            dense_instrs.push(getf(100 + i, i));
            dense_instrs.push(release(i));
        }
        out.push_str(&dump_case(
            "dense_pairs",
            mk_func(
                obj_locals(6, 110),
                vec![Block {
                    instrs: dense_instrs,
                    term: Terminator::Ret(vec![100]),
                }],
            ),
        ));
        out.push_str(&dump_case(
            "call_barrier",
            mk_func(
                obj_locals(1, 4),
                vec![Block {
                    instrs: vec![retain(0), builtin_call(), getf(1, 0), release(0)],
                    term: Terminator::Ret(vec![1]),
                }],
            ),
        ));
        out.push_str(&dump_case(
            "sparse_setfield",
            mk_func(
                obj_locals(3, 10),
                vec![Block {
                    instrs: vec![
                        retain(0),
                        Instr::SetField { span: UNKNOWN_SPAN, obj: 5, field: 0, value: 6 },
                        retain(1),
                        Instr::SetField { span: UNKNOWN_SPAN, obj: 5, field: 0, value: 0 },
                        release(0),
                        release(1),
                        retain(2),
                        Instr::SetField { span: UNKNOWN_SPAN, obj: 2, field: 0, value: 6 },
                        release(2),
                    ],
                    term: Terminator::Ret(vec![4]),
                }],
            ),
        ));
        out.push_str(&dump_case(
            "closure_capture",
            mk_func(
                obj_locals(2, 4),
                vec![Block {
                    instrs: vec![
                        retain(0),
                        retain(1),
                        Instr::ClosureNew {
                            span: UNKNOWN_SPAN,
                            dst: 3,
                            func: 0,
                            captures: vec![0],
                            decay: false,
                            decay_this: false,
                        },
                        release(0),
                        release(1),
                    ],
                    term: Terminator::Ret(vec![4]),
                }],
            ),
        ));
        out.push_str(&dump_case(
            "array_value_barriers",
            mk_func(
                obj_locals(2, 10),
                vec![Block {
                    instrs: vec![
                        retain(0),
                        Instr::ArrayPush { span: UNKNOWN_SPAN, arr: 5, value: 0, elem_size: 8 },
                        release(0),
                        retain(1),
                        Instr::ArraySet {
                            span: UNKNOWN_SPAN,
                            arr: 5,
                            index: 6,
                            value: 1,
                            elem_size: 8,
                            unchecked: false,
                        },
                        release(1),
                    ],
                    term: Terminator::Ret(vec![4]),
                }],
            ),
        ));
        out.push_str(&dump_case(
            "redefine_barrier",
            mk_func(
                obj_locals(1, 4),
                vec![Block {
                    instrs: vec![
                        retain(0),
                        Instr::Copy { span: UNKNOWN_SPAN, dst: 0, src: 1 },
                        release(0),
                    ],
                    term: Terminator::Ret(vec![2]),
                }],
            ),
        ));
        out.push_str(&dump_case(
            "genref_direct",
            mk_func(
                obj_locals(1, 10),
                vec![
                    Block { instrs: vec![retain(0), getf(2, 0)], term: Terminator::Br(1) },
                    Block {
                        instrs: vec![
                            Instr::GenRefOf { span: UNKNOWN_SPAN, dst: 9, obj: 0 },
                            release(0),
                        ],
                        term: Terminator::Ret(vec![2]),
                    },
                ],
            ),
        ));
        out.push_str(&dump_case(
            "genref_alias",
            mk_func(
                obj_locals(1, 10),
                vec![
                    Block {
                        instrs: vec![
                            retain(0),
                            Instr::Copy { span: UNKNOWN_SPAN, dst: 7, src: 0 },
                            getf(2, 0),
                        ],
                        term: Terminator::Br(1),
                    },
                    Block {
                        instrs: vec![
                            Instr::GenRefOf { span: UNKNOWN_SPAN, dst: 9, obj: 7 },
                            release(0),
                        ],
                        term: Terminator::Ret(vec![2]),
                    },
                ],
            ),
        ));
        out.push_str(&dump_case(
            "returned_excluded",
            mk_func(
                obj_locals(1, 4),
                vec![Block {
                    instrs: vec![retain(0), getf(1, 0), release(0)],
                    term: Terminator::Ret(vec![0]),
                }],
            ),
        ));
        out.push_str(&dump_case(
            "thrown_excluded",
            mk_func(
                obj_locals(1, 4),
                vec![Block {
                    instrs: vec![retain(0), release(0)],
                    term: Terminator::Throw { span: UNKNOWN_SPAN, src: 0, catch: None },
                }],
            ),
        ));
        out.push_str(&dump_case(
            "cross_block_epilogue",
            mk_func(
                obj_locals(1, 4),
                vec![
                    Block { instrs: vec![retain(0)], term: Terminator::Br(1) },
                    Block { instrs: vec![release(0)], term: Terminator::Ret(vec![]) },
                ],
            ),
        ));
        out.push_str(&dump_case(
            "defer_barrier",
            mk_func(
                obj_locals(1, 4),
                vec![Block {
                    instrs: vec![
                        retain(0),
                        Instr::Defer { span: UNKNOWN_SPAN, body: vec![] },
                        release(0),
                    ],
                    term: Terminator::Ret(vec![1]),
                }],
            ),
        ));
        out.push_str(&dump_case(
            "spawn_barrier",
            mk_func(
                obj_locals(1, 4),
                vec![Block {
                    instrs: vec![
                        retain(0),
                        Instr::ThreadSpawn {
                            span: UNKNOWN_SPAN,
                            dst: 2,
                            func: 0,
                            closure: None,
                            ret_tag: 0,
                        },
                        release(0),
                    ],
                    term: Terminator::Ret(vec![1]),
                }],
            ),
        ));
        out.push_str(&dump_case(
            "pool_barriers",
            mk_func(
                vec![
                    LirType::Obj("C".to_string()),
                    LirType::Obj("C".to_string()),
                    LirType::Pool,
                    LirType::I64,
                    LirType::I64,
                ],
                vec![Block {
                    instrs: vec![
                        retain(0),
                        Instr::PoolJoin { span: UNKNOWN_SPAN, pool: 2 },
                        release(0),
                        retain(1),
                        Instr::PoolInit { span: UNKNOWN_SPAN, dst: 2, id: 3, workers: 4 },
                        release(1),
                    ],
                    term: Terminator::Ret(vec![3]),
                }],
            ),
        ));
        out.push_str(&dump_case(
            "call_intra",
            mk_func(
                obj_locals(1, 4),
                vec![Block {
                    instrs: vec![retain(0), builtin_call(), release(0), retain(0)],
                    term: Terminator::Ret(vec![]),
                }],
            ),
        ));
        out.push_str(&dump_case(
            "defer_intra",
            mk_func(
                obj_locals(1, 4),
                vec![Block {
                    instrs: vec![
                        retain(0),
                        Instr::Defer { span: UNKNOWN_SPAN, body: vec![] },
                        release(0),
                        retain(0),
                    ],
                    term: Terminator::Ret(vec![]),
                }],
            ),
        ));
        out.push_str(&dump_case(
            "spawn_intra",
            mk_func(
                obj_locals(1, 4),
                vec![Block {
                    instrs: vec![
                        retain(0),
                        Instr::ThreadSpawn {
                            span: UNKNOWN_SPAN,
                            dst: 2,
                            func: 0,
                            closure: None,
                            ret_tag: 0,
                        },
                        release(0),
                        retain(0),
                    ],
                    term: Terminator::Ret(vec![]),
                }],
            ),
        ));
        out.push_str(&dump_case(
            "pool_intra",
            mk_func(
                vec![
                    LirType::Obj("C".to_string()),
                    LirType::Pool,
                    LirType::I64,
                ],
                vec![Block {
                    instrs: vec![
                        retain(0),
                        Instr::PoolJoin { span: UNKNOWN_SPAN, pool: 1 },
                        release(0),
                        retain(0),
                    ],
                    term: Terminator::Ret(vec![]),
                }],
            ),
        ));
        out.push_str(&dump_case(
            "first_retain_wins",
            mk_func(
                obj_locals(1, 4),
                vec![Block {
                    instrs: vec![retain(0), retain(0), release(0)],
                    term: Terminator::Ret(vec![1]),
                }],
            ),
        ));
        out.push_str(&dump_case(
            "lone_release",
            mk_func(
                obj_locals(1, 4),
                vec![Block {
                    instrs: vec![release(0)],
                    term: Terminator::Ret(vec![1]),
                }],
            ),
        ));
        out.push_str(&dump_case(
            "array_type_ineligible",
            mk_func(
                vec![LirType::Array(Box::new(LirType::I64)), LirType::I64],
                vec![Block {
                    instrs: vec![retain(0), release(0)],
                    term: Terminator::Ret(vec![1]),
                }],
            ),
        ));
        out.push_str(&dump_case(
            "diamond_epilogue",
            mk_func(
                obj_locals(1, 4),
                vec![
                    Block {
                        instrs: vec![retain(0)],
                        term: Terminator::BrIf { span: UNKNOWN_SPAN, cond: 2, then_bb: 1, else_bb: 2 },
                    },
                    Block { instrs: vec![release(0)], term: Terminator::Ret(vec![]) },
                    Block { instrs: vec![release(0)], term: Terminator::Ret(vec![]) },
                ],
            ),
        ));
        out
    }

    #[test]
    fn arc_differential_corpus() {
        let dump = corpus_dump();
        println!("{dump}");
        assert!(dump.contains("=== dense_pairs ===\nchanged=true retains=6->0 releases=6->0"));
        assert!(dump.contains("=== call_barrier ===\nchanged=true retains=1->0 releases=1->0"));
        assert!(dump.contains("=== call_intra ===\nchanged=false retains=2->2 releases=1->1"));
        assert!(dump.contains("=== defer_intra ===\nchanged=false retains=2->2 releases=1->1"));
        assert!(dump.contains("=== spawn_intra ===\nchanged=false retains=2->2 releases=1->1"));
        assert!(dump.contains("=== pool_intra ===\nchanged=false retains=2->2 releases=1->1"));
        assert!(dump.contains("=== sparse_setfield ===\nchanged=true retains=3->1 releases=3->1"));
        assert!(dump.contains("=== first_retain_wins ===\nchanged=true retains=2->1 releases=1->0"));
        assert!(dump.contains("=== lone_release ===\nchanged=false retains=0->0 releases=1->1"));
        assert!(dump.contains("=== array_type_ineligible ===\nchanged=false"));
    }
}
