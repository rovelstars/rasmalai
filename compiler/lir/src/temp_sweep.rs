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
            // Boxing a value for an `Any` slot always hands the caller a fresh
            // reference: either a newly allocated box or a retained existing
            // box. Releasing an inline (unboxed) passthrough is a no-op on
            // every backend, so treating the result as fresh is exact.
            | "__rnx_any_box"
    )
}

pub(crate) fn alias_root(func: &Function, mut local: Local) -> Option<Local> {
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

fn fresh_def(module: &Module, ins: &Instr, local: Local) -> bool {
    match ins {
        Instr::Concat { dst, .. } | Instr::ToStr { dst, .. } => *dst == local,
        Instr::ObjNew { dst, .. }
        | Instr::ArrayNew { dst, .. }
        | Instr::EnumNew { dst, .. }
        | Instr::ClosureNew { dst, .. } => *dst == local,
        Instr::GetField { dst, .. }
        | Instr::GetFieldByName { dst, .. }
        | Instr::ArrayGet { dst, .. } => *dst == local,
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
                    // Every function return is an owned value in this
                    // language. Body-based freshness misses precompiled
                    // dependency functions, so the return slot is enough.
                    let _ = id;
                    true
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

pub(crate) fn consuming_builtin(name: &str) -> bool {
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

fn last_owning_use(
    module: &Module,
    func: &Function,
    bi: usize,
    local: Local,
    defs: &BTreeMap<Local, (usize, usize)>,
    def_count: &BTreeMap<Local, usize>,
) -> Option<usize> {
    if !heap_type(func, local) || def_count.get(&local).copied().unwrap_or(0) != 1 {
        return None;
    }
    let (dbi, dii) = *defs.get(&local)?;
    if dbi != bi {
        return None;
    }
    if !fresh_def(module, &func.blocks[bi].instrs[dii], local) {
        return None;
    }
    for (b, block) in func.blocks.iter().enumerate() {
        for ins in &block.instrs {
            if let Instr::Release { obj, .. } = ins {
                if *obj == local {
                    return None;
                }
            }
            if b == bi {
                continue;
            }
            let mut reads = Vec::new();
            crate::opt::instr_reads(ins, &mut reads);
            if reads.contains(&local) {
                return None;
            }
        }
        if b != bi {
            let mut treads = Vec::new();
            term_reads(&block.term, &mut treads);
            if treads.contains(&local) {
                return None;
            }
        }
    }
    let block = &func.blocks[bi];
    let mut last = None;
    for (ii, ins) in block.instrs.iter().enumerate() {
        let mut reads = Vec::new();
        crate::opt::instr_reads(ins, &mut reads);
        if !reads.contains(&local) {
            continue;
        }
        if !matches!(
            ins,
            Instr::SetField { .. }
                | Instr::SetFieldByName { .. }
                | Instr::ArraySet { .. }
                | Instr::ArrayPush { .. }
                | Instr::GetField { .. }
                | Instr::GetFieldByName { .. }
                | Instr::ArrayGet { .. }
                | Instr::ArrayLen { .. }
                | Instr::ArrayPop { .. }
        ) {
            return None;
        }
        last = Some(ii);
    }
    let mut treads = Vec::new();
    term_reads(&block.term, &mut treads);
    if treads.contains(&local) {
        return None;
    }
    match last {
        Some(ii) if ii >= dii => Some(ii),
        _ => None,
    }
}

pub fn sweep_temps(module: &mut Module, fi: usize) -> bool {
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
    let mut released: BTreeSet<Local> = BTreeSet::new();
    let mut reads = Vec::new();
    for block in &func.blocks {
        for ins in &block.instrs {
            if let Instr::Release { obj, .. } = ins {
                released.insert(*obj);
            }
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

    let releasable = |func: &Function, bi: usize, local: Local, expect_uses: usize| -> bool {
        if !heap_type(func, local) || released.contains(&local) {
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
                fresh_def(module, &ins, local)
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
                Instr::Call { dsts, target, args, err, .. } => {
                    for d in dsts.iter() {
                        if releasable(func, bi, *d, 0) {
                            insert.push((bi, ii, *d));
                        }
                    }
                    if let CallTarget::Fn(id) = target {
                        if let Some(recv) = args.first() {
                            if module
                                .functions
                                .get(*id)
                                .is_some_and(|f| f.method_self)
                                && releasable(func, bi, *recv, 1)
                            {
                                let recv_ty = func.locals.get(*recv as usize);
                                let ret = module.functions.get(*id).map(|f| &f.ret);
                                let same_class = matches!(
                                    (recv_ty, ret),
                                    (Some(LirType::Obj(a)), Some(LirType::Obj(b))) if a == b
                                );
                                if !same_class {
                                    insert.push((bi, ii, *recv));
                                }
                            }
                        }
                        if !callee_may_consume(module, *id) {
                            let ret = module.functions.get(*id).map(|f| f.ret.clone());
                            for a in args.iter() {
                                if dsts.contains(a) || *err == Some(*a) {
                                    continue;
                                }
                                if !releasable(func, bi, *a, 1) {
                                    continue;
                                }
                                let aty = func.locals.get(*a as usize);
                                let aliases_ret = match (aty, ret.as_ref()) {
                                    (Some(LirType::Obj(x)), Some(LirType::Obj(y))) => x == y,
                                    (Some(LirType::Str), Some(LirType::Str)) => true,
                                    (Some(LirType::Array(x)), Some(LirType::Array(y))) => x == y,
                                    (Some(LirType::Enum(x)), Some(LirType::Enum(y))) => x == y,
                                    (Some(LirType::Closure), Some(LirType::Closure)) => true,
                                    (Some(LirType::Error), Some(LirType::Error)) => true,
                                    // An `Any` return may carry the argument
                                    // itself (identity closures, boxing
                                    // passthroughs), so releasing the argument
                                    // would free memory the result aliases.
                                    (Some(LirType::Any), Some(LirType::Any)) => true,
                                    (Some(LirType::Any), Some(LirType::Error))
                                    | (Some(LirType::Error), Some(LirType::Any)) => true,
                                    _ => false,
                                };
                                if !aliases_ret {
                                    insert.push((bi, ii, *a));
                                }
                            }
                        }
                    } else if matches!(target, CallTarget::Method { .. } | CallTarget::Dyn { .. } | CallTarget::Value(_)) {
                        let ret = dsts
                            .first()
                            .and_then(|d| func.locals.get(*d as usize))
                            .cloned();
                        for a in args.iter() {
                            if dsts.contains(a) || *err == Some(*a) {
                                continue;
                            }
                            if !releasable(func, bi, *a, 1) {
                                continue;
                            }
                            let aty = func.locals.get(*a as usize);
                            let aliases_ret = match (aty, ret.as_ref()) {
                                (Some(LirType::Obj(x)), Some(LirType::Obj(y))) => x == y,
                                (Some(LirType::Str), Some(LirType::Str)) => true,
                                (Some(LirType::Array(x)), Some(LirType::Array(y))) => x == y,
                                (Some(LirType::Enum(x)), Some(LirType::Enum(y))) => x == y,
                                (Some(LirType::Closure), Some(LirType::Closure)) => true,
                                (Some(LirType::Error), Some(LirType::Error)) => true,
                                // An `Any` return may carry the argument
                                // itself (identity closures, boxing
                                // passthroughs), so releasing the argument
                                // would free memory the result aliases.
                                (Some(LirType::Any), Some(LirType::Any)) => true,
                                (Some(LirType::Any), Some(LirType::Error))
                                | (Some(LirType::Error), Some(LirType::Any)) => true,
                                _ => false,
                            };
                            if !aliases_ret {
                                insert.push((bi, ii, *a));
                            }
                        }
                    } else if let CallTarget::Builtin(name) = target {
                        if !consuming_builtin(name) {
                            let ret = dsts
                                .first()
                                .and_then(|d| func.locals.get(*d as usize))
                                .cloned();
                            for a in args.iter() {
                                if dsts.contains(a) || *err == Some(*a) {
                                    continue;
                                }
                                if !releasable(func, bi, *a, 1) {
                                    continue;
                                }
                                let aty = func.locals.get(*a as usize);
                                let aliases_ret = match (aty, ret.as_ref()) {
                                    (Some(LirType::Obj(x)), Some(LirType::Obj(y))) => x == y,
                                    (Some(LirType::Str), Some(LirType::Str)) => true,
                                    (Some(LirType::Array(x)), Some(LirType::Array(y))) => x == y,
                                    (Some(LirType::Enum(x)), Some(LirType::Enum(y))) => x == y,
                                    (Some(LirType::Closure), Some(LirType::Closure)) => true,
                                    (Some(LirType::Error), Some(LirType::Error)) => true,
                                    (Some(LirType::Any), Some(LirType::Any)) => true,
                                    (Some(LirType::Any), Some(LirType::Error))
                                    | (Some(LirType::Error), Some(LirType::Any)) => true,
                                    _ => false,
                                };
                                if !aliases_ret {
                                    insert.push((bi, ii, *a));
                                }
                            }
                        }
                    }
                }
                Instr::SetField { obj, value, .. } | Instr::SetFieldByName { obj, value, .. } => {
                    for l in [*obj, *value] {
                        if releasable(func, bi, l, 1) {
                            insert.push((bi, ii, l));
                        } else if let Some(last) = last_owning_use(module, func, bi, l, &defs, &def_count) {
                            insert.push((bi, last, l));
                        }
                    }
                }
                Instr::ArraySet { arr, value, .. } | Instr::ArrayPush { arr, value, .. } => {
                    for l in [*arr, *value] {
                        if releasable(func, bi, l, 1) {
                            insert.push((bi, ii, l));
                        } else if let Some(last) = last_owning_use(module, func, bi, l, &defs, &def_count) {
                            insert.push((bi, last, l));
                        }
                    }
                }
                Instr::GetField { obj, .. } | Instr::GetFieldByName { obj, .. } => {
                    if releasable(func, bi, *obj, 1) {
                        insert.push((bi, ii, *obj));
                    }
                }
                Instr::ArrayGet { arr, .. } | Instr::ArrayLen { arr, .. } | Instr::ArrayPop { arr, .. } => {
                    if releasable(func, bi, *arr, 1) {
                        insert.push((bi, ii, *arr));
                    }
                }
                Instr::EnumNew { payload, .. } => {
                    for p in payload {
                        if releasable(func, bi, *p, 1) {
                            insert.push((bi, ii, *p));
                        }
                    }
                }
                Instr::ClosureNew { captures, .. } => {
                    for c in captures {
                        if releasable(func, bi, *c, 1) {
                            insert.push((bi, ii, *c));
                        }
                    }
                }
                _ => {}
            }
        }
    }
    let mut changed = false;
    if !insert.is_empty() {
        insert.sort();
        insert.dedup();
        let func = &mut module.functions[fi];
        for (bi, ii, local) in insert.into_iter().rev() {
            func.blocks[bi].instrs.insert(
                ii + 1,
                Instr::Release { span: UNKNOWN_SPAN, obj: local },
            );
        }
        changed = true;
    }
    changed |= release_dead_temps(module, fi);
    changed |= release_overwrites(module, fi);
    changed
}

fn death_owned_ty(func: &Function, module: &Module, local: Local) -> bool {
    match func.locals.get(local as usize) {
        Some(LirType::Obj(name)) => match module.class_index.get(name) {
            Some(ci) => !module.classes[*ci].is_struct,
            None => false,
        },
        Some(LirType::Str)
        | Some(LirType::Array(_))
        | Some(LirType::Enum(_))
        | Some(LirType::Closure) => true,
        _ => false,
    }
}

fn group_union(parent: &mut BTreeMap<Local, Local>, a: Local, b: Local) {
    let ra = group_find(parent, a);
    let rb = group_find(parent, b);
    if ra != rb {
        parent.insert(ra, rb);
    }
}

fn group_find(parent: &BTreeMap<Local, Local>, mut x: Local) -> Local {
    while let Some(p) = parent.get(&x) {
        if *p == x {
            break;
        }
        x = *p;
    }
    x
}

fn death_int_ptr_ty(func: &Function, local: Local) -> bool {
    matches!(
        func.locals.get(local as usize),
        Some(LirType::I64) | Some(LirType::I8) | Some(LirType::Pointer(_))
    )
}

fn returns_fresh(
    module: &Module,
    fi: usize,
    memo: &mut BTreeMap<usize, bool>,
    stack: &mut Vec<usize>,
) -> bool {
    if let Some(v) = memo.get(&fi) {
        return *v;
    }
    if stack.contains(&fi) {
        // Coinductive assumption: a recursive call returns whatever the
        // function returns. Every non-recursive return is still checked
        // below, so a function that ever returns a borrowed value stays
        // ineligible.
        return true;
    }
    let func = match module.functions.get(fi) {
        Some(f) => f,
        None => return false,
    };
    stack.push(fi);
    let heap_ret = matches!(
        func.ret,
        LirType::Obj(_) | LirType::Str | LirType::Array(_) | LirType::Enum(_) | LirType::Closure
    );
    let mut ok = true;
    let mut seen_ret = false;
    for block in &func.blocks {
        if let Terminator::Ret(v) = &block.term {
            if v.is_empty() {
                if heap_ret {
                    ok = false;
                    break;
                }
                continue;
            }
            seen_ret = true;
            for r in v {
                let fresh = match alias_root(func, *r) {
                    Some(root) => fresh_return_root(module, func, root, memo, stack),
                    None => false,
                };
                if !fresh {
                    ok = false;
                    break;
                }
            }
            if !ok {
                break;
            }
        }
    }
    if !seen_ret {
        ok = false;
    }
    stack.pop();
    memo.insert(fi, ok);
    ok
}

fn fresh_return_root(
    module: &Module,
    func: &Function,
    root: Local,
    memo: &mut BTreeMap<usize, bool>,
    stack: &mut Vec<usize>,
) -> bool {
    let mut count = 0;
    let mut site = None;
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, ins) in block.instrs.iter().enumerate() {
            let mut dsts = Vec::new();
            crate::opt::instr_dsts(ins, &mut dsts);
            if let Instr::Call { err: Some(e), .. } = ins {
                dsts.push(*e);
            }
            if dsts.contains(&root) {
                count += 1;
                site = Some((bi, ii));
            }
        }
        if let Terminator::BrErr { catch_bind, .. } = &block.term {
            if *catch_bind == root {
                count += 1;
            }
        }
    }
    if count != 1 {
        return false;
    }
    let (bi, ii) = match site {
        Some(s) => s,
        None => return false,
    };
    match &func.blocks[bi].instrs[ii] {
        Instr::ObjNew { .. }
        | Instr::ArrayNew { .. }
        | Instr::EnumNew { .. }
        | Instr::ClosureNew { .. }
        | Instr::Concat { .. }
        | Instr::ToStr { .. }
        | Instr::ArrayPop { .. }
        | Instr::GetField { .. }
        | Instr::GetFieldByName { .. }
        | Instr::ArrayGet { .. }
        | Instr::EnumPayload { .. }
        | Instr::GenRefGet { .. } => true,
        Instr::Call { target, .. } => match target {
            CallTarget::Fn(id) => returns_fresh(module, *id, memo, stack),
            CallTarget::Method { method, .. } => returns_fresh(module, *method, memo, stack),
            CallTarget::Builtin(n) => fresh_builtin(n),
            _ => false,
        },
        _ => false,
    }
}

fn own_def_fresh(
    module: &Module,
    func: &Function,
    bi: usize,
    ii: usize,
    memo: &mut BTreeMap<usize, bool>,
) -> bool {
    match &func.blocks[bi].instrs[ii] {
        Instr::ObjNew { .. }
        | Instr::ArrayNew { .. }
        | Instr::EnumNew { .. }
        | Instr::ClosureNew { .. }
        | Instr::Concat { .. }
        | Instr::ToStr { .. }
        | Instr::ArrayPop { .. } => true,
        Instr::Call { target, .. } => match target {
            CallTarget::Fn(id) => returns_fresh(module, *id, memo, &mut Vec::new()),
            CallTarget::Method { method, .. } => returns_fresh(module, *method, memo, &mut Vec::new()),
            CallTarget::Builtin(n) => fresh_builtin(n),
            _ => false,
        },
        _ => false,
    }
}

fn term_successors(term: &Terminator) -> Vec<BlockId> {
    match term {
        Terminator::Ret(_) | Terminator::Unreachable { .. } => Vec::new(),
        Terminator::Br(t) => vec![*t],
        Terminator::BrIf { then_bb, else_bb, .. } => vec![*then_bb, *else_bb],
        Terminator::BrErr { catch_bb, next_bb, .. } => vec![*catch_bb, *next_bb],
        Terminator::Switch { cases, default, .. } => {
            let mut out: Vec<BlockId> = cases.iter().map(|(_, b)| *b).collect();
            out.push(*default);
            out
        }
        Terminator::Throw { catch, .. } => catch.map(|(b, _, _)| vec![b]).unwrap_or_default(),
        Terminator::Rethrow { catch_bb, .. } => vec![*catch_bb],
    }
}

fn walk_defer_defs(ins: &Instr, out: &mut Vec<Local>) {
    if let Instr::Defer { body, .. } = ins {
        for nested in body {
            crate::opt::instr_dsts(nested, out);
            if let Instr::Call { err: Some(e), .. } = nested {
                out.push(*e);
            }
            walk_defer_defs(nested, out);
        }
    }
}

fn release_dead_temps(module: &mut Module, fi: usize) -> bool {
    if !module.functions[fi].locals.iter().any(|t| {
        matches!(
            t,
            LirType::Obj(_) | LirType::Str | LirType::Array(_) | LirType::Enum(_) | LirType::Closure
        )
    }) {
        return false;
    }
    let func = &module.functions[fi];
    let mut parent: BTreeMap<Local, Local> = BTreeMap::new();
    for block in &func.blocks {
        for ins in &block.instrs {
            match ins {
                Instr::Copy { dst, src, .. } | Instr::Cast { dst, src, .. } => {
                    group_union(&mut parent, *dst, *src);
                }
                Instr::Defer { body, .. } => {
                    for nested in body {
                        if let Instr::Copy { dst, src, .. } | Instr::Cast { dst, src, .. } = nested
                        {
                            group_union(&mut parent, *dst, *src);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    let mut mentioned: BTreeSet<Local> = BTreeSet::new();
    let mut reads_buf: Vec<Local> = Vec::new();
    for block in &func.blocks {
        for ins in &block.instrs {
            let mut dsts = Vec::new();
            crate::opt::instr_dsts(ins, &mut dsts);
            walk_defer_defs(ins, &mut dsts);
            mentioned.extend(dsts.iter().copied());
            if let Instr::Call { dsts, err, .. } = ins {
                mentioned.extend(dsts.iter().copied());
                if let Some(e) = err {
                    mentioned.insert(*e);
                }
            }
            reads_buf.clear();
            crate::opt::instr_reads(ins, &mut reads_buf);
            mentioned.extend(reads_buf.iter().copied());
        }
        reads_buf.clear();
        crate::opt::term_reads(&block.term, &mut reads_buf);
        mentioned.extend(reads_buf.iter().copied());
        if let Terminator::BrErr { catch_bind, .. } = &block.term {
            mentioned.insert(*catch_bind);
        }
    }
    let mut groups: BTreeMap<Local, Vec<Local>> = BTreeMap::new();
    for l in mentioned {
        let r = group_find(&parent, l);
        groups.entry(r).or_default().push(l);
    }
    let mut defs: BTreeMap<Local, Vec<(usize, usize, bool)>> = BTreeMap::new();
    let mut defer_touched: BTreeSet<Local> = BTreeSet::new();
    let mut managed: BTreeSet<Local> = BTreeSet::new();
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, ins) in block.instrs.iter().enumerate() {
            let mut dsts = Vec::new();
            crate::opt::instr_dsts(ins, &mut dsts);
            if let Instr::Call { dsts: cd, err, .. } = ins {
                for d in cd {
                    if !dsts.contains(d) {
                        dsts.push(*d);
                    }
                }
                if let Some(e) = err {
                    if !dsts.contains(e) {
                        dsts.push(*e);
                    }
                }
            }
            let copying = matches!(ins, Instr::Copy { .. } | Instr::Cast { .. });
            for d in dsts {
                defs.entry(d).or_default().push((bi, ii, copying));
            }
            let mut nested_dsts = Vec::new();
            walk_defer_defs(ins, &mut nested_dsts);
            for d in nested_dsts {
                defer_touched.insert(d);
                defs.entry(d).or_default().push((bi, ii, false));
            }
            match ins {
                Instr::Retain { obj, .. }
                | Instr::Release { obj, .. }
                | Instr::ReleaseAs { obj, .. } => {
                    managed.insert(*obj);
                }
                _ => {}
            }
        }
        if let Terminator::BrErr { catch_bind, .. } = &block.term {
            defs.entry(*catch_bind).or_default().push((bi, usize::MAX, false));
        }
    }
    let mut pinned_roots: BTreeSet<Local> = BTreeSet::new();
    let mut pin_members = |members: &[Local], l: &Local| {
        if members.contains(l) {
            pinned_roots.insert(group_find(&parent, *l));
        }
    };
    let all_members: Vec<Local> = groups.values().flatten().copied().collect();
    for block in &func.blocks {
        for ins in &block.instrs {
            let mut targets: Vec<Instr> = vec![ins.clone()];
            if let Instr::Defer { body, .. } = ins {
                targets.extend(body.iter().cloned());
            }
            for t in &targets {
                match t {
                    Instr::AddrOf { src, .. } => pin_members(&all_members, src),
                    Instr::Call { dsts, target, args, .. } => {
                        let mut pin = matches!(target, CallTarget::Foreign { .. });
                        if !pin {
                            for d in dsts {
                                if death_int_ptr_ty(func, *d) {
                                    pin = true;
                                    break;
                                }
                            }
                        }
                        if pin {
                            for a in args {
                                pin_members(&all_members, a);
                            }
                        }
                    }
                    Instr::ThreadSpawn { closure, .. } => {
                        if let Some(c) = closure {
                            pin_members(&all_members, c);
                        }
                    }
                    Instr::PoolSubmit { closure, arg, .. } => {
                        if let Some(c) = closure {
                            pin_members(&all_members, c);
                        }
                        if let Some(a) = arg {
                            pin_members(&all_members, a);
                        }
                    }
                    Instr::PoolParallelFor { closure, .. } => {
                        if let Some(c) = closure {
                            pin_members(&all_members, c);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    let pinned: BTreeSet<Local> = pinned_roots
        .iter()
        .flat_map(|r| groups.get(r).cloned().unwrap_or_default())
        .collect();
    let mut memo: BTreeMap<usize, bool> = BTreeMap::new();
    let mut eligible: BTreeMap<Local, Vec<Local>> = BTreeMap::new();
    for (root, members) in &groups {
        if members.iter().any(|m| pinned.contains(m)) {
            continue;
        }
        if members.iter().any(|m| defer_touched.contains(m)) {
            continue;
        }
        if members.iter().any(|m| managed.contains(m)) {
            continue;
        }
        if !members.iter().all(|m| death_owned_ty(func, module, *m)) {
            continue;
        }
        if members.iter().any(|m| (*m as usize) < func.params.len()) {
            continue;
        }
        let mut real: Option<(Local, usize, usize)> = None;
        let mut ok = true;
        for m in members {
            match defs.get(m).map(|v| v.as_slice()) {
                Some([(_, _, true)]) => continue,
                Some([(bi, ii, false)]) => {
                    if real.is_some() {
                        ok = false;
                        break;
                    }
                    real = Some((*m, *bi, *ii));
                }
                _ => {
                    ok = false;
                    break;
                }
            }
        }
        if !ok {
            continue;
        }
        let (_, bi, ii) = match real {
            Some(r) => r,
            None => continue,
        };
        if ii == usize::MAX {
            continue;
        }
        if !own_def_fresh(module, func, bi, ii, &mut memo) {
            continue;
        }
        eligible.insert(*root, members.clone());
    }
    if eligible.is_empty() {
        return false;
    }
    let member_group: BTreeMap<Local, Local> = eligible
        .iter()
        .flat_map(|(r, ms)| ms.iter().map(|m| (*m, *r)))
        .collect();
    let nblocks = func.blocks.len();
    let mut gen_set: Vec<BTreeSet<Local>> = vec![BTreeSet::new(); nblocks];
    let mut kill_set: Vec<BTreeSet<Local>> = vec![BTreeSet::new(); nblocks];
    let mut entry_defs: Vec<BTreeSet<Local>> = vec![BTreeSet::new(); nblocks];
    for (bi, block) in func.blocks.iter().enumerate() {
        for ins in &block.instrs {
            reads_buf.clear();
            crate::opt::instr_reads(ins, &mut reads_buf);
            for r in reads_buf.drain(..) {
                if !kill_set[bi].contains(&r) {
                    gen_set[bi].insert(r);
                }
            }
            let mut dsts = Vec::new();
            crate::opt::instr_dsts(ins, &mut dsts);
            if let Instr::Call { dsts: cd, err, .. } = ins {
                for d in cd {
                    if !dsts.contains(d) {
                        dsts.push(*d);
                    }
                }
                if let Some(e) = err {
                    if !dsts.contains(e) {
                        dsts.push(*e);
                    }
                }
            }
            walk_defer_defs(ins, &mut dsts);
            for d in dsts {
                kill_set[bi].insert(d);
            }
        }
        reads_buf.clear();
        crate::opt::term_reads(&block.term, &mut reads_buf);
        for r in reads_buf.drain(..) {
            if !kill_set[bi].contains(&r) {
                gen_set[bi].insert(r);
            }
        }
        match &block.term {
            Terminator::BrErr { catch_bb, catch_bind, .. } => {
                if *catch_bb < nblocks {
                    entry_defs[*catch_bb].insert(*catch_bind);
                }
            }
            Terminator::Throw { catch: Some((bb, bind, _)), .. } => {
                if *bb < nblocks {
                    entry_defs[*bb].insert(*bind);
                }
            }
            _ => {}
        }
    }
    let mut live_in: Vec<BTreeSet<Local>> = vec![BTreeSet::new(); nblocks];
    let mut live_out: Vec<BTreeSet<Local>> = vec![BTreeSet::new(); nblocks];
    let mut changed_live = true;
    while changed_live {
        changed_live = false;
        for bi in (0..nblocks).rev() {
            let mut out = BTreeSet::new();
            for s in term_successors(&func.blocks[bi].term) {
                if s < nblocks {
                    let mut incoming = live_in[s].clone();
                    for d in &entry_defs[s] {
                        incoming.remove(d);
                    }
                    out.extend(incoming);
                }
            }
            let mut inn = gen_set[bi].clone();
            for l in out.iter() {
                if !kill_set[bi].contains(l) {
                    inn.insert(*l);
                }
            }
            if out != live_out[bi] {
                live_out[bi] = out;
                changed_live = true;
            }
            if inn != live_in[bi] {
                live_in[bi] = inn;
                changed_live = true;
            }
        }
    }
    let mut insert: Vec<(usize, usize, Local)> = Vec::new();
    let mut group_reads: BTreeMap<Local, bool> = BTreeMap::new();
    for r in eligible.keys() {
        group_reads.insert(*r, false);
    }
    for (bi, block) in func.blocks.iter().enumerate() {
        let mut live = live_out[bi].clone();
        reads_buf.clear();
        crate::opt::term_reads(&block.term, &mut reads_buf);
        live.extend(reads_buf.drain(..));
        let mut after: Vec<BTreeSet<Local>> = vec![BTreeSet::new(); block.instrs.len()];
        for ii in (0..block.instrs.len()).rev() {
            after[ii] = live.clone();
            let ins = &block.instrs[ii];
            let mut dsts = Vec::new();
            crate::opt::instr_dsts(ins, &mut dsts);
            if let Instr::Call { dsts: cd, err, .. } = ins {
                for d in cd {
                    if !dsts.contains(d) {
                        dsts.push(*d);
                    }
                }
                if let Some(e) = err {
                    if !dsts.contains(e) {
                        dsts.push(*e);
                    }
                }
            }
            reads_buf.clear();
            crate::opt::instr_reads(ins, &mut reads_buf);
            for r in reads_buf.drain(..) {
                if !dsts.contains(&r) {
                    live.insert(r);
                }
            }
            for d in dsts {
                live.remove(&d);
            }
        }
        for (ii, ins) in block.instrs.iter().enumerate() {
            if matches!(ins, Instr::Panic { .. }) {
                continue;
            }
            reads_buf.clear();
            crate::opt::instr_reads(ins, &mut reads_buf);
            let readers: Vec<Local> = reads_buf.drain(..).collect();
            for m in readers {
                let gid = match member_group.get(&m) {
                    Some(g) => *g,
                    None => continue,
                };
                if let Some(v) = group_reads.get_mut(&gid) {
                    *v = true;
                }
                if matches!(ins, Instr::Copy { src, .. } | Instr::Cast { src, .. } if *src == m)
                {
                    continue;
                }
                let members = match eligible.get(&gid) {
                    Some(ms) => ms,
                    None => continue,
                };
                if members.iter().any(|x| after[ii].contains(x)) {
                    continue;
                }
                insert.push((bi, ii, m));
            }
        }
        reads_buf.clear();
        crate::opt::term_reads(&block.term, &mut reads_buf);
        let term_readers: Vec<Local> = reads_buf.drain(..).collect();
        let term_diverges = matches!(
            block.term,
            Terminator::Ret(_) | Terminator::Throw { .. } | Terminator::Rethrow { .. }
        );
        if !term_diverges {
            for m in term_readers {
                let gid = match member_group.get(&m) {
                    Some(g) => *g,
                    None => continue,
                };
                if let Some(v) = group_reads.get_mut(&gid) {
                    *v = true;
                }
                let members = match eligible.get(&gid) {
                    Some(ms) => ms,
                    None => continue,
                };
                if members.iter().any(|x| live_out[bi].contains(x)) {
                    continue;
                }
                insert.push((bi, block.instrs.len(), m));
            }
        } else {
            for m in term_readers {
                if let Some(gid) = member_group.get(&m) {
                    if let Some(v) = group_reads.get_mut(gid) {
                        *v = true;
                    }
                }
            }
        }
    }
    for (root, members) in &eligible {
        if group_reads.get(root).copied().unwrap_or(true) {
            continue;
        }
        let real = members.iter().find_map(|m| {
            defs.get(m).and_then(|v| match v.as_slice() {
                [(bi, ii, false)] => Some((*m, *bi, *ii)),
                _ => None,
            })
        });
        if let Some((m, bi, ii)) = real {
            if ii != usize::MAX {
                insert.push((bi, ii, m));
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
        let rel = Instr::Release { span: UNKNOWN_SPAN, obj: local };
        if ii >= func.blocks[bi].instrs.len() {
            func.blocks[bi].instrs.push(rel);
        } else {
            func.blocks[bi].instrs.insert(ii + 1, rel);
        }
    }
    let changed = true;
    changed
}

fn overwrite_owned_ty(func: &Function, module: &Module, local: Local) -> bool {    match func.locals.get(local as usize) {
        Some(LirType::Obj(name)) => module.class_index.contains_key(name),
        Some(LirType::Str)
        | Some(LirType::Array(_))
        | Some(LirType::Enum(_))
        | Some(LirType::Closure) => true,
        _ => false,
    }
}

fn instr_dominates(
    dom: &crate::licm::DominatorTree,
    def_pos: (usize, usize),
    use_pos: (usize, usize),
) -> bool {
    if def_pos.0 == use_pos.0 {
        def_pos.1 < use_pos.1
    } else {
        dom.dominates(def_pos.0, use_pos.0)
    }
}

fn term_successors_all(term: &Terminator) -> Vec<BlockId> {
    term_successors(term)
}

fn block_preds(func: &Function) -> Vec<Vec<BlockId>> {
    let mut preds: Vec<Vec<BlockId>> = vec![Vec::new(); func.blocks.len()];
    for (bi, block) in func.blocks.iter().enumerate() {
        for s in term_successors_all(&block.term) {
            if s < preds.len() {
                preds[s].push(bi);
            }
        }
    }
    preds
}

fn instr_def_site(func: &Function, bi: usize, ii: usize) -> Option<(usize, usize)> {
    let ins = &func.blocks[bi].instrs[ii];
    let mut dsts = Vec::new();
    crate::opt::instr_dsts(ins, &mut dsts);
    if let Instr::Call { dsts: cd, err, .. } = ins {
        for d in cd {
            if !dsts.contains(d) {
                dsts.push(*d);
            }
        }
        if let Some(e) = err {
            if !dsts.contains(e) {
                dsts.push(*e);
            }
        }
    }
    if dsts.is_empty() {
        None
    } else {
        Some((bi, ii))
    }
}

fn def_site_local(func: &Function, site: (usize, usize)) -> Local {
    let ins = &func.blocks[site.0].instrs[site.1];
    let mut dsts = Vec::new();
    crate::opt::instr_dsts(ins, &mut dsts);
    if let Instr::Call { dsts: cd, err, .. } = ins {
        for d in cd {
            if !dsts.contains(d) {
                dsts.push(*d);
            }
        }
        if let Some(e) = err {
            if !dsts.contains(e) {
                dsts.push(*e);
            }
        }
    }
    dsts.into_iter().next().unwrap_or(Local::MAX)
}

fn reaching_in_sets(func: &Function, preds: &[Vec<BlockId>]) -> Vec<BTreeSet<(usize, usize)>> {
    let n = func.blocks.len();
    let mut gen_set: Vec<BTreeSet<(usize, usize)>> = vec![BTreeSet::new(); n];
    let mut kill: Vec<BTreeSet<Local>> = vec![BTreeSet::new(); n];
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, ins) in block.instrs.iter().enumerate() {
            if instr_def_site(func, bi, ii).is_some() {
                let _ = ins;
                for d in instr_def_dsts(func, bi, ii) {
                    kill[bi].insert(d);
                }
                gen_set[bi].insert((bi, ii));
            }
        }
    }
    let mut in_sets: Vec<BTreeSet<(usize, usize)>> = vec![BTreeSet::new(); n];
    let mut changed = true;
    while changed {
        changed = false;
        for bi in 0..n {
            let mut inn: BTreeSet<(usize, usize)> = BTreeSet::new();
            for p in &preds[bi] {
                for d in gen_set[*p].iter() {
                    inn.insert(*d);
                }
                for d in in_sets[*p].iter() {
                    if !kill[*p].contains(&def_site_local(func, *d)) {
                        inn.insert(*d);
                    }
                }
            }
            if inn != in_sets[bi] {
                in_sets[bi] = inn;
                changed = true;
            }
        }
    }
    in_sets
}

fn instr_def_dsts(func: &Function, bi: usize, ii: usize) -> Vec<Local> {
    let ins = &func.blocks[bi].instrs[ii];
    let mut dsts = Vec::new();
    crate::opt::instr_dsts(ins, &mut dsts);
    if let Instr::Call { dsts: cd, err, .. } = ins {
        for d in cd {
            if !dsts.contains(d) {
                dsts.push(*d);
            }
        }
        if let Some(e) = err {
            if !dsts.contains(e) {
                dsts.push(*e);
            }
        }
    }
    dsts
}

fn reaching_defs_at(
    func: &Function,
    in_sets: &[BTreeSet<(usize, usize)>],
    local: Local,
    pos: (usize, usize),
) -> BTreeSet<(usize, usize)> {
    let mut out: BTreeSet<(usize, usize)> = BTreeSet::new();
    for d in &in_sets[pos.0] {
        if def_site_local(func, *d) == local {
            out.insert(*d);
        }
    }
    for ii in 0..pos.1.min(func.blocks[pos.0].instrs.len()) {
        if instr_def_dsts(func, pos.0, ii).contains(&local) {
            out.clear();
            out.insert((pos.0, ii));
        }
    }
    out
}

fn unique_reaching_def(
    func: &Function,
    in_sets: &[BTreeSet<(usize, usize)>],
    local: Local,
    pos: (usize, usize),
) -> Option<(usize, usize)> {
    let set = reaching_defs_at(func, in_sets, local, pos);
    if set.len() == 1 {
        set.into_iter().next()
    } else {
        None
    }
}

fn overwrite_int_ptr_ty(func: &Function, local: Local) -> bool {
    matches!(
        func.locals.get(local as usize),
        Some(LirType::I64) | Some(LirType::I8) | Some(LirType::Pointer(_))
    )
}

fn chain_fresh(
    module: &Module,
    func: &Function,
    in_sets: &[BTreeSet<(usize, usize)>],
    dom: &crate::licm::DominatorTree,
    innermost: &[Option<usize>],
    target: (usize, usize),
    memo: &mut BTreeMap<usize, bool>,
) -> bool {
    let mut cur = target;
    let mut seen: BTreeSet<(usize, usize)> = BTreeSet::new();
    loop {
        if !seen.insert(cur) {
            return false;
        }
        if cur != target {
            if innermost[cur.0] != innermost[target.0] {
                return false;
            }
            if !instr_dominates(dom, cur, target) {
                return false;
            }
        }
        let ins = &func.blocks[cur.0].instrs[cur.1];
        match ins {
            Instr::ObjNew { .. }
            | Instr::ArrayNew { .. }
            | Instr::EnumNew { .. }
            | Instr::ClosureNew { .. }
            | Instr::Concat { .. }
            | Instr::ToStr { .. }
            | Instr::ArrayPop { .. } => return true,
            Instr::Call { .. } => {
                return own_def_fresh(module, func, cur.0, cur.1, memo);
            }
            Instr::Copy { dst, src, .. } | Instr::Cast { dst, src, .. } => {
                if !overwrite_owned_ty(func, module, *dst)
                    || !overwrite_owned_ty(func, module, *src)
                {
                    return false;
                }
                let next = match unique_reaching_def(func, in_sets, *src, cur) {
                    Some(n) => n,
                    None => return false,
                };
                cur = next;
            }
            _ => return false,
        }
    }
}

fn release_overwrites(module: &mut Module, fi: usize) -> bool {
    let func = &module.functions[fi];
    if !func.locals.iter().any(|t| {
        matches!(
            t,
            LirType::Obj(_) | LirType::Str | LirType::Array(_) | LirType::Enum(_) | LirType::Closure
        )
    }) {
        return false;
    }
    let dom = crate::licm::compute_dominators(func);
    let loops = crate::licm::find_natural_loops(func, &dom);
    if loops.is_empty() {
        return false;
    }
    let mut in_loop = vec![false; func.blocks.len()];
    let mut inner: Vec<Option<usize>> = vec![None; func.blocks.len()];
    for (i, lp) in loops.iter().enumerate() {
        for b in &lp.blocks {
            in_loop[*b] = true;
            let replace = match inner[*b] {
                None => true,
                Some(j) => loops[j].blocks.len() > lp.blocks.len(),
            };
            if replace {
                inner[*b] = Some(i);
            }
        }
    }
    let preds = block_preds(func);
    let in_sets = reaching_in_sets(func, &preds);
    let mut defs: BTreeMap<Local, Vec<(usize, usize)>> = BTreeMap::new();
    let mut copy_src: BTreeSet<Local> = BTreeSet::new();
    let mut managed: BTreeSet<Local> = BTreeSet::new();
    let mut defer_touched: BTreeSet<Local> = BTreeSet::new();
    let mut pinned: BTreeSet<Local> = BTreeSet::new();
    let mut reads_buf: Vec<Local> = Vec::new();
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, ins) in block.instrs.iter().enumerate() {
            for d in instr_def_dsts(func, bi, ii) {
                defs.entry(d).or_default().push((bi, ii));
            }
            match ins {
                Instr::Copy { dst: _, src, .. } | Instr::Cast { dst: _, src, .. } => {
                    copy_src.insert(*src);
                }
                Instr::Retain { obj, .. }
                | Instr::Release { obj, .. }
                | Instr::ReleaseAs { obj, .. } => {
                    managed.insert(*obj);
                }
                Instr::AddrOf { src, .. } => {
                    pinned.insert(*src);
                }
                Instr::Call { dsts, target, args, .. } => {
                    let mut pin = matches!(target, CallTarget::Foreign { .. });
                    if !pin {
                        for d in dsts {
                            if overwrite_int_ptr_ty(func, *d) {
                                pin = true;
                                break;
                            }
                        }
                    }
                    if pin {
                        for a in args {
                            pinned.insert(*a);
                        }
                    }
                }
                Instr::ThreadSpawn { closure, .. } => {
                    if let Some(c) = closure {
                        pinned.insert(*c);
                    }
                }
                Instr::PoolSubmit { closure, arg, .. } => {
                    if let Some(c) = closure {
                        pinned.insert(*c);
                    }
                    if let Some(a) = arg {
                        pinned.insert(*a);
                    }
                }
                Instr::PoolParallelFor { closure, .. } => {
                    if let Some(c) = closure {
                        pinned.insert(*c);
                    }
                }
                Instr::Defer { body, .. } => {
                    for nested in body {
                        reads_buf.clear();
                        crate::opt::instr_reads(nested, &mut reads_buf);
                        for r in reads_buf.drain(..) {
                            defer_touched.insert(r);
                        }
                        if let Instr::Copy { dst, src, .. } | Instr::Cast { dst, src, .. } =
                            nested
                        {
                            copy_src.insert(*src);
                            defer_touched.insert(*dst);
                        } else {
                            let mut dsts = Vec::new();
                            crate::opt::instr_dsts(nested, &mut dsts);
                            for d in dsts {
                                defer_touched.insert(d);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    let mut memo: BTreeMap<usize, bool> = BTreeMap::new();
    let mut insert: Vec<(usize, usize, Local)> = Vec::new();
    for (m, dsites) in &defs {
        let m = *m;
        if !overwrite_owned_ty(func, module, m) {
            continue;
        }
        if (m as usize) < func.params.len() {
            continue;
        }
        if copy_src.contains(&m) {
            continue;
        }
        if managed.contains(&m) || pinned.contains(&m) || defer_touched.contains(&m) {
            continue;
        }
        for (bi, ii) in dsites {
            let (bi, ii) = (*bi, *ii);
            if !in_loop[bi] {
                continue;
            }
            let def_ins = &func.blocks[bi].instrs[ii];
            match def_ins {
                Instr::Copy { .. } | Instr::Cast { .. } => {}
                _ => continue,
            }
            if !chain_fresh(module, func, &in_sets, &dom, &inner, (bi, ii), &mut memo) {
                continue;
            }
            let mut ok = true;
            let mut rds: Vec<Local> = Vec::new();
            for (rbi, block) in func.blocks.iter().enumerate() {
                crate::opt::term_reads(&block.term, &mut rds);
                for r in rds.drain(..) {
                    if r == m && !instr_dominates(&dom, (bi, ii), (rbi, block.instrs.len())) {
                        ok = false;
                        break;
                    }
                }
                if !ok {
                    break;
                }
                for (rii, ins) in block.instrs.iter().enumerate() {
                    rds.clear();
                    crate::opt::instr_reads(ins, &mut rds);
                    for r in rds.drain(..) {
                        if r == m && !instr_dominates(&dom, (bi, ii), (rbi, rii)) {
                            ok = false;
                            break;
                        }
                    }
                    if !ok {
                        break;
                    }
                }
                if !ok {
                    break;
                }
            }
            if !ok {
                continue;
            }
            insert.push((bi, ii, m));
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
            ii,
            Instr::Release { span: UNKNOWN_SPAN, obj: local },
        );
    }
    true
}
