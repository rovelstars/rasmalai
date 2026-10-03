use crate::instr::*;
use crate::licm::{DominatorTree, NaturalLoop};
use crate::opt::instr_dst;
use std::collections::{BTreeMap, BTreeSet};

fn all_defs(func: &Function, local: Local) -> Vec<(BlockId, usize)> {
    let mut out: Vec<(BlockId, usize)> = Vec::new();
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, ins) in block.instrs.iter().enumerate() {
            if instr_dst(ins) == Some(local) {
                out.push((bi, ii));
            }
            if let Instr::Call { dsts, err, .. } = ins {
                if dsts.contains(&local) || *err == Some(local) {
                    out.push((bi, ii));
                }
            }
            if let Instr::Defer {  body , ..} = ins {
                for nested in body {
                    if instr_dst(nested) == Some(local) {
                        out.push((bi, ii));
                    }
                }
            }
        }
        if let Terminator::BrErr { catch_bind, .. } = &block.term {
            if *catch_bind == local {
                out.push((bi, usize::MAX));
            }
        }
    }
    out
}

fn term_targets(term: &Terminator) -> Vec<BlockId> {
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

fn header_check(func: &Function, header: BlockId) -> Option<(Local, Local, BlockId)> {
    let block = func.blocks.get(header)?;
    let Terminator::BrIf { cond, then_bb, .. } = &block.term else {
        return None;
    };
    for ins in &block.instrs {
        match ins {
            Instr::Cmp { op: CmpOp::Lt, lhs, rhs, dst, .. } if *dst == *cond => {
                return Some((*lhs, *rhs, *then_bb));
            }
            Instr::Cmp { op: CmpOp::Gt, lhs, rhs, dst, .. } if *dst == *cond => {
                return Some((*rhs, *lhs, *then_bb));
            }
            _ => {}
        }
    }
    None
}

fn resolve_init(
    func: &Function,
    blocks: &BTreeSet<BlockId>,
    iv: Local,
) -> Option<(BlockId, i64)> {
    let mut outside: Vec<(BlockId, usize)> = all_defs(func, iv)
        .into_iter()
        .filter(|(bi, _)| !blocks.contains(bi))
        .collect();
    if outside.len() != 1 {
        return None;
    }
    let mut seen: BTreeSet<Local> = BTreeSet::from([iv]);
    let (mut bi, mut ii) = outside.pop().unwrap();
    loop {
        match func.blocks.get(bi)?.instrs.get(ii)? {
            Instr::Const { lit: Lit::Int(n), .. } => {
                return if *n >= 0 { Some((bi, *n)) } else { None };
            }
            Instr::Copy { src, .. } => {
                if !seen.insert(*src) {
                    return None;
                }
                let defs = all_defs(func, *src);
                if defs.len() != 1 || blocks.contains(&defs[0].0) {
                    return None;
                }
                bi = defs[0].0;
                ii = defs[0].1;
            }
            _ => return None,
        }
    }
}

fn const_positive(func: &Function, local: Local) -> bool {
    let defs = all_defs(func, local);
    if defs.len() != 1 {
        return false;
    }
    let (bi, ii) = defs[0];
    match func.blocks.get(bi).and_then(|b| b.instrs.get(ii)) {
        Some(Instr::Const { lit: Lit::Int(n), .. }) => *n > 0,
        _ => false,
    }
}

fn increment_at(
    func: &Function,
    blocks: &BTreeSet<BlockId>,
    iv: Local,
) -> Option<(BlockId, usize)> {
    let defs = all_defs(func, iv);
    if defs.len() != 2 {
        return None;
    }
    let in_loop: Vec<(BlockId, usize)> = defs
        .into_iter()
        .filter(|(bi, _)| blocks.contains(bi))
        .collect();
    if in_loop.len() != 1 {
        return None;
    }
    Some(in_loop[0])
}

fn increment_ok(func: &Function, bi: BlockId, ii: usize, iv: Local) -> bool {
    let block = match func.blocks.get(bi) {
        Some(b) => b,
        None => return false,
    };
    match block.instrs.get(ii) {
        Some(Instr::Copy {  dst, src , ..}) if *dst == iv => {
            let tdefs = all_defs(func, *src);
            if tdefs.len() != 1 {
                return false;
            }
            let (tbi, tii) = tdefs[0];
            match func.blocks.get(tbi).and_then(|b| b.instrs.get(tii)) {
                Some(Instr::Arith { op: ArithOp::Add, lhs, rhs, .. }) => {
                    (*lhs == iv && const_positive(func, *rhs))
                        || (*rhs == iv && const_positive(func, *lhs))
                }
                _ => false,
            }
        }
        Some(Instr::Arith { op: ArithOp::Add, lhs, rhs, dst, .. }) if *dst == iv => {
            (*lhs == iv && const_positive(func, *rhs))
                || (*rhs == iv && const_positive(func, *lhs))
        }
        _ => false,
    }
}

fn resolve_len(func: &Function, mut local: Local) -> Option<Local> {
    let mut seen: BTreeSet<Local> = BTreeSet::new();
    loop {
        if !seen.insert(local) {
            return None;
        }
        let defs = all_defs(func, local);
        if defs.len() != 1 {
            return None;
        }
        let (bi, ii) = defs[0];
        match func.blocks.get(bi).and_then(|b| b.instrs.get(ii)) {
            Some(Instr::ArrayLen { arr, .. }) => return Some(*arr),
            Some(Instr::Copy { src, .. }) => local = *src,
            _ => return None,
        }
    }
}

fn defines_in(instrs: &[Instr], local: Local) -> bool {
    for ins in instrs {
        if instr_dst(ins) == Some(local) {
            return true;
        }
        if let Instr::Call { dsts, err, .. } = ins {
            if dsts.contains(&local) || *err == Some(local) {
                return true;
            }
        }
        if let Instr::Defer {  body , ..} = ins {
            if defines_in(body, local) {
                return true;
            }
        }
    }
    false
}

fn clean_reachable(
    func: &Function,
    header: BlockId,
    then_bb: BlockId,
    iv: Local,
    target: BlockId,
    prefix: usize,
) -> bool {
    if then_bb == header {
        return false;
    }
    let n = func.blocks.len();
    let mut clean: BTreeSet<BlockId> = BTreeSet::new();
    let mut dirty: BTreeSet<BlockId> = BTreeSet::new();
    let mut stack: Vec<(BlockId, bool)> = vec![(then_bb, false)];
    let mut seen: BTreeSet<(BlockId, bool)> = BTreeSet::new();
    while let Some((b, d)) = stack.pop() {
        if b == header || b >= n || !seen.insert((b, d)) {
            continue;
        }
        let mut dirty_here = d;
        if b != target && defines_in(&func.blocks[b].instrs, iv) {
            dirty_here = true;
        }
        if dirty_here {
            dirty.insert(b);
        } else {
            clean.insert(b);
        }
        for t in term_targets(&func.blocks[b].term) {
            stack.push((t, dirty_here));
        }
    }
    if dirty.contains(&target) || !clean.contains(&target) {
        return false;
    }
    let instrs = &func.blocks[target].instrs;
    !defines_in(&instrs[..prefix.min(instrs.len())], iv)
}

fn external_preds(func: &Function, header: BlockId, blocks: &BTreeSet<BlockId>) -> Vec<BlockId> {
    let n = func.blocks.len();
    let mut out: Vec<BlockId> = Vec::new();
    for (i, block) in func.blocks.iter().enumerate() {
        if blocks.contains(&i) {
            continue;
        }
        if i < n && term_targets(&block.term).contains(&header) {
            out.push(i);
        }
    }
    out.sort();
    out.dedup();
    out
}

fn elide_loop(func: &mut Function, dom: &DominatorTree, lp: &NaturalLoop) -> bool {
    let header = lp.header;
    let (iv, bound, then_bb) = match header_check(func, header) {
        Some(v) => v,
        None => return false,
    };
    let then_preds: Vec<BlockId> = {
        let n = func.blocks.len();
        let mut out: Vec<BlockId> = Vec::new();
        for (i, block) in func.blocks.iter().enumerate() {
            if i < n && term_targets(&block.term).contains(&then_bb) {
                out.push(i);
            }
        }
        out
    };
    if then_preds != vec![header] {
        return false;
    }
    let arr = match resolve_len(func, bound) {
        Some(a) => a,
        None => return false,
    };
    for b in &lp.blocks {
        for ins in &func.blocks[*b].instrs {
            if let Instr::ArrayPush { .. } | Instr::ArrayPop { .. } = ins {
                return false;
            }
        }
    }
    if all_defs(func, arr).iter().any(|(bi, _)| lp.blocks.contains(bi)) {
        return false;
    }
    let (inc_bi, inc_ii) = match increment_at(func, &lp.blocks, iv) {
        Some(v) => v,
        None => return false,
    };
    if !increment_ok(func, inc_bi, inc_ii, iv) {
        return false;
    }
    let (init_block, _) = match resolve_init(func, &lp.blocks, iv) {
        Some(v) => v,
        None => return false,
    };
    let externals = external_preds(func, header, &lp.blocks);
    if externals.is_empty() {
        return false;
    }
    if !externals.iter().all(|e| dom.dominates(init_block, *e)) {
        return false;
    }
    let mut targets: Vec<(BlockId, usize)> = Vec::new();
    for b in &lp.blocks {
        if *b == header {
            continue;
        }
        if !dom.dominates(then_bb, *b) {
            continue;
        }
        for (ii, ins) in func.blocks[*b].instrs.iter().enumerate() {
            let is_access = match ins {
                Instr::ArrayGet { arr: a, index: x, .. } => *a == arr && *x == iv,
                Instr::ArraySet { arr: a, index: x, .. } => *a == arr && *x == iv,
                _ => false,
            };
            if !is_access {
                continue;
            }
            if !clean_reachable(func, header, then_bb, iv, *b, ii) {
                continue;
            }
            targets.push((*b, ii));
        }
    }
    let mut changed = false;
    for (b, ii) in targets {
        let block = &mut func.blocks[b];
        match &mut block.instrs[ii] {
            Instr::ArrayGet { unchecked, .. } => *unchecked = true,
            Instr::ArraySet { unchecked, .. } => *unchecked = true,
            _ => {}
        }
        changed = true;
    }
    changed
}

pub fn eliminate_bounds_checks(
    func: &mut Function,
    dom: &DominatorTree,
    loops: &[NaturalLoop],
) -> bool {
    let mut ordered: Vec<&NaturalLoop> = loops.iter().collect();
    ordered.sort_by_key(|l| l.blocks.len());
    let mut changed = false;
    for lp in ordered {
        if elide_loop(func, dom, lp) {
            changed = true;
        }
    }
    changed |= elide_affine_loops(func, dom, loops);
    changed
}

fn const_of(func: &Function, local: Local) -> Option<i64> {
    let mut seen: BTreeSet<Local> = BTreeSet::new();
    let mut cur = local;
    loop {
        if !seen.insert(cur) {
            return None;
        }
        let defs = all_defs(func, cur);
        if defs.len() != 1 {
            return None;
        }
        let (bi, ii) = defs[0];
        match func.blocks.get(bi)?.instrs.get(ii)? {
            Instr::Const { lit: Lit::Int(n), .. } => return Some(*n),
            Instr::Copy { src, .. } => cur = *src,
            _ => return None,
        }
    }
}

#[derive(Clone)]
enum InitExpr {
    Const(i64),
    OuterAdd { outer: Local, add: i64 },
}

fn init_shape(func: &Function, local: Local, depth: usize) -> Option<InitExpr> {
    if depth > 8 {
        return None;
    }
    if let Some(c) = const_of(func, local) {
        return Some(InitExpr::Const(c));
    }
    let defs = all_defs(func, local);
    if defs.len() != 1 {
        return Some(InitExpr::OuterAdd { outer: local, add: 0 });
    }
    let (bi, ii) = defs[0];
    match func.blocks.get(bi)?.instrs.get(ii)? {
        Instr::Const { lit: Lit::Int(c), .. } => Some(InitExpr::Const(*c)),
        Instr::Copy { src, .. } => init_shape(func, *src, depth + 1),
        Instr::Arith { op: ArithOp::Add, lhs, rhs, .. } => {
            if let Some(c) = const_of(func, *lhs) {
                Some(InitExpr::OuterAdd { outer: *rhs, add: c })
            } else {
                const_of(func, *rhs).map(|c| InitExpr::OuterAdd { outer: *lhs, add: c })
            }
        }
        _ => None,
    }
}

struct CountedLoop {
    header: BlockId,
    then_bb: BlockId,
    iv: Local,
    init: InitExpr,
    bound: i64,
    blocks: BTreeSet<BlockId>,
}

fn loop_step_value(func: &Function, blocks: &BTreeSet<BlockId>, iv: Local) -> Option<i64> {
    let defs = all_defs(func, iv);
    if defs.len() != 2 {
        return None;
    }
    let in_loop: Vec<(BlockId, usize)> = defs
        .into_iter()
        .filter(|(bi, _)| blocks.contains(bi))
        .collect();
    if in_loop.len() != 1 {
        return None;
    }
    let (bi, ii) = in_loop[0];
    let step_of = |l: Local| const_of(func, l);
    match func.blocks.get(bi)?.instrs.get(ii)? {
        Instr::Copy {  dst, src , ..} if *dst == iv => {
            let tdefs = all_defs(func, *src);
            if tdefs.len() != 1 {
                return None;
            }
            let (tbi, tii) = tdefs[0];
            match func.blocks.get(tbi)?.instrs.get(tii)? {
                Instr::Arith { op: ArithOp::Add, lhs, rhs, .. } => {
                    if *lhs == iv {
                        step_of(*rhs)
                    } else if *rhs == iv {
                        step_of(*lhs)
                    } else {
                        None
                    }
                }
                _ => None,
            }
        }
        Instr::Arith { op: ArithOp::Add, lhs, rhs, dst, .. } if *dst == iv => {
            if *lhs == iv {
                step_of(*rhs)
            } else if *rhs == iv {
                step_of(*lhs)
            } else {
                None
            }
        }
        _ => None,
    }
}

fn analyze_counted(func: &Function, lp: &NaturalLoop) -> Option<CountedLoop> {
    let (iv, bound_local, then_bb) = header_check(func, lp.header)?;
    let n = func.blocks.len();
    let mut then_preds: Vec<BlockId> = Vec::new();
    for (i, block) in func.blocks.iter().enumerate() {
        if i < n && term_targets(&block.term).contains(&then_bb) {
            then_preds.push(i);
        }
    }
    if then_preds != vec![lp.header] {
        return None;
    }
    let bound = const_of(func, bound_local)?;
    if bound <= 0 {
        return None;
    }
    if loop_step_value(func, &lp.blocks, iv)? != 1 {
        return None;
    }
    let outside: Vec<(BlockId, usize)> = all_defs(func, iv)
        .into_iter()
        .filter(|(bi, _)| !lp.blocks.contains(bi))
        .collect();
    if outside.len() != 1 {
        return None;
    }
    let (bi, ii) = outside[0];
    let init = match func.blocks.get(bi)?.instrs.get(ii)? {
        Instr::Const { lit: Lit::Int(c), .. } => InitExpr::Const(*c),
        Instr::Copy { src, .. } => init_shape(func, *src, 0)?,
        Instr::Arith { op: ArithOp::Add, lhs, rhs, .. } => {
            if let Some(c) = const_of(func, *lhs) {
                InitExpr::OuterAdd { outer: *rhs, add: c }
            } else {
                let c = const_of(func, *rhs)?;
                InitExpr::OuterAdd { outer: *lhs, add: c }
            }
        }
        _ => return None,
    };
    Some(CountedLoop {
        header: lp.header,
        then_bb,
        iv,
        init,
        bound,
        blocks: lp.blocks.clone(),
    })
}

fn iv_range(nest: &[&CountedLoop], iv: Local) -> Option<(i128, i128)> {
    iv_range_depth(nest, iv, 0)
}

fn iv_range_depth(nest: &[&CountedLoop], iv: Local, depth: usize) -> Option<(i128, i128)> {
    if depth > 16 {
        return None;
    }
    let pos = nest.iter().rposition(|l| l.iv == iv)?;
    let l = nest[pos];
    let lo = match &l.init {
        InitExpr::Const(c) => *c as i128,
        InitExpr::OuterAdd { outer, add } => {
            let (olo, _) = iv_range_depth(&nest[..pos], *outer, depth + 1)?;
            olo.checked_add(*add as i128)?
        }
    };
    let hi = l.bound as i128 - 1;
    if lo > hi {
        return None;
    }
    Some((lo, hi))
}

fn idx_interval(
    func: &Function,
    nest: &[&CountedLoop],
    dom: &DominatorTree,
    access_b: BlockId,
    access_ii: usize,
    local: Local,
    depth: usize,
) -> Option<(i128, i128, BTreeSet<Local>)> {
    if depth > 32 {
        return None;
    }
    if let Some((lo, hi)) = iv_range(nest, local) {
        return Some((lo, hi, BTreeSet::from([local])));
    }
    let defs = all_defs(func, local);
    if defs.len() != 1 {
        return None;
    }
    let (bi, ii) = defs[0];
    let dominates = (bi == access_b && ii < access_ii)
        || (bi != access_b && dom.dominates(bi, access_b));
    if !dominates {
        return None;
    }
    match func.blocks.get(bi)?.instrs.get(ii)? {
        Instr::Const { lit: Lit::Int(c), .. } => Some((*c as i128, *c as i128, BTreeSet::new())),
        Instr::Copy { src, .. } => idx_interval(func, nest, dom, access_b, access_ii, *src, depth + 1),
        Instr::Arith { op: ArithOp::Add, lhs, rhs, .. } => {
            let (a_lo, a_hi, mut a_ivs) = idx_interval(func, nest, dom, access_b, access_ii, *lhs, depth + 1)?;
            let (b_lo, b_hi, b_ivs) = idx_interval(func, nest, dom, access_b, access_ii, *rhs, depth + 1)?;
            a_ivs.extend(b_ivs);
            Some((a_lo.checked_add(b_lo)?, a_hi.checked_add(b_hi)?, a_ivs))
        }
        Instr::Arith { op: ArithOp::Sub, lhs, rhs, .. } => {
            let (a_lo, a_hi, mut a_ivs) = idx_interval(func, nest, dom, access_b, access_ii, *lhs, depth + 1)?;
            let (b_lo, b_hi, b_ivs) = idx_interval(func, nest, dom, access_b, access_ii, *rhs, depth + 1)?;
            if !b_ivs.is_empty() {
                return None;
            }
            a_ivs.extend(b_ivs);
            Some((a_lo.checked_sub(b_hi)?, a_hi.checked_sub(b_lo)?, a_ivs))
        }
        Instr::Arith { op: ArithOp::Mul, lhs, rhs, .. } => {
            let (a_lo, a_hi, mut a_ivs) = idx_interval(func, nest, dom, access_b, access_ii, *lhs, depth + 1)?;
            let (b_lo, b_hi, b_ivs) = idx_interval(func, nest, dom, access_b, access_ii, *rhs, depth + 1)?;
            if a_lo < 0 || b_lo < 0 {
                return None;
            }
            a_ivs.extend(b_ivs);
            let corners = [
                a_lo.checked_mul(b_lo)?,
                a_lo.checked_mul(b_hi)?,
                a_hi.checked_mul(b_lo)?,
                a_hi.checked_mul(b_hi)?,
            ];
            Some((*corners.iter().min()?, *corners.iter().max()?, a_ivs))
        }
        _ => None,
    }
}

fn loop_latches(func: &Function, lp_blocks: &BTreeSet<BlockId>, header: BlockId) -> Vec<BlockId> {
    let mut out = Vec::new();
    for b in lp_blocks {
        if *b == header {
            continue;
        }
        if let Some(block) = func.blocks.get(*b)
            && term_targets(&block.term).contains(&header)
        {
            out.push(*b);
        }
    }
    out.sort();
    out.dedup();
    out
}

fn nest_has_exits(func: &Function, union_blocks: &BTreeSet<BlockId>, headers: &BTreeSet<BlockId>) -> bool {
    for b in union_blocks {
        let block = match func.blocks.get(*b) {
            Some(x) => x,
            None => return true,
        };
        match &block.term {
            Terminator::Ret(_)
            | Terminator::Throw { .. }
            | Terminator::Rethrow { .. }
            | Terminator::Unreachable { .. }
            | Terminator::BrErr { .. } => return true,
            _ => {}
        }
        for t in term_targets(&block.term) {
            if union_blocks.contains(&t) {
                continue;
            }
            if headers.contains(b) {
                continue;
            }
            return true;
        }
    }
    false
}

fn array_aliases(func: &Function, arr: Local) -> BTreeSet<Local> {
    let mut set: BTreeSet<Local> = BTreeSet::from([arr]);
    loop {
        let mut grown = false;
        for block in &func.blocks {
            for ins in &block.instrs {
                if let Instr::Copy {  dst, src , ..} = ins {
                    if set.contains(dst) && set.insert(*src) {
                        grown = true;
                    }
                    if set.contains(src) && set.insert(*dst) {
                        grown = true;
                    }
                }
            }
        }
        if !grown {
            break;
        }
    }
    set
}

fn call_passes_array(func: &Function, union_blocks: &BTreeSet<BlockId>) -> bool {
    let mut arrays: BTreeSet<Local> = BTreeSet::new();
    for (i, t) in func.locals.iter().enumerate() {
        if matches!(t, LirType::Array(_)) {
            arrays.insert(i as Local);
        }
    }
    for b in union_blocks {
        let block = match func.blocks.get(*b) {
            Some(x) => x,
            None => continue,
        };
        for ins in &block.instrs {
            if let Instr::Call { args, .. } = ins {
                for a in args {
                    if arrays.contains(a) {
                        return true;
                    }
                    for arr in array_aliases(func, *a) {
                        if arrays.contains(&arr) {
                            return true;
                        }
                    }
                }
            }
        }
    }
    false
}

fn elide_affine_loops(func: &mut Function, dom: &DominatorTree, loops: &[NaturalLoop]) -> bool {
    let infos: Vec<CountedLoop> = loops.iter().filter_map(|lp| analyze_counted(func, lp)).collect();
    if infos.is_empty() {
        return false;
    }
    let mut ordered: Vec<&CountedLoop> = infos.iter().collect();
    ordered.sort_by_key(|l| l.blocks.len());
    let mut changed = false;
    let mut guarded: BTreeSet<(Local, BlockId, i64)> = BTreeSet::new();
    let mut dummies: BTreeSet<(BlockId, usize)> = BTreeSet::new();
    for lp in ordered {
        let mut nest: Vec<&CountedLoop> = infos
            .iter()
            .filter(|o| o.blocks.is_superset(&lp.blocks))
            .collect();
        nest.sort_by_key(|l| std::cmp::Reverse(l.blocks.len()));
        let union_blocks: BTreeSet<BlockId> = nest.iter().flat_map(|l| l.blocks.iter().copied()).collect();
        let headers: BTreeSet<BlockId> = nest.iter().map(|l| l.header).collect();
        if nest_has_exits(func, &union_blocks, &headers) {
            continue;
        }
        let mut bad = false;
        for b in &union_blocks {
            for ins in &func.blocks[*b].instrs {
                if let Instr::ArrayPush { .. } | Instr::ArrayPop { .. } = ins {
                    bad = true;
                }
            }
        }
        if bad || call_passes_array(func, &union_blocks) {
            continue;
        }
        let mut cands: Vec<(BlockId, usize, Local, usize, i64)> = Vec::new();
        for b in &lp.blocks {
            if headers.contains(b) {
                continue;
            }
            if !dom.dominates(lp.then_bb, *b) {
                continue;
            }
            let latches = loop_latches(func, &lp.blocks, lp.header);
            if latches.is_empty() || !latches.iter().all(|l| dom.dominates(*b, *l)) {
                continue;
            }
            for (ii, ins) in func.blocks[*b].instrs.iter().enumerate() {
                if dummies.contains(&(*b, ii)) {
                    continue;
                }
                let (arr, idx, size) = match ins {
                    Instr::ArrayGet { arr, index, elem_size, unchecked: false, .. } => (*arr, *index, *elem_size),
                    Instr::ArraySet { arr, index, elem_size, unchecked: false, .. } => (*arr, *index, *elem_size),
                    _ => continue,
                };
                if !matches!(func.locals.get(arr as usize), Some(LirType::Array(_))) {
                    continue;
                }
                if all_defs(func, arr).iter().any(|(db, _)| union_blocks.contains(db)) {
                    continue;
                }
                let (lo, hi, ivs) = match idx_interval(func, &nest, dom, *b, ii, idx, 0) {
                    Some(v) => v,
                    None => continue,
                };
                if lo < 0 {
                    continue;
                }
                if ivs.len() >= 2 {
                    let mut ok = true;
                    for v in &ivs {
                        let mut found = false;
                        for l in nest.iter() {
                            if l.iv == *v {
                                found = true;
                                if !matches!(l.init, InitExpr::Const(_)) {
                                    ok = false;
                                }
                            }
                        }
                        if !found {
                            ok = false;
                        }
                    }
                    if !ok {
                        continue;
                    }
                }
                let k = match hi.checked_add(1) {
                    Some(k) if k > 0 && k <= i64::MAX as i128 => k as i64,
                    _ => continue,
                };
                let mut reach_ok = true;
                for v in &ivs {
                    let mut done = false;
                    for l in nest.iter() {
                        if l.iv == *v {
                            if !clean_reachable(func, l.header, l.then_bb, *v, *b, ii) {
                                reach_ok = false;
                            }
                            done = true;
                            break;
                        }
                    }
                    if !done {
                        reach_ok = false;
                    }
                    if !reach_ok {
                        break;
                    }
                }
                if !reach_ok {
                    continue;
                }
                cands.push((*b, ii, arr, size, k));
            }
        }
        if cands.is_empty() {
            continue;
        }
        let mut by_arr: BTreeMap<Local, (i64, usize)> = BTreeMap::new();
        for (_, _, arr, size, k) in &cands {
            let e = by_arr.entry(*arr).or_insert((0, *size));
            if *k > e.0 {
                e.0 = *k;
            }
        }
        let externals = external_preds(func, lp.header, &lp.blocks);
        if externals.len() != 1 {
            continue;
        }
        let pred = externals[0];
        let pred_ok = match func.blocks.get(pred).map(|b| &b.term) {
            Some(Terminator::Br(t)) => *t == lp.header,
            _ => false,
        };
        if !pred_ok {
            continue;
        }
        for (arr, (k, size)) in &by_arr {
            if *k <= 0 {
                continue;
            }
            if !guarded.insert((*arr, pred, *k)) {
                continue;
            }
            let kc = func.locals.len() as Local;
            func.locals.push(LirType::I64);
            let dummy = func.locals.len() as Local;
            let inner = match func.locals.get(*arr as usize) {
                Some(LirType::Array(b)) => (**b).clone(),
                _ => LirType::Any,
            };
            func.locals.push(inner);
            if let Some(block) = func.blocks.get_mut(pred) {
                block.instrs.push(Instr::Const {span: UNKNOWN_SPAN,  dst: kc, lit: Lit::Int(k - 1) });
                block.instrs.push(Instr::ArrayGet {
                    span: UNKNOWN_SPAN,
                    dst: dummy,
                    arr: *arr,
                    index: kc,
                    elem_size: *size,
                    unchecked: false,
                });
                dummies.insert((pred, block.instrs.len() - 1));
            }
        }
        for (b, ii, _, _, _) in cands {
            let block = match func.blocks.get_mut(b) {
                Some(x) => x,
                None => continue,
            };
            match block.instrs.get_mut(ii) {
                Some(Instr::ArrayGet { unchecked, .. }) => *unchecked = true,
                Some(Instr::ArraySet { unchecked, .. }) => *unchecked = true,
                _ => {}
            }
            changed = true;
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header_check_fixture() -> Function {
        Function {
            name: "Main".to_string(),
            params: Vec::new(),
            sig_params: Vec::new(),
            ret: LirType::I64,
            throws: false,
            is_unsafe: false,
            method_self: false,
            is_pub: false,
            is_closure: false,
            locals: vec![LirType::I64; 6],
            blocks: vec![
                Block {
                    instrs: vec![Instr::Const {span: UNKNOWN_SPAN,  dst: 1, lit: Lit::Int(0) }],
                    term: Terminator::Br(1),
                },
                Block {
                    instrs: vec![
                        Instr::Const {span: UNKNOWN_SPAN,  dst: 2, lit: Lit::Int(4) },
                        Instr::Const {span: UNKNOWN_SPAN,  dst: 5, lit: Lit::Int(1) },
                        Instr::Cmp {span: UNKNOWN_SPAN,  
                            op: CmpOp::Lt,
                            kind: NumKind::Int,
                            dst: 3,
                            lhs: 1,
                            rhs: 2},
                    ],
                    term: Terminator::BrIf {span: UNKNOWN_SPAN,  cond: 3, then_bb: 2, else_bb: 3 },
                },
                Block {
                    instrs: vec![
                        Instr::Arith {span: UNKNOWN_SPAN,  
                            op: ArithOp::Add,
                            kind: NumKind::Int,
                            dst: 4,
                            lhs: 1,
                            rhs: 5},
                        Instr::Copy {span: UNKNOWN_SPAN,  dst: 1, src: 4 },
                    ],
                    term: Terminator::Br(1),
                },
                Block { instrs: Vec::new(), term: Terminator::Ret(vec![1]) },
            ],
        }
    }

    #[test]
    fn detects_header_check() {
        let f = header_check_fixture();
        let (iv, bound, then_bb) = header_check(&f, 1).unwrap();
        assert_eq!((iv, bound, then_bb), (1, 2, 2));
        let inc = increment_at(&f, &BTreeSet::from([1, 2]), 1).unwrap();
        assert!(increment_ok(&f, inc.0, inc.1, 1));
    }

    #[test]
    fn rejects_missing_check() {
        let mut f = header_check_fixture();
        f.blocks[1].term = Terminator::Br(2);
        assert!(header_check(&f, 1).is_none());
    }

    fn unchecked_flags(src: &str, entry: &str) -> Vec<bool> {
        let mut m = frontend::parser::Parser::parse_module(src).expect("parse");
        assert!(frontend::desugar::desugar(&mut m).is_empty());
        let mut module = crate::lower::lower(&m).expect("lower");
        let f = module
            .functions
            .iter_mut()
            .find(|f| f.name == entry)
            .expect("entry");
        let dom = crate::licm::compute_dominators(f);
        let loops = crate::licm::find_natural_loops(f, &dom);
        eliminate_bounds_checks(f, &dom, &loops);
        let mut out = Vec::new();
        for b in &f.blocks {
            for ins in &b.instrs {
                match ins {
                    Instr::ArrayGet { unchecked, .. } | Instr::ArraySet { unchecked, .. } => {
                        out.push(*unchecked)
                    }
                    _ => {}
                }
            }
        }
        out
    }

    #[test]
    fn const_bound_plain_index_elided() {
        let flags = unchecked_flags(
            "fn Main(): Int {\n    let a = [1, 2, 3, 4, 5];\n    let s = 0;\n    let i = 0;\n    while i < 5 {\n        s = s + a[i];\n        i = i + 1;\n    }\n    return s;\n}\n",
            "Main",
        );
        let mut sorted = flags.clone();
        sorted.sort();
        assert_eq!(sorted, vec![false, true]);
        assert_eq!(flags.iter().filter(|u| **u).count(), 1);
    }

    #[test]
    fn nested_offset_index_elided() {
        let flags = unchecked_flags(
            "fn Main(): Int {\n    let a = [1, 2, 3, 4, 5];\n    let s = 0;\n    let i = 0;\n    while i < 5 {\n        let j = i + 1;\n        while j < 5 {\n            s = s + a[j];\n            j = j + 1;\n        }\n        i = i + 1;\n    }\n    return s;\n}\n",
            "Main",
        );
        let mut sorted = flags.clone();
        sorted.sort();
        assert_eq!(sorted, vec![false, true]);
        assert_eq!(flags.iter().filter(|u| **u).count(), 1);
    }

    #[test]
    fn affine_index_elided() {
        let flags = unchecked_flags(
            "fn Main(): Int {\n    let n = 8;\n    let a: Array<Int> = [];\n    let r = 0;\n    while r < n {\n        let j = 0;\n        while j < n {\n            a[r * n + j] = r;\n            j = j + 1;\n        }\n        r = r + 1;\n    }\n    return 0;\n}\n",
            "Main",
        );
        let mut sorted = flags.clone();
        sorted.sort();
        assert_eq!(sorted, vec![false, true]);
        assert_eq!(flags.iter().filter(|u| **u).count(), 1);
    }

    #[test]
    fn push_in_loop_keeps_checks() {
        let flags = unchecked_flags(
            "fn Main(): Int {\n    let a = [1, 2, 3];\n    let s = 0;\n    let i = 0;\n    while i < 5 {\n        s = s + a[i];\n        a.push(9);\n        i = i + 1;\n    }\n    return s;\n}\n",
            "Main",
        );
        assert_eq!(flags, vec![false]);
    }

    #[test]
    fn conditional_access_keeps_checks() {
        let flags = unchecked_flags(
            "fn Main(): Int {\n    let a = [1, 2, 3];\n    let s = 0;\n    let i = 0;\n    while i < 5 {\n        if i > 10 {\n            s = s + a[i];\n        }\n        i = i + 1;\n    }\n    return s;\n}\n",
            "Main",
        );
        assert_eq!(flags, vec![false]);
    }

    #[test]
    fn len_bound_still_elided() {
        let flags = unchecked_flags(
            "fn Main(): Int {\n    let a = [1, 2, 3, 4, 5];\n    let s = 0;\n    let i = 0;\n    let n = a.length;\n    while i < n {\n        s = s + a[i];\n        i = i + 1;\n    }\n    return s;\n}\n",
            "Main",
        );
        assert_eq!(flags, vec![true]);
    }

    #[test]
    fn param_bound_keeps_checks() {
        let flags = unchecked_flags(
            "fn f(a: Array<Int>, n: Int): Int {\n    let s = 0;\n    let i = 0;\n    while i < n {\n        s = s + a[i];\n        i = i + 1;\n    }\n    return s;\n}\nfn Main(): Int {\n    return 0;\n}\n",
            "f",
        );
        assert!(!flags.is_empty());
        assert!(flags.iter().all(|u| !u));
    }
}
