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

fn group_has_genref(func: &Function, alias: &BTreeMap<Local, Local>, local: Local) -> bool {
    let root = find_root(alias, local);
    for block in &func.blocks {
        for ins in &block.instrs {
            if let Instr::GenRefOf {  obj , ..} = ins {
                if find_root(alias, *obj) == root {
                    return true;
                }
            }
        }
    }
    false
}

fn group_returned(func: &Function, alias: &BTreeMap<Local, Local>, local: Local) -> bool {
    let root = find_root(alias, local);
    for block in &func.blocks {
        match &block.term {
            Terminator::Ret(v) => {
                if v.iter().any(|l| find_root(alias, *l) == root) {
                    return true;
                }
            }
            Terminator::Throw { src, .. } => {
                if find_root(alias, *src) == root {
                    return true;
                }
            }
            Terminator::Rethrow { err, .. } => {
                if find_root(alias, *err) == root {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

fn is_barrier(ins: &Instr, local: Local) -> bool {
    match ins {
        Instr::SetField {  obj, value , ..} | Instr::SetFieldByName { obj, value , ..} => {
            *obj == local || *value == local
        }
        Instr::ArrayPush {  value , ..} => *value == local,
        Instr::ArraySet {  value , ..} => *value == local,
        Instr::ThreadSpawn { ..} => true,
        Instr::PoolInit { ..}
        | Instr::PoolSubmit { ..}
        | Instr::PoolParallelFor { ..}
        | Instr::PoolJoin { ..}
        | Instr::PoolShutdown { ..} => true,
        Instr::ClosureNew {  captures , ..} => captures.contains(&local),
        Instr::Call { ..} => true,
        Instr::Defer { ..} => true,
        _ => match instr_dst(ins) {
            Some(dst) => dst == local,
            None => false,
        },
    }
}

fn pair_eligible(module: &Module, func: &Function, local: Local) -> bool {
    let alias = collect_aliases(func);
    if group_has_genref(func, &alias, local) {
        return false;
    }
    if group_returned(func, &alias, local) {
        return false;
    }
    trivial_class(module, &local_type(func, local))
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
    candidates
        .into_iter()
        .filter(|l| pair_eligible(module, func, *l))
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
            for (local, _) in active.clone() {
                if is_barrier(ins, local) {
                    active.remove(&local);
                }
            }
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
    for (local, _) in entry_retains {
        if !eligible.contains(&local) {
            continue;
        }
        if all_defs(&module.functions[fi], local) > 0 {
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
        if escapes_anywhere(&module.functions[fi], local) {
            continue;
        }
        let mut exits: Vec<(usize, usize)> = Vec::new();
        let mut ok = true;
        for (bi, block) in module.functions[fi].blocks.iter().enumerate() {
            if !matches!(block.term, Terminator::Ret(_)) {
                continue;
            }
            let mut found = None;
            for (ii, ins) in block.instrs.iter().enumerate() {
                if matches!(ins, Instr::Release {  obj , ..} if *obj == local) {
                    if found.is_some() {
                        ok = false;
                        break;
                    }
                    found = Some(ii);
                }
            }
            if !ok {
                break;
            }
            if let Some(ii) = found {
                if !dom.dominates(0, bi) {
                    ok = false;
                    break;
                }
                exits.push((bi, ii));
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
    }
    changed
}

fn all_defs(func: &Function, local: Local) -> usize {
    let mut count = 0;
    for block in &func.blocks {
        for ins in &block.instrs {
            if instr_dst(ins) == Some(local) {
                count += 1;
            }
            if let Instr::Call {  dsts, err , ..} = ins {
                if dsts.contains(&local) || *err == Some(local) {
                    count += 1;
                }
            }
        }
        if let Terminator::BrErr { catch_bind, .. } = &block.term {
            if *catch_bind == local {
                count += 1;
            }
        }
    }
    count
}

fn escapes_anywhere(func: &Function, local: Local) -> bool {
    for block in &func.blocks {
        for ins in &block.instrs {
            match ins {
                Instr::Call {  args , ..} => {
                    if args.contains(&local) {
                        return true;
                    }
                }
                Instr::SetField {  value , ..} | Instr::SetFieldByName { value , ..} => {
                    if *value == local {
                        return true;
                    }
                }
                Instr::ArraySet {  value , ..} => {
                    if *value == local {
                        return true;
                    }
                }
                Instr::ArrayPush {  value , ..} => {
                    if *value == local {
                        return true;
                    }
                }
                Instr::EnumNew {  payload , ..} => {
                    if payload.contains(&local) {
                        return true;
                    }
                }
                Instr::ClosureNew {  captures , ..} => {
                    if captures.contains(&local) {
                        return true;
                    }
                }
                Instr::GenRefOf {  obj , ..} => {
                    if *obj == local {
                        return true;
                    }
                }
                Instr::AddrOf {  src , ..} => {
                    if *src == local {
                        return true;
                    }
                }
                _ => {}
            }
        }
        match &block.term {
            Terminator::Ret(v) => {
                if v.contains(&local) {
                    return true;
                }
            }
            Terminator::Throw { src, .. } => {
                if *src == local {
                    return true;
                }
            }
            Terminator::Rethrow { err, .. } => {
                if *err == local {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
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
}
