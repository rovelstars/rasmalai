use crate::instr::*;
use std::collections::{BTreeMap, BTreeSet};

fn fresh_builtin(name: &str) -> bool {
    matches!(
        name,
        "__rnx_json_parse"
            | "__rnx_json_parse_typed"
            | "__rnx_json_stringify"
            | "__rnx_string_concat"
            | "__rnx_any_to_str"
            | "__rnx_int_to_str"
            | "__rnx_float_to_str"
            | "__rnx_bool_to_str"
    )
}

fn alias_root(func: &Function, mut local: Local) -> Option<Local> {
    let mut seen = BTreeSet::new();
    loop {
        if !seen.insert(local) {
            return None;
        }
        let mut found = None;
        for block in &func.blocks {
            for ins in &block.instrs {
                match ins {
                    Instr::Copy { dst, src, .. } | Instr::Cast { dst, src, .. } => {
                        if *dst == local {
                            if found.is_some() {
                                return None;
                            }
                            found = Some(*src);
                        }
                    }
                    _ => {}
                }
            }
        }
        match found {
            Some(src) => local = src,
            None => return Some(local),
        }
    }
}

fn root_fresh(
    module: &Module,
    fi: usize,
    root: Local,
    memo: &mut BTreeMap<usize, bool>,
    stack: &mut Vec<usize>,
) -> bool {
    let func = &module.functions[fi];
    let mut count = 0;
    let mut def_ins: Option<(usize, usize)> = None;
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, ins) in block.instrs.iter().enumerate() {
            let mut dsts = Vec::new();
            crate::opt::instr_dsts(ins, &mut dsts);
            if dsts.contains(&root) {
                count += 1;
                def_ins = Some((bi, ii));
            }
        }
    }
    if count != 1 {
        return false;
    }
    let (bi, ii) = def_ins.unwrap_or((usize::MAX, usize::MAX));
    if bi == usize::MAX {
        return false;
    }
    match &func.blocks[bi].instrs[ii] {
        Instr::Concat { .. } | Instr::ToStr { .. } => true,
        Instr::ObjNew { .. }
        | Instr::ArrayNew { .. }
        | Instr::EnumNew { .. }
        | Instr::ClosureNew { .. } => true,
        Instr::Call { target, .. } => match target {
            CallTarget::Fn(id) => callee_fresh(module, *id, memo, stack),
            CallTarget::Builtin(n) => fresh_builtin(n),
            _ => false,
        },
        _ => false,
    }
}

fn callee_fresh(
    module: &Module,
    fi: usize,
    memo: &mut BTreeMap<usize, bool>,
    stack: &mut Vec<usize>,
) -> bool {
    if let Some(v) = memo.get(&fi) {
        return *v;
    }
    if stack.contains(&fi) {
        return false;
    }
    stack.push(fi);
    let func = match module.functions.get(fi) {
        Some(f) => f,
        None => {
            stack.pop();
            return false;
        }
    };
    let rets = flat_sig(&func.ret);
    let mut ok = false;
    for block in &func.blocks {
        if let Terminator::Ret(v) = &block.term {
            if v.is_empty() {
                continue;
            }
            ok = true;
            for (i, r) in v.iter().enumerate() {
                if i >= rets.len() {
                    ok = false;
                    break;
                }
                match alias_root(func, *r) {
                    Some(root) => {
                        if !root_fresh(module, fi, root, memo, stack) {
                            ok = false;
                            break;
                        }
                    }
                    None => {
                        ok = false;
                        break;
                    }
                }
            }
            if !ok {
                break;
            }
        }
    }
    stack.pop();
    memo.insert(fi, ok);
    ok
}

fn fresh_def(
    module: &Module,
    ins: &Instr,
    local: Local,
    memo: &mut BTreeMap<usize, bool>,
    stack: &mut Vec<usize>,
) -> bool {
    match ins {
        Instr::Concat { dst, .. } | Instr::ToStr { dst, .. } => *dst == local,
        Instr::Call { dsts, target, .. } => {
            if !dsts.contains(&local) {
                return false;
            }
            match target {
                CallTarget::Fn(id) => {
                    let didx = dsts.iter().position(|d| *d == local).unwrap_or(usize::MAX);
                    let rets = module
                        .functions
                        .get(*id)
                        .map(|f| flat_sig(&f.ret))
                        .unwrap_or_default();
                    if didx >= rets.len() {
                        return false;
                    }
                    callee_fresh(module, *id, memo, stack)
                }
                CallTarget::Builtin(n) => fresh_builtin(n),
                _ => false,
            }
        }
        _ => false,
    }
}

fn heap_type(func: &Function, local: Local) -> bool {
    matches!(
        func.locals.get(local as usize),
        Some(LirType::Str)
            | Some(LirType::Array(_))
            | Some(LirType::Enum(_))
            | Some(LirType::Any)
            | Some(LirType::Error)
            | Some(LirType::Obj(_))
    )
}

fn term_reads(term: &Terminator, out: &mut Vec<Local>) {
    match term {
        Terminator::Ret(v) => out.extend(v.iter().copied()),
        Terminator::Br(_) => {}
        Terminator::BrIf { cond, .. } => out.push(*cond),
        Terminator::BrErr { err, .. } => out.push(*err),
        Terminator::Switch { scrut, .. } => out.push(*scrut),
        Terminator::Throw { src, .. } => out.push(*src),
        Terminator::Rethrow { err, .. } => out.push(*err),
        _ => {}
    }
}

fn consuming_builtin(name: &str) -> bool {
    matches!(
        name,
        "__rnx_any_release_box"
            | "__rnx_json_unwrap"
            | "__rnx_gmap_free"
            | "__rnx_bytes_free"
            | "__rnx_sync_channel_drop"
    )
}

fn param_aliases(func: &Function, local: Local, nparams: usize) -> bool {
    if (local as usize) < nparams {
        return true;
    }
    let mut seen = BTreeSet::new();
    let mut cur = local;
    loop {
        if !seen.insert(cur) {
            return false;
        }
        let mut found = None;
        let mut multi = false;
        for block in &func.blocks {
            for ins in &block.instrs {
                match ins {
                    Instr::Copy { dst, src, .. } | Instr::Cast { dst, src, .. } => {
                        if *dst == cur {
                            if found.is_some() {
                                multi = true;
                                break;
                            }
                            found = Some(*src);
                        }
                    }
                    _ => {}
                }
            }
            if multi {
                break;
            }
        }
        if multi {
            return false;
        }
        match found {
            Some(src) => {
                if (src as usize) < nparams {
                    return true;
                }
                cur = src;
            }
            None => return false,
        }
    }
}

fn walk_all_instrs(func: &Function, f: &mut impl FnMut(&Instr)) {
    for block in &func.blocks {
        for ins in &block.instrs {
            f(ins);
            if let Instr::Defer { body, .. } = ins {
                for nested in body {
                    f(nested);
                }
            }
        }
    }
}

pub fn callee_may_consume(module: &Module, fi: usize) -> bool {
    let func = match module.functions.get(fi) {
        Some(f) => f,
        None => return true,
    };
    let nparams = func.params.len();
    let mut escapes = false;
    walk_all_instrs(func, &mut |ins| {
        if escapes {
            return;
        }
        if let Instr::Call { target, args, .. } = ins {
            if let CallTarget::Builtin(n) = target {
                if consuming_builtin(n)
                    && args.iter().any(|a| param_aliases(func, *a, nparams))
                {
                    escapes = true;
                }
            }
        }
    });
    escapes
}

pub fn sweep_temps(module: &mut Module, fi: usize) -> bool {
    let mut memo: BTreeMap<usize, bool> = BTreeMap::new();
    let mut stack: Vec<usize> = Vec::new();
    let func = &module.functions[fi];
    let mut defs: BTreeMap<Local, (usize, usize)> = BTreeMap::new();
    let mut def_count: BTreeMap<Local, usize> = BTreeMap::new();
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, ins) in block.instrs.iter().enumerate() {
            let mut dsts = Vec::new();
            crate::opt::instr_dsts(ins, &mut dsts);
            if let Instr::Call { err: Some(e), .. } = ins {
                dsts.push(*e);
            }
            for d in dsts {
                *def_count.entry(d).or_insert(0) += 1;
                defs.entry(d).or_insert((bi, ii));
            }
        }
        if let Terminator::BrErr { catch_bind, .. } = &block.term {
            *def_count.entry(*catch_bind).or_insert(0) += 1;
        }
    }
    let mut uses: BTreeMap<Local, usize> = BTreeMap::new();
    let mut reads = Vec::new();
    for block in &func.blocks {
        for ins in &block.instrs {
            reads.clear();
            crate::opt::instr_reads(ins, &mut reads);
            for r in reads.drain(..) {
                *uses.entry(r).or_insert(0) += 1;
            }
        }
        reads.clear();
        term_reads(&block.term, &mut reads);
        for r in reads.drain(..) {
            *uses.entry(r).or_insert(0) += 1;
        }
    }

    let mut releasable = |func: &Function, bi: usize, local: Local, expect_uses: usize| -> bool {
        if !heap_type(func, local) {
            return false;
        }
        if uses.get(&local).copied().unwrap_or(0) != expect_uses {
            return false;
        }
        if def_count.get(&local).copied().unwrap_or(0) != 1 {
            return false;
        }
        match defs.get(&local) {
            Some((dbi, dii)) if *dbi == bi => {
                let ins = func.blocks[*dbi].instrs[*dii].clone();
                fresh_def(module, &ins, local, &mut memo, &mut stack)
            }
            _ => false,
        }
    };

    let mut insert: Vec<(usize, usize, Local)> = Vec::new();
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, ins) in block.instrs.iter().enumerate() {
            match ins {
                Instr::Copy { dst, src, .. } => {
                    if dst != src
                        && matches!(func.locals.get(*dst as usize), Some(LirType::Any))
                        && releasable(func, bi, *src, 1)
                    {
                        insert.push((bi, ii, *src));
                    }
                }
                Instr::Concat { dst, lhs, rhs, .. } => {
                    for op in [*lhs, *rhs] {
                        if releasable(func, bi, op, 1) {
                            insert.push((bi, ii, op));
                        }
                    }
                    if releasable(func, bi, *dst, 0) {
                        insert.push((bi, ii, *dst));
                    }
                }
                Instr::ToStr { dst, src, .. } => {
                    if releasable(func, bi, *src, 1) {
                        insert.push((bi, ii, *src));
                    }
                    if releasable(func, bi, *dst, 0) {
                        insert.push((bi, ii, *dst));
                    }
                }
                Instr::Call { dsts, .. } => {
                    for d in dsts.iter() {
                        if releasable(func, bi, *d, 0) {
                            insert.push((bi, ii, *d));
                        }
                    }
                }
                _ => {}
            }
        }
    }
    if insert.is_empty() {
        return false;
    }
    insert.sort();
    insert.dedup();
    let func = &mut module.functions[fi];
    for (bi, ii, local) in insert.into_iter().rev() {
        func.blocks[bi].instrs.insert(
            ii + 1,
            Instr::Release { span: UNKNOWN_SPAN, obj: local },
        );
    }
    true
}
