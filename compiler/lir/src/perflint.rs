use crate::accum::{self, LoopFacts};
use crate::instr::*;
use crate::licm::DominatorTree;
use crate::opt::{instr_dst, instr_reads, term_reads};
use diagnostics::{Code, Diagnostic, Span};
use std::collections::BTreeSet;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ReceiptKind {
    AccumV1,
    AccumPeel,
    BceVersioned,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransformReceipt {
    pub func: String,
    pub kind: ReceiptKind,
    pub span: Span,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum PerfCode {
    P101,
    P102,
    P201,
    P202,
}

pub const P202_SCALE_PRODUCT: i64 = 1024;
pub const P202_LARGE_TRIP: i64 = 256;
pub const P202_MIN_TRIP: i64 = 4;
pub const P202_INDEXOF_MIN_OUTER: i64 = 64;
const P202_MAX_BODY_INSTRS: usize = 128;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MissedReason {
    Alias,
    ZeroInit,
    CostGate,
    SubscriptRole,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MissedReduction {
    pub func: String,
    pub reason: MissedReason,
    pub span: Span,
    pub inner_trip: i64,
    pub alloc: Option<Span>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PerfFail {
    Alias,
    ZeroInit,
    CostGate,
    SubscriptRole,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PerfLint {
    pub func: String,
    pub code: PerfCode,
    pub span: Span,
    pub message: String,
    pub hint: String,
}

pub fn lint_module(module: &Module) -> Vec<PerfLint> {
    let mut out: Vec<PerfLint> = Vec::new();
    for func in &module.functions {
        for receipt in module
            .transform_receipts
            .iter()
            .filter(|r| r.func == func.name)
        {
            match receipt.kind {
                ReceiptKind::AccumV1 => out.push(note(
                    &func.name,
                    PerfCode::P101,
                    receipt.span,
                    "this ijk-style reduction nest was restructured to ikj-style accumulation (zero-init) by the compiler; inner loop is now unit-stride and vectorizer-friendly. No action needed.",
                    "variant: V1 zero-init",
                )),
                ReceiptKind::AccumPeel => out.push(note(
                    &func.name,
                    PerfCode::P101,
                    receipt.span,
                    "this ijk-style reduction nest was restructured to ikj-style accumulation (peeled) by the compiler; inner loop is now unit-stride and vectorizer-friendly. No action needed.",
                    "variant: peel",
                )),
                ReceiptKind::BceVersioned => out.push(note(
                    &func.name,
                    PerfCode::P102,
                    receipt.span,
                    "bounds checks in this loop were versioned (fast path runs unchecked after a hoisted guard). No action needed.",
                    "versioned loop keeps a hoisted guard; fast path is unchecked",
                )),
            }
        }
        for missed in module
            .missed_reductions
            .iter()
            .filter(|r| r.func == func.name)
        {
            if missed.inner_trip < accum::MIN_ACCUM_TRIP {
                continue;
            }
            let fail = match missed.reason {
                MissedReason::Alias => PerfFail::Alias,
                MissedReason::ZeroInit => PerfFail::ZeroInit,
                MissedReason::CostGate => PerfFail::CostGate,
                MissedReason::SubscriptRole => PerfFail::SubscriptRole,
            };
            let extra = match missed.alloc {
                Some(s) => format!("{}..{}", s.start, s.end),
                None => String::new(),
            };
            let (message, hint) = fail_text(fail, &extra);
            out.push(PerfLint {
                func: func.name.clone(),
                code: PerfCode::P201,
                span: missed.span,
                message,
                hint,
            });
        }
        out.extend(lint_search_and_accum(func));
    }
    out.sort_by(|a, b| {
        a.func
            .cmp(&b.func)
            .then(a.span.start.cmp(&b.span.start))
            .then(a.span.end.cmp(&b.span.end))
            .then(a.code.cmp(&b.code))
    });
    out
}

fn note(func: &str, code: PerfCode, span: Span, message: &str, hint: &str) -> PerfLint {
    PerfLint {
        func: func.to_string(),
        code,
        span,
        message: message.to_string(),
        hint: hint.to_string(),
    }
}

fn fail_text(fail: PerfFail, extra: &str) -> (String, String) {
    match fail {
        PerfFail::Alias => (
            "reduction nest kept scalar (ijx order) because the input and output arrays may alias; the compiler cannot hoist the load. If aliasing is intended, ignore this. Otherwise use distinct arrays (or copy) so each input is read-only in the nest.".to_string(),
            "give each input array a distinct allocation, or copy the input before the nest".to_string(),
        ),
        PerfFail::ZeroInit => {
            let mut message = "reduction nest kept scalar order because zero-initialization of the output could not be proven".to_string();
            if extra.is_empty() {
                message.push_str("; the allocation site is not provably zero-filled");
            } else {
                message.push_str(&format!(" (allocation site at {extra} not provably zero-filled)"));
            }
            message.push_str(". Zero-fill the output before the nest (or fill it in a counted loop) to enable accumulation form.");
            (
                message,
                "zero-fill the output array in a counted loop immediately before the nest".to_string(),
            )
        }
        PerfFail::CostGate => (
            format!(
                "reduction nest kept scalar order because the inner loop is not provably unit-stride after interchange / trip count below {}. If the trip count is a larger constant, hoist it to a named constant.",
                accum::MIN_ACCUM_TRIP
            ),
            "make the middle-loop trip a constant of at least 16 with unit-stride access after interchange".to_string(),
        ),
        PerfFail::SubscriptRole => (
            "nest looks like a reduction but an operand index varies along an unexpected loop; interchange is not legal. No action available; reported so the missed optimization is visible.".to_string(),
            "no source change enables this transform; the nest shape itself blocks interchange".to_string(),
        ),
    }
}

fn is_indexof_builtin(name: &str) -> bool {
    name == "__rnx_string_index_of" || name == "__rnx_string_index_of_from"
}

fn counted_trip(cl: &crate::bce::CountedLoop) -> Option<i64> {
    match &cl.init {
        crate::bce::InitExpr::Const(c) => {
            let trip = cl.bound.checked_sub(*c)?;
            if trip >= 1 {
                Some(trip)
            } else {
                None
            }
        }
        _ => None,
    }
}

fn block_instr_count(func: &Function, blocks: &BTreeSet<BlockId>) -> usize {
    let mut n = 0usize;
    for b in blocks {
        n += func.blocks.get(*b).map(|x| x.instrs.len()).unwrap_or(0);
    }
    n
}

fn has_conditional_exit(func: &Function, inner: &crate::bce::CountedLoop) -> bool {
    for b in &inner.blocks {
        if *b == inner.header {
            continue;
        }
        let block = match func.blocks.get(*b) {
            Some(x) => x,
            None => return false,
        };
        if let Terminator::BrIf { then_bb, else_bb, .. } = &block.term
            && (!inner.blocks.contains(then_bb) || !inner.blocks.contains(else_bb))
        {
            return true;
        }
    }
    false
}

fn scan_side(
    func: &Function,
    maps: &FuncMaps,
    dom: &DominatorTree,
    inner: &crate::bce::CountedLoop,
    operand: Local,
    cmp_block: BlockId,
    cmp_idx: usize,
) -> Option<Local> {
    let (db, di) = single_def_uniq(maps, operand)?;
    if !inner.blocks.contains(&db) {
        return None;
    }
    let dominates = (db == cmp_block && di < cmp_idx)
        || (db != cmp_block && dom.dominates(db, cmp_block));
    if !dominates {
        return None;
    }
    match func.blocks.get(db)?.instrs.get(di)? {
        Instr::ArrayGet { arr, index, .. } => {
            let nest = [inner];
            let form = crate::bce::idx_affine(func, &nest, dom, db, di, *index, 0)?;
            if form.coeffs.is_empty() {
                return None;
            }
            if form.coeffs.keys().all(|k| *k == inner.iv) {
                Some(*arr)
            } else {
                None
            }
        }
        _ => None,
    }
}

fn invariant_side(
    func: &Function,
    maps: &FuncMaps,
    dom: &DominatorTree,
    inner: &crate::bce::CountedLoop,
    operand: Local,
) -> bool {
    let (db, di) = match single_def_uniq(maps, operand) {
        Some(x) => x,
        None => return false,
    };
    match func.blocks.get(db).and_then(|b| b.instrs.get(di)) {
        Some(Instr::Const { lit: Lit::Int(_), .. }) => true,
        Some(_) => {
            if inner.blocks.contains(&db) {
                return false;
            }
            dom.dominates(db, inner.header)
        }
        None => false,
    }
}

struct ScanCompare {
    span: Span,
    site: (BlockId, usize),
    lhs: Local,
    rhs: Local,
    is_str: bool,
}

fn inner_scan_compare(
    func: &Function,
    inner: &crate::bce::CountedLoop,
) -> Option<ScanCompare> {
    let mut found: Option<ScanCompare> = None;
    let mut ordered: Vec<BlockId> = inner.blocks.iter().copied().collect();
    ordered.sort();
    for b in ordered {
        let count = func.blocks.get(b).map(|x| x.instrs.len()).unwrap_or(0);
        for ii in 0..count {
            let ins = func.blocks.get(b)?.instrs.get(ii)?;
            let cand = match ins {
                Instr::Cmp { span, op: CmpOp::Eq, kind: NumKind::Int, lhs, rhs, .. } => {
                    Some(ScanCompare { span: *span, site: (b, ii), lhs: *lhs, rhs: *rhs, is_str: false })
                }
                Instr::Cmp { op: CmpOp::Eq, .. } => return None,
                Instr::Call { span, dsts, target: CallTarget::Builtin(name), args, .. }
                    if name == "streq" && args.len() == 2 && dsts.len() == 1 =>
                {
                    Some(ScanCompare {
                        span: *span,
                        site: (b, ii),
                        lhs: args[0],
                        rhs: args[1],
                        is_str: true,
                    })
                }
                _ => None,
            };
            if cand.is_some() {
                if found.is_some() {
                    return None;
                }
                found = cand;
            }
        }
    }
    found
}

fn inner_scan_veto(func: &Function, inner: &crate::bce::CountedLoop) -> bool {
    let mut ordered: Vec<BlockId> = inner.blocks.iter().copied().collect();
    ordered.sort();
    for b in ordered {
        let count = func.blocks.get(b).map(|x| x.instrs.len()).unwrap_or(0);
        for ii in 0..count {
            match func.blocks.get(b).and_then(|x| x.instrs.get(ii)) {
                Some(Instr::Call { target: CallTarget::Builtin(name), .. })
                    if name == "streq" || is_indexof_builtin(name) => {}
                Some(Instr::Call { .. }) => return true,
                _ => {}
            }
        }
    }
    false
}

fn match_inner_scan(
    func: &Function,
    maps: &FuncMaps,
    dom: &DominatorTree,
    inner: &crate::bce::CountedLoop,
    outer_trip: i64,
    inner_trip: i64,
) -> Option<PerfLint> {
    let product = (outer_trip as i128).checked_mul(inner_trip as i128)?;
    let big_enough = product >= P202_SCALE_PRODUCT as i128
        || (outer_trip >= P202_LARGE_TRIP && inner_trip >= P202_MIN_TRIP)
        || (inner_trip >= P202_LARGE_TRIP && outer_trip >= P202_MIN_TRIP);
    if !big_enough {
        return None;
    }
    if block_instr_count(func, &inner.blocks) > P202_MAX_BODY_INSTRS {
        return None;
    }
    if inner_scan_veto(func, inner) {
        return None;
    }
    let cmp = inner_scan_compare(func, inner)?;
    if cmp.span == UNKNOWN_SPAN {
        return None;
    }
    let (cb, ci) = cmp.site;
    let left_scan = scan_side(func, maps, dom, inner, cmp.lhs, cb, ci);
    let right_scan = scan_side(func, maps, dom, inner, cmp.rhs, cb, ci);
    let left_inv = invariant_side(func, maps, dom, inner, cmp.lhs);
    let right_inv = invariant_side(func, maps, dom, inner, cmp.rhs);
    let arr = match (left_scan, right_scan, left_inv, right_inv) {
        (Some(a), None, _, true) => a,
        (None, Some(a), true, _) => a,
        _ => return None,
    };
    if !has_conditional_exit(func, inner) {
        return None;
    }
    let what = if cmp.is_str {
        "string"
    } else {
        match func.locals.get(arr as usize) {
            Some(LirType::Array(inner_ty))
                if matches!(inner_ty.as_ref(), LirType::Str) =>
            {
                "string"
            }
            _ => "array",
        }
    };
    Some(PerfLint {
        func: String::new(),
        code: PerfCode::P202,
        span: cmp.span,
        message: format!(
            "linear scan over {what} of length {inner_trip} inside loop of {outer_trip} iterations (~{} comparisons); this is quadratic.",
            product
        ),
        hint: "Hoist the scan out of the loop, or build a lookup table (object/dictionary keyed by the compared field) once before the loop and probe it inside.".to_string(),
    })
}

fn iv_forward_reach(func: &Function, outer: &crate::bce::CountedLoop) -> BTreeSet<Local> {
    let mut set: BTreeSet<Local> = BTreeSet::from([outer.iv]);
    let mut ordered: Vec<BlockId> = outer.blocks.iter().copied().collect();
    ordered.sort();
    let mut tmp: Vec<Local> = Vec::new();
    let mut dsts: Vec<Local> = Vec::new();
    loop {
        let before = set.len();
        for b in &ordered {
            let count = func.blocks.get(*b).map(|x| x.instrs.len()).unwrap_or(0);
            for ii in 0..count {
                let ins = match func.blocks.get(*b).and_then(|x| x.instrs.get(ii)) {
                    Some(x) => x,
                    None => continue,
                };
                tmp.clear();
                instr_reads(ins, &mut tmp);
                if tmp.iter().any(|l| set.contains(l)) {
                    dsts.clear();
                    match ins {
                        Instr::Call { dsts: d, err, .. } => {
                            dsts.extend(d.iter().copied());
                            dsts.extend(err.iter().copied());
                        }
                        _ => {
                            if let Some(d) = instr_dst(ins) {
                                dsts.push(d);
                            }
                        }
                    }
                    for d in &dsts {
                        set.insert(*d);
                    }
                }
            }
        }
        if set.len() == before {
            break;
        }
    }
    set
}

fn match_indexof(
    func: &Function,
    outer: &crate::bce::CountedLoop,
    outer_trip: i64,
    skip_blocks: &BTreeSet<BlockId>,
) -> Vec<PerfLint> {
    let mut out = Vec::new();
    if outer_trip < P202_INDEXOF_MIN_OUTER {
        return out;
    }
    let varying = iv_forward_reach(func, outer);
    let mut ordered: Vec<BlockId> = outer.blocks.iter().copied().collect();
    ordered.sort();
    for b in ordered {
        if skip_blocks.contains(&b) {
            continue;
        }
        let count = func.blocks.get(b).map(|x| x.instrs.len()).unwrap_or(0);
        for ii in 0..count {
            match func.blocks.get(b).and_then(|x| x.instrs.get(ii)) {
                Some(Instr::Call { span, target: CallTarget::Builtin(name), args, .. })
                    if is_indexof_builtin(name) =>
                {
                    if *span == UNKNOWN_SPAN {
                        continue;
                    }
                    if args.iter().any(|a| varying.contains(a)) {
                        out.push(PerfLint {
                            func: String::new(),
                            code: PerfCode::P202,
                            span: *span,
                            message: format!(
                                "string search (indexOf) inside loop of {outer_trip} iterations; each call scans the haystack, so total work grows with the trip count times the haystack length."
                            ),
                            hint: "replace per-iteration indexOf with a precomputed position table or a single pass.".to_string(),
                        });
                    }
                }
                _ => {}
            }
        }
    }
    out
}

fn lint_search_and_accum(func: &Function) -> Vec<PerfLint> {
    let mut out = Vec::new();
    if !has_cfg_cycle(func) {
        return out;
    }
    let dom = crate::licm::compute_dominators(func);
    let loops = crate::licm::find_natural_loops(func, &dom);
    let mut counted: Vec<crate::bce::CountedLoop> = loops
        .iter()
        .filter_map(|lp| crate::bce::analyze_counted(func, lp))
        .collect();
    if counted.is_empty() {
        return out;
    }
    let maps = func_maps(func);
    if maps.truncated {
        return out;
    }
    counted.sort_by_key(|l| (l.header, l.iv));
    let trips: Vec<Option<i64>> = counted.iter().map(counted_trip).collect();
    let mut scan_inner_blocks: BTreeSet<BlockId> = BTreeSet::new();
    for (oi, outer) in counted.iter().enumerate() {
        let outer_trip = match trips[oi] {
            Some(t) => t,
            None => continue,
        };
        let mut inners: Vec<usize> = Vec::new();
        for (ii, inner) in counted.iter().enumerate() {
            if ii != oi
                && inner.blocks.is_subset(&outer.blocks)
                && inner.blocks != outer.blocks
            {
                inners.push(ii);
            }
        }
        inners.sort_by_key(|ii| counted[*ii].blocks.len());
        for ii in inners {
            if counted[ii].blocks.iter().any(|b| scan_inner_blocks.contains(b)) {
                continue;
            }
            let inner_trip = match trips[ii] {
                Some(t) => t,
                None => continue,
            };
            if let Some(mut lint) =
                match_inner_scan(func, &maps, &dom, &counted[ii], outer_trip, inner_trip)
            {
                lint.func = func.name.clone();
                scan_inner_blocks.extend(counted[ii].blocks.iter().copied());
                out.push(lint);
                break;
            }
        }
    }
    for (oi, outer) in counted.iter().enumerate() {
        let outer_trip = match trips[oi] {
            Some(t) => t,
            None => continue,
        };
        let mut lints = match_indexof(func, outer, outer_trip, &scan_inner_blocks);
        for lint in lints.drain(..) {
            let mut lint = lint;
            lint.func = func.name.clone();
            out.push(lint);
        }
    }
    out
}

fn has_cfg_cycle(func: &Function) -> bool {
    let n = func.blocks.len();
    let mut color: Vec<u8> = vec![0; n];
    for start in 0..n {
        if color[start] != 0 {
            continue;
        }
        let mut stack: Vec<(usize, bool)> = vec![(start, false)];
        while let Some((b, done)) = stack.pop() {
            if b >= n {
                continue;
            }
            if done {
                color[b] = 2;
                continue;
            }
            if color[b] != 0 {
                continue;
            }
            color[b] = 1;
            stack.push((b, true));
            let targets: Vec<usize> = match &func.blocks[b].term {
                Terminator::Ret(_) | Terminator::Unreachable { .. } => Vec::new(),
                Terminator::Br(t) => vec![*t],
                Terminator::BrIf { then_bb, else_bb, .. } => vec![*then_bb, *else_bb],
                Terminator::BrErr { catch_bb, next_bb, .. } => vec![*catch_bb, *next_bb],
                Terminator::Switch { cases, default, .. } => {
                    let mut out: Vec<usize> = cases.iter().map(|(_, x)| *x).collect();
                    out.push(*default);
                    out
                }
                Terminator::Throw { catch, .. } => {
                    catch.map(|(x, _, _)| vec![x]).unwrap_or_default()
                }
                Terminator::Rethrow { catch_bb, .. } => vec![*catch_bb],
            };
            for x in targets {
                if x < n {
                    if color[x] == 1 {
                        return true;
                    }
                    if color[x] == 0 {
                        stack.push((x, false));
                    }
                }
            }
        }
    }
    false
}

struct FuncMaps {
    uses: Vec<Vec<(BlockId, usize)>>,
    defs: Vec<Vec<(BlockId, usize)>>,
    truncated: bool,
}

const MAX_INDEXED_LOCAL: u32 = 1_000_000;

fn maps_push(
    vecs: &mut Vec<Vec<(BlockId, usize)>>,
    truncated: &mut bool,
    local: Local,
    site: (BlockId, usize),
) {
    if local >= MAX_INDEXED_LOCAL {
        *truncated = true;
        return;
    }
    let idx = local as usize;
    if vecs.len() <= idx {
        vecs.resize(idx + 1, Vec::new());
    }
    vecs[idx].push(site);
}

fn func_maps(func: &Function) -> FuncMaps {
    let mut maps = FuncMaps {
        uses: Vec::new(),
        defs: Vec::new(),
        truncated: false,
    };
    let mut tmp: Vec<Local> = Vec::new();
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, ins) in block.instrs.iter().enumerate() {
            if let Some(dst) = instr_dst(ins) {
                maps_push(&mut maps.defs, &mut maps.truncated, dst, (bi, ii));
            }
            if let Instr::Call { dsts, err, .. } = ins {
                for d in dsts.iter().chain(err.iter()) {
                    maps_push(&mut maps.defs, &mut maps.truncated, *d, (bi, ii));
                }
            }
            if let Instr::Defer { body, .. } = ins {
                for nested in body {
                    if let Some(dst) = instr_dst(nested) {
                        maps_push(&mut maps.defs, &mut maps.truncated, dst, (bi, ii));
                    }
                }
            }
            tmp.clear();
            instr_reads(ins, &mut tmp);
            for l in tmp.iter() {
                maps_push(&mut maps.uses, &mut maps.truncated, *l, (bi, ii));
            }
        }
        tmp.clear();
        term_reads(&block.term, &mut tmp);
        for l in tmp.iter() {
            maps_push(&mut maps.uses, &mut maps.truncated, *l, (bi, usize::MAX));
        }
        if let Terminator::BrErr { catch_bind, .. } = &block.term {
            maps_push(&mut maps.defs, &mut maps.truncated, *catch_bind, (bi, usize::MAX));
        }
    }
    maps
}

fn sites_of(vecs: &[Vec<(BlockId, usize)>], local: Local) -> &[(BlockId, usize)] {
    vecs.get(local as usize).map(|v| v.as_slice()).unwrap_or(&[])
}

fn single_def_in(maps: &FuncMaps, local: Local) -> Option<(BlockId, usize)> {
    match sites_of(&maps.defs, local) {
        [(bi, ii)] if *ii != usize::MAX => Some((*bi, *ii)),
        _ => None,
    }
}

fn single_def_uniq(maps: &FuncMaps, local: Local) -> Option<(BlockId, usize)> {
    let mut sites = def_sites(maps, local);
    sites.sort();
    sites.dedup();
    match sites.as_slice() {
        [(bi, ii)] if *ii != usize::MAX => Some((*bi, *ii)),
        _ => None,
    }
}

fn use_count_in(maps: &FuncMaps, local: Local) -> usize {
    sites_of(&maps.uses, local).len()
}

fn def_sites(maps: &FuncMaps, local: Local) -> Vec<(BlockId, usize)> {
    sites_of(&maps.defs, local).to_vec()
}

fn single_use_get(
    func: &Function,
    maps: &FuncMaps,
    local: Local,
    z_body: &BTreeSet<BlockId>,
) -> Option<(BlockId, usize, Local, Local)> {
    if use_count_in(maps, local) != 1 {
        return None;
    }
    let (db, di) = single_def_in(maps, local)?;
    if !z_body.contains(&db) {
        return None;
    }
    match func.blocks.get(db)?.instrs.get(di)? {
        Instr::ArrayGet {
            dst, arr, index, ..
        } if *dst == local => Some((db, di, *arr, *index)),
        _ => None,
    }
}

fn is_array_i64(func: &Function, local: Local) -> bool {
    matches!(func.locals.get(local as usize), Some(LirType::Array(inner)) if matches!(inner.as_ref(), LirType::I64))
}

struct RoleSkeleton {
    store_span: Span,
}

fn role_skeleton(
    func: &Function,
    dom: &DominatorTree,
    nest: &[LoopFacts; 3],
    maps: &FuncMaps,
) -> Option<RoleSkeleton> {
    let (x, y, z) = (&nest[0], &nest[1], &nest[2]);
    let nest_refs = [&x.counted, &y.counted, &z.counted];
    let union_blocks: BTreeSet<BlockId> =
        nest.iter().flat_map(|l| l.blocks.iter().copied()).collect();
    let z_body: BTreeSet<BlockId> = z
        .blocks
        .iter()
        .copied()
        .filter(|b| *b != z.header)
        .collect();
    let y_outer: BTreeSet<BlockId> = y
        .blocks
        .iter()
        .copied()
        .filter(|b| !z.blocks.contains(b))
        .collect();
    let mut ordered_body: Vec<BlockId> = z_body.iter().copied().collect();
    ordered_body.sort();
    for ab in ordered_body {
        let count = func.blocks.get(ab).map(|b| b.instrs.len()).unwrap_or(0);
        for ai in 0..count {
            let (t, lhs, rhs) = match func.blocks.get(ab).and_then(|b| b.instrs.get(ai)) {
                Some(Instr::Arith {
                    op: ArithOp::Add,
                    kind: NumKind::Int,
                    dst,
                    lhs,
                    rhs,
                    ..
                }) => (*dst, *lhs, *rhs),
                _ => continue,
            };
            for (s_cand, m) in [(lhs, rhs), (rhs, lhs)] {
                if let Some(skel) = check_roles(
                    func,
                    dom,
                    &nest_refs,
                    x,
                    y,
                    z,
                    &union_blocks,
                    &z_body,
                    &y_outer,
                    maps,
                    ab,
                    ai,
                    t,
                    s_cand,
                    m,
                ) {
                    return Some(skel);
                }
            }
        }
    }
    None
}

#[allow(clippy::too_many_arguments)]
fn check_roles(
    func: &Function,
    dom: &DominatorTree,
    nest: &[&crate::bce::CountedLoop],
    x: &LoopFacts,
    y: &LoopFacts,
    z: &LoopFacts,
    union_blocks: &BTreeSet<BlockId>,
    z_body: &BTreeSet<BlockId>,
    y_outer: &BTreeSet<BlockId>,
    maps: &FuncMaps,
    ab: BlockId,
    ai: usize,
    t: Local,
    s_cand: Local,
    m: Local,
) -> Option<RoleSkeleton> {
    if !matches!(func.locals.get(s_cand as usize), Some(LirType::I64)) {
        return None;
    }
    let (mb, mi) = single_def_in(maps, m)?;
    if !z_body.contains(&mb) {
        return None;
    }
    match func.blocks.get(mb)?.instrs.get(mi)? {
        Instr::Arith {
            op: ArithOp::Mul,
            kind: NumKind::Int,
            dst,
            ..
        } if *dst == m => {}
        _ => return None,
    }
    if ab != mb {
        return None;
    }
    if use_count_in(maps, m) != 1 {
        return None;
    }
    if t != s_cand && use_count_in(maps, t) != 1 {
        return None;
    }
    let (p, q) = match func.blocks.get(mb)?.instrs.get(mi)? {
        Instr::Arith {
            op: ArithOp::Mul,
            kind: NumKind::Int,
            lhs,
            rhs,
            ..
        } => (*lhs, *rhs),
        _ => return None,
    };
    let (la_block, la_idx, arr_a, idx_a) = single_use_get(func, maps, p, z_body)?;
    let (lb_block, lb_idx, arr_b, idx_b) = single_use_get(func, maps, q, z_body)?;
    if la_block != ab || lb_block != ab {
        return None;
    }
    if !is_array_i64(func, arr_a) || !is_array_i64(func, arr_b) {
        return None;
    }
    let s_defs: Vec<(BlockId, usize)> = def_sites(maps, s_cand);
    if s_defs.len() != 2 || s_defs.iter().any(|(_, ii)| *ii == usize::MAX) {
        return None;
    }
    let mut update_def: Option<(BlockId, usize)> = None;
    if t == s_cand {
        update_def = Some((ab, ai));
    } else {
        for (db, di) in &s_defs {
            match func.blocks.get(*db)?.instrs.get(*di)? {
                Instr::Copy { dst, src, .. } if *dst == s_cand && *src == t => {
                    if update_def.is_some() {
                        return None;
                    }
                    update_def = Some((*db, *di));
                }
                _ => {}
            }
        }
    }
    let update_def = update_def?;
    let mut init_def: Option<(BlockId, usize)> = None;
    for (db, di) in &s_defs {
        if (*db, *di) == update_def {
            continue;
        }
        if init_def.is_some() {
            return None;
        }
        match func.blocks.get(*db)?.instrs.get(*di)? {
            Instr::Const {
                lit: Lit::Int(0),
                dst,
                ..
            } if *dst == s_cand => {
                init_def = Some((*db, *di));
            }
            Instr::Copy { dst, src, .. } if *dst == s_cand => {
                let (tb, ti) = single_def_in(maps, *src)?;
                match func.blocks.get(tb)?.instrs.get(ti)? {
                    Instr::Const {
                        lit: Lit::Int(0), ..
                    } => {
                        if use_count_in(maps, *src) != 1 {
                            return None;
                        }
                        if tb != *db {
                            return None;
                        }
                        init_def = Some((*db, *di));
                    }
                    _ => return None,
                }
            }
            _ => return None,
        }
    }
    let init_def = init_def?;
    if !y_outer.contains(&init_def.0) {
        return None;
    }
    if !dom.dominates(init_def.0, z.header) {
        return None;
    }
    let mut store_site: Option<(BlockId, usize, Span, Local, Local)> = None;
    let mut ordered_outer: Vec<BlockId> = y_outer.iter().copied().collect();
    ordered_outer.sort();
    for sb in ordered_outer {
        let count = func.blocks.get(sb).map(|b| b.instrs.len()).unwrap_or(0);
        for si in 0..count {
            match func.blocks.get(sb)?.instrs.get(si)? {
                Instr::ArraySet {
                    arr,
                    index,
                    value,
                    span,
                    ..
                } if *value == s_cand => {
                    if store_site.is_some() {
                        return None;
                    }
                    store_site = Some((sb, si, *span, *arr, *index));
                }
                _ => {}
            }
        }
    }
    let (store_block, store_idx, store_span, arr_c, idx_c) = store_site?;
    if !is_array_i64(func, arr_c) {
        return None;
    }
    if !dom.dominates(z.exit_bb, store_block) {
        return None;
    }
    if !dom.dominates(store_block, y.latch) {
        return None;
    }
    let s_use_sites: BTreeSet<(BlockId, usize)> =
        sites_of(&maps.uses, s_cand).iter().copied().collect();
    if s_use_sites.iter().any(|(_, ui)| *ui == usize::MAX) {
        return None;
    }
    let expected: BTreeSet<(BlockId, usize)> = BTreeSet::from([(ab, ai), (store_block, store_idx)]);
    if s_use_sites != expected {
        return None;
    }
    let mut sites: Vec<(BlockId, usize)> = Vec::new();
    let mut ordered: Vec<BlockId> = union_blocks.iter().copied().collect();
    ordered.sort();
    for b in ordered {
        if let Some(block) = func.blocks.get(b) {
            for (ii, ins) in block.instrs.iter().enumerate() {
                if ins.span() == UNKNOWN_SPAN {
                    continue;
                }
                if matches!(ins, Instr::ArrayGet { .. } | Instr::ArraySet { .. }) {
                    sites.push((b, ii));
                }
            }
        }
    }
    sites.sort();
    let mut triple_sites = vec![
        (la_block, la_idx),
        (lb_block, lb_idx),
        (store_block, store_idx),
    ];
    triple_sites.sort();
    if sites != triple_sites {
        return None;
    }
    let form_a = crate::bce::idx_affine(func, nest, dom, la_block, la_idx, idx_a, 0)?;
    let form_b = crate::bce::idx_affine(func, nest, dom, lb_block, lb_idx, idx_b, 0)?;
    let form_c = crate::bce::idx_affine(func, nest, dom, store_block, store_idx, idx_c, 0)?;
    let allowed_a = [x.iv, z.iv];
    let allowed_b = [z.iv, y.iv];
    let allowed_c = [x.iv, y.iv];
    let bad = !form_a.coeffs.keys().all(|k| allowed_a.contains(k))
        || !form_b.coeffs.keys().all(|k| allowed_b.contains(k))
        || !form_c.coeffs.keys().all(|k| allowed_c.contains(k));
    if !bad {
        return None;
    }
    Some(RoleSkeleton { store_span })
}

pub(crate) fn subscript_missed(
    func: &Function,
    dom: &DominatorTree,
    located: &[[LoopFacts; 3]],
) -> Option<(MissedReason, Span, i64, Option<Span>)> {
    if !plausible_shape(func) {
        return None;
    }
    if located.is_empty() {
        return None;
    }
    let maps = func_maps(func);
    if maps.truncated {
        return None;
    }
    for nest in located {
        let trip = nest[2].bound_const - nest[2].init_const;
        if let Some(skel) = role_skeleton(func, dom, nest, &maps) {
            return Some((
                MissedReason::SubscriptRole,
                skel.store_span,
                trip,
                None,
            ));
        }
    }
    None
}

fn plausible_shape(func: &Function) -> bool {
    let mut has_mul = false;
    let mut has_set = false;
    for block in &func.blocks {
        for ins in &block.instrs {
            match ins {
                Instr::Arith {
                    op: ArithOp::Mul,
                    kind: NumKind::Int,
                    ..
                } => has_mul = true,
                Instr::ArraySet { .. } => has_set = true,
                _ => {}
            }
            if has_mul && has_set {
                return true;
            }
        }
    }
    false
}

pub fn to_diagnostics(lints: &[PerfLint]) -> Vec<Diagnostic> {
    lints
        .iter()
        .map(|l| {
            let code = match l.code {
                PerfCode::P101 => Code::P101,
                PerfCode::P102 => Code::P102,
                PerfCode::P201 => Code::P201,
                PerfCode::P202 => Code::P202,
            };
            Diagnostic::new(code, l.message.clone())
                .with_span(l.span)
                .with_hint(l.hint.clone())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn lower_module(src: &str) -> Module {
        let mut m = frontend::parser::Parser::parse_module(src).expect("parse");
        assert!(frontend::desugar::desugar(&mut m).is_empty());
        crate::lower::lower(&m).expect("lower")
    }

    fn opt_module(src: &str) -> Module {
        let mut module = lower_module(src);
        crate::opt::optimize_lir(&mut module, 1, "Main");
        module
    }

    fn matmul_src() -> String {
        std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../benches/compute/matmul/main.rnx"
        ))
        .expect("matmul bench")
    }

    fn variant(base: &str, from: &str, to: &str) -> String {
        let out = base.replacen(from, to, 1);
        assert_ne!(out, base, "variant anchor missing: {from}");
        assert!(
            base.matches(from).count() == 1,
            "variant anchor not unique: {from}"
        );
        out
    }

    fn lints_of(module: &Module) -> &[PerfLint] {
        &module.perf_lints
    }

    fn codes(module: &Module) -> Vec<PerfCode> {
        lints_of(module).iter().map(|l| l.code).collect()
    }

    fn receipt_kinds(module: &Module) -> Vec<ReceiptKind> {
        module.transform_receipts.iter().map(|r| r.kind).collect()
    }

    fn has_accum_receipt(module: &Module) -> bool {
        module
            .transform_receipts
            .iter()
            .any(|r| matches!(r.kind, ReceiptKind::AccumV1 | ReceiptKind::AccumPeel))
    }

    fn p201_lints(module: &Module) -> Vec<&PerfLint> {
        lints_of(module)
            .iter()
            .filter(|l| l.code == PerfCode::P201)
            .collect()
    }

    #[test]
    fn matmul_256_fires_p101_only() {
        let module = opt_module(&matmul_src());
        assert_eq!(
            receipt_kinds(&module),
            vec![ReceiptKind::AccumV1, ReceiptKind::BceVersioned]
        );
        assert_eq!(codes(&module), vec![PerfCode::P101, PerfCode::P102]);
    }

    #[test]
    fn alias_variant_fires_p201_alias() {
        let src = variant(&matmul_src(), "c[x * n + y] = s;", "a[x * n + y] = s;");
        let module = opt_module(&src);
        assert!(!has_accum_receipt(&module));
        let p201 = p201_lints(&module);
        assert_eq!(p201.len(), 1);
        assert!(p201[0].message.contains("may alias"));
        assert!(p201[0].message.contains("ignore this"));
    }

    #[test]
    fn garbage_variant_fires_p201_zeroinit_with_peel_note() {
        let src = variant(&matmul_src(), "c.push(0);", "c.push(7);");
        let module = opt_module(&src);
        let kinds = receipt_kinds(&module);
        assert!(kinds.contains(&ReceiptKind::AccumPeel));
        assert!(!kinds.contains(&ReceiptKind::AccumV1));
        let mut p101 = 0;
        let mut p102 = 0;
        let mut p201 = 0;
        for l in lints_of(&module) {
            match l.code {
                PerfCode::P101 => p101 += 1,
                PerfCode::P102 => p102 += 1,
                PerfCode::P201 => {
                    p201 += 1;
                    assert!(l.message.contains("zero-initialization"));
                }
                PerfCode::P202 => {
                    panic!("unexpected {:?} on garbage matmul variant: {l:?}", l.code)
                }
            }
        }
        assert_eq!(p101, 1);
        assert_eq!(p201, 1);
        assert!(p102 <= 1, "at most the checksum note");
    }

    #[test]
    fn smalltrip_variant_fires_p201_costgate() {
        let src = variant(&matmul_src(), "while y < n {", "while y < 4 {");
        let module = opt_module(&src);
        assert!(!has_accum_receipt(&module));
        let p201 = p201_lints(&module);
        assert_eq!(p201.len(), 1);
        assert!(p201[0].message.contains("trip count below"));
    }

    #[test]
    fn subscript_variant_fires_p201_subscript_role() {
        let src = variant(&matmul_src(), "a[x * n + z]", "a[y * n + z]");
        let module = opt_module(&src);
        assert!(!has_accum_receipt(&module));
        let p201 = p201_lints(&module);
        assert_eq!(p201.len(), 1);
        assert!(p201[0].message.contains("unexpected loop"));
    }

    #[test]
    fn tiny_nest_fires_no_p201() {
        let src = matmul_src().replace("let n = 256;", "let n = 4;");
        let module = opt_module(&src);
        assert!(!has_accum_receipt(&module));
        assert!(p201_lints(&module).is_empty());
    }

    const STENCIL: &str = "fn Main(): Int {\n    let n = 32;\n    let a: Array<Int> = [];\n    let b: Array<Int> = [];\n    let c: Array<Int> = [];\n    let i = 0;\n    while i < n * n {\n        a.push(0);\n        b.push(0);\n        c.push(0);\n        i = i + 1;\n    }\n    let x = 0;\n    while x < n {\n        let y = 0;\n        while y < n {\n            let z = 0;\n            while z < n {\n                c[(x * n + y + z) % (n * n)] = a[x * n + z] + b[z * n + y];\n                z = z + 1;\n            }\n            y = y + 1;\n        }\n        x = x + 1;\n    }\n    return c[0];\n}\n";

    const FLOAT_MM: &str = "fn Main(): Float {\n    let n = 32;\n    let a: Array<Float> = [];\n    let b: Array<Float> = [];\n    let c: Array<Float> = [];\n    let i = 0;\n    while i < n * n {\n        a.push(0.0);\n        b.push(0.0);\n        c.push(0.0);\n        i = i + 1;\n    }\n    let x = 0;\n    while x < n {\n        let y = 0;\n        while y < n {\n            let s = 0.0;\n            let z = 0;\n            while z < n {\n                s = s + a[x * n + z] * b[z * n + y];\n                z = z + 1;\n            }\n            c[x * n + y] = s;\n            y = y + 1;\n        }\n        x = x + 1;\n    }\n    return c[0];\n}\n";

    const NEST_CALL: &str = "fn Main(): Int {\n    let n = 32;\n    let a: Array<Int> = [];\n    let b: Array<Int> = [];\n    let c: Array<Int> = [];\n    let i = 0;\n    while i < n * n {\n        a.push(0);\n        b.push(0);\n        c.push(0);\n        i = i + 1;\n    }\n    let x = 0;\n    while x < n {\n        let y = 0;\n        while y < n {\n            let s = 0;\n            let z = 0;\n            while z < n {\n                s = s + a[x * n + z] * b[z * n + y];\n                print(\"z:\", z);\n                z = z + 1;\n            }\n            c[x * n + y] = s;\n            y = y + 1;\n        }\n        x = x + 1;\n    }\n    return c[0];\n}\n";

    const NEST_EXIT: &str = "fn Main(): Int {\n    let n = 32;\n    let a: Array<Int> = [];\n    let b: Array<Int> = [];\n    let c: Array<Int> = [];\n    let i = 0;\n    while i < n * n {\n        a.push(0);\n        b.push(0);\n        c.push(0);\n        i = i + 1;\n    }\n    let x = 0;\n    while x < n {\n        let y = 0;\n        while y < n {\n            let s = 0;\n            let z = 0;\n            while z < n {\n                s = s + a[x * n + z] * b[z * n + y];\n                if s > 1000000 {\n                    return s;\n                }\n                z = z + 1;\n            }\n            c[x * n + y] = s;\n            y = y + 1;\n        }\n        x = x + 1;\n    }\n    return c[0];\n}\n";

    const SINGLE_SUM: &str = "fn Main(): Int {\n    let a = [10, 20, 30, 40];\n    let s = 0;\n    let i = 0;\n    while i < 4 {\n        s = s + a[i] * 2;\n        i = i + 1;\n    }\n    return s;\n}\n";

    const BUILDER: &str = "fn Main(): Int {\n    let out: Array<Int> = [];\n    let i = 0;\n    while i < 64 {\n        out.push(i * 2);\n        i = i + 1;\n    }\n    return out[3];\n}\n";

    #[test]
    fn near_miss_shapes_stay_silent() {
        for (name, src) in [
            ("stencil", STENCIL),
            ("float", FLOAT_MM),
            ("call", NEST_CALL),
            ("exit", NEST_EXIT),
            ("single", SINGLE_SUM),
            ("builder", BUILDER),
        ] {
            let module = opt_module(src);
            assert!(
                module.perf_lints.is_empty(),
                "{name} fired {:?}",
                module.perf_lints
            );
        }
    }

    const SCAN_NEST: &str = "fn Main(): Int {\n    let n = 64;\n    let m = 32;\n    let keys: Array<Int> = [];\n    let table: Array<Int> = [];\n    let out: Array<Int> = [];\n    let i = 0;\n    while i < 2048 {\n        keys.push(i % 97);\n        table.push((i * 3) % 101);\n        out.push(0);\n        i = i + 1;\n    }\n    let o = 0;\n    while o < n {\n        let needle = keys[o];\n        let found = -1;\n        let k = 0;\n        while k < m {\n            if table[k] == needle {\n                found = k;\n                break;\n            }\n            k = k + 1;\n        }\n        out[o] = found;\n        o = o + 1;\n    }\n    return out[0] + out[1];\n}\n";

    const SCAN_NEST_STR: &str = "fn Main(): Int {\n    let n = 64;\n    let m = 32;\n    let keys: Array<String> = [];\n    let table: Array<String> = [];\n    let out: Array<Int> = [];\n    let i = 0;\n    while i < 64 {\n        keys.push(\"k\");\n        table.push(\"v\");\n        out.push(0);\n        i = i + 1;\n    }\n    let o = 0;\n    while o < n {\n        let needle = keys[o];\n        let found = -1;\n        let k = 0;\n        while k < m {\n            if table[k] == needle {\n                found = k;\n                break;\n            }\n            k = k + 1;\n        }\n        out[o] = found;\n        o = o + 1;\n    }\n    return out[0];\n}\n";

    const INDEXOF_NEST: &str = "fn Main(): Int {\n    let n = 64;\n    let hay = \"abcdefghijklmnopqrstuvwxyz0123456789\";\n    let needles: Array<String> = [];\n    let outs: Array<Int> = [];\n    let q = 0;\n    while q < 32 {\n        needles.push(\"zz\");\n        outs.push(0);\n        q = q + 1;\n    }\n    let o = 0;\n    while o < n {\n        outs[o] = hay.indexOf(needles[o]);\n        o = o + 1;\n    }\n    return outs[0] + outs[1];\n}\n";

    const SCAN_SINGLE: &str = "fn Main(): Int {\n    let table: Array<Int> = [];\n    let i = 0;\n    while i < 128 {\n        table.push(i * 3);\n        i = i + 1;\n    }\n    let needle = 42;\n    let found = -1;\n    let k = 0;\n    while k < 128 {\n        if table[k] == needle {\n            found = k;\n            break;\n        }\n        k = k + 1;\n    }\n    return found;\n}\n";

    const SCAN_NOEXIT: &str = "fn Main(): Int {\n    let n = 64;\n    let m = 32;\n    let table: Array<Int> = [];\n    let out: Array<Int> = [];\n    let i = 0;\n    while i < 2048 {\n        table.push(i % 5);\n        out.push(0);\n        i = i + 1;\n    }\n    let o = 0;\n    while o < n {\n        let c = 0;\n        let k = 0;\n        while k < m {\n            if table[k] == 3 {\n                c = c + 1;\n            }\n            k = k + 1;\n        }\n        out[o] = c;\n        o = o + 1;\n    }\n    return out[0];\n}\n";

    const SCAN_DISPATCH: &str = "fn Main(): Int {\n    let n = 64;\n    let ops: Array<Int> = [];\n    let keys: Array<Int> = [];\n    let i = 0;\n    while i < 4 {\n        ops.push(i);\n        i = i + 1;\n    }\n    let q = 0;\n    while q < 64 {\n        keys.push(q % 4);\n        q = q + 1;\n    }\n    let o = 0;\n    let acc = 0;\n    while o < n {\n        let needle = keys[o];\n        let k = 0;\n        while k < 4 {\n            if ops[k] == needle {\n                acc = acc + k;\n                break;\n            }\n            k = k + 1;\n        }\n        o = o + 1;\n    }\n    return acc;\n}\n";

    const SCAN_MAPFILTER: &str = "fn Main(): Int {\n    let a: Array<Int> = [];\n    let i = 0;\n    while i < 64 {\n        a.push(i);\n        i = i + 1;\n    }\n    let b = a.map((x) => x * 2);\n    let c = b.filter((x) => x == 4);\n    return c[0];\n}\n";

    const SCAN_DYNCALL: &str = "class Box {\n    fn get(self, i: Int): Int {\n        return i * 2;\n    }\n}\nfn Main(): Int {\n    let n = 64;\n    let m = 32;\n    let b = new Box();\n    let o = 0;\n    let acc = 0;\n    while o < n {\n        let k = 0;\n        while k < m {\n            if b.get(k) == o {\n                acc = acc + 1;\n                break;\n            }\n            k = k + 1;\n        }\n        o = o + 1;\n    }\n    return acc;\n}\n";

    const INDEXOF_CONST: &str = "fn Main(): Int {\n    let hay = \"abcdefghijklmnopqrstuvwxyz\";\n    let at = hay.indexOf(\"mn\");\n    return at;\n}\n";

    fn p202_lints(module: &Module) -> Vec<&PerfLint> {
        module.perf_lints.iter().filter(|l| l.code == PerfCode::P202).collect()
    }

    #[test]
    fn nested_scan_fires_p202() {
        let module = opt_module(SCAN_NEST);
        let fires = p202_lints(&module);
        assert_eq!(fires.len(), 1, "{:?}", module.perf_lints);
        assert!(fires[0].message.contains("linear scan over array of length 32"));
        assert!(fires[0].message.contains("inside loop of 64 iterations"));
        assert!(fires[0].message.contains("2048 comparisons"));
        assert!(fires[0].hint.contains("lookup table"));
    }

    #[test]
    fn nested_str_scan_fires_p202() {
        let module = opt_module(SCAN_NEST_STR);
        let fires = p202_lints(&module);
        assert_eq!(fires.len(), 1, "{:?}", module.perf_lints);
        assert!(fires[0].message.contains("linear scan over string"));
    }

    #[test]
    fn indexof_in_loop_fires_p202() {
        let module = opt_module(INDEXOF_NEST);
        let fires = p202_lints(&module);
        assert_eq!(fires.len(), 1, "{:?}", module.perf_lints);
        assert!(fires[0].message.contains("indexOf"));
        assert!(fires[0].hint.contains("precomputed"));
    }

    #[test]
    fn p202_near_miss_shapes_stay_silent() {
        for (name, src) in [
            ("single", SCAN_SINGLE),
            ("noexit", SCAN_NOEXIT),
            ("dispatch", SCAN_DISPATCH),
            ("mapfilter", SCAN_MAPFILTER),
            ("dyncall", SCAN_DYNCALL),
            ("constidx", INDEXOF_CONST),
        ] {
            let module = opt_module(src);
            let fires: Vec<&PerfLint> = module
                .perf_lints
                .iter()
                .filter(|l| l.code == PerfCode::P202)
                .collect();
            assert!(fires.is_empty(), "{name} fired {fires:?}");
        }
    }

    #[test]
    fn p202_stream_matches_across_jobs() {
        for src in [SCAN_NEST, INDEXOF_NEST] {
            let seq = opt_module(src);
            let mut par = lower_module(src);
            crate::opt::optimize_lir_parallel(&mut par, 1, "Main", 4);
            let seq_lints: Vec<&PerfLint> = seq
                .perf_lints
                .iter()
                .filter(|l| l.code == PerfCode::P202)
                .collect();
            let par_lints: Vec<&PerfLint> = par
                .perf_lints
                .iter()
                .filter(|l| l.code == PerfCode::P202)
                .collect();
            assert_eq!(
                format!("{seq_lints:?}"),
                format!("{par_lints:?}"),
                "jobs diverge"
            );
        }
    }

    #[test]
    fn p202_diagnostics_carry_warn_codes() {
        for (src, code) in [(SCAN_NEST, Code::P202), (INDEXOF_NEST, Code::P202)] {
            let module = opt_module(src);
            let diags = to_diagnostics(&module.perf_lints);
            assert_eq!(diags.len(), 1, "{code:?}: {diags:?}");
            assert_eq!(diags[0].code, code);
            assert!(diags[0].code.is_warning());
            assert!(diags[0].code.is_perf_lint());
            assert!(!diags[0].code.is_perf_note());
            assert!(diags[0].hint.is_some());
        }
    }

    fn lower_bench(path: &str) -> Option<Module> {
        let dir = std::env::temp_dir().join(format!("rnx-perflint-corpus-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).ok()?;
        let src = std::fs::read_to_string(path).ok()?;
        std::fs::write(dir.join("main.rnx"), &src).ok()?;
        let g = frontend::modules::ModuleGraph::build(&dir.join("main.rnx")).ok()?;
        let mut m = g.resolve().ok()?;
        if !frontend::desugar::desugar(&mut m).is_empty() {
            return None;
        }
        let mut out = crate::lower::lower(&m).ok()?;
        crate::opt::optimize_lir(&mut out, 1, "Main");
        let _ = std::fs::remove_dir_all(&dir);
        Some(out)
    }

    #[test]
    fn corpus_receipt_note_audit() {
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../benches");
        let mut kinds: Vec<String> = Vec::new();
        for sub in ["compute", "memory"] {
            let dir = format!("{root}/{sub}");
            let mut subs: Vec<String> = Vec::new();
            for e in std::fs::read_dir(&dir).expect("read benches") {
                let e = e.expect("entry");
                if e.path().is_dir() {
                    subs.push(e.file_name().to_string_lossy().to_string());
                }
            }
            subs.sort();
            for kind in subs {
                kinds.push(format!("{sub}/{kind}"));
            }
        }
        kinds.sort();
        assert!(kinds.len() >= 7, "corpus too small: {kinds:?}");
        let mut p101 = 0usize;
        let mut p102 = 0usize;
        let mut p201: Vec<String> = Vec::new();
        let mut p202: Vec<String> = Vec::new();
        for kind in &kinds {
            let module = lower_bench(&format!("{root}/{kind}/main.rnx")).expect("bench lowers");
            let mut notes: Vec<(ReceiptKind, Span)> = Vec::new();
            for lint in &module.perf_lints {
                match lint.code {
                    PerfCode::P101 => {
                        p101 += 1;
                        let kind = module
                            .transform_receipts
                            .iter()
                            .find(|r| r.func == lint.func && r.span == lint.span)
                            .map(|r| r.kind);
                        assert!(
                            matches!(
                                kind,
                                Some(ReceiptKind::AccumV1) | Some(ReceiptKind::AccumPeel)
                            ),
                            "{kind:?} P101 without receipt: {lint:?}"
                        );
                        notes.push((kind.expect("receipt"), lint.span));
                    }
                    PerfCode::P102 => {
                        p102 += 1;
                        assert!(
                            module.transform_receipts.iter().any(|r| r.func == lint.func
                                && r.kind == ReceiptKind::BceVersioned
                                && r.span == lint.span),
                            "{kind} P102 without receipt: {lint:?}"
                        );
                    }
                    PerfCode::P201 => p201.push(format!("{kind}:{}", lint.func)),
                    PerfCode::P202 => p202.push(format!(
                        "{kind}:{}@{}..{} {}",
                        lint.func, lint.span.start, lint.span.end, lint.message
                    )),
                }
            }
            for receipt in &module.transform_receipts {
                let covered = module.perf_lints.iter().any(|l| {
                    l.func == receipt.func
                        && l.span == receipt.span
                        && match receipt.kind {
                            ReceiptKind::AccumV1 | ReceiptKind::AccumPeel => {
                                l.code == PerfCode::P101
                            }
                            ReceiptKind::BceVersioned => l.code == PerfCode::P102,
                        }
                });
                assert!(covered, "{kind} receipt without note: {receipt:?}");
            }
            for missed in &module.missed_reductions {
                let gated = missed.inner_trip >= accum::MIN_ACCUM_TRIP;
                let covered = module.perf_lints.iter().any(|l| {
                    l.code == PerfCode::P201 && l.func == missed.func && l.span == missed.span
                });
                assert_eq!(
                    covered, gated,
                    "{kind} missed record coverage wrong: {missed:?}"
                );
            }
        }
        assert!(p201.is_empty(), "P201 fired on corpus: {p201:?}");
        assert!(p202.is_empty(), "P202 fired on corpus: {p202:?}");
        eprintln!("corpus audit: kinds={kinds:?} p101={p101} p102={p102}");
    }

    #[test]
    fn lint_stream_matches_across_jobs() {
        let src = matmul_src();
        let seq = opt_module(&src);
        let mut par = lower_module(&src);
        crate::opt::optimize_lir_parallel(&mut par, 1, "Main", 4);
        assert_eq!(
            format!("{:?}", seq.transform_receipts),
            format!("{:?}", par.transform_receipts)
        );
        assert_eq!(
            format!("{:?}", seq.perf_lints),
            format!("{:?}", par.perf_lints)
        );
    }

    fn line_of(src: &str, offset: u32) -> String {
        let off = (offset as usize).min(src.len());
        let start = src[..off].rfind('\n').map(|i| i + 1).unwrap_or(0);
        let end = src[off..].find('\n').map(|i| off + i).unwrap_or(src.len());
        src[start..end].to_string()
    }

    #[test]
    fn receipt_spans_resolve_to_source_lines() {
        let src = matmul_src();
        let module = opt_module(&src);
        assert!(!module.transform_receipts.is_empty());
        for receipt in &module.transform_receipts {
            assert!(
                (receipt.span.start as usize) < src.len(),
                "receipt span out of range: {receipt:?}"
            );
            let line = line_of(&src, receipt.span.start);
            match receipt.kind {
                ReceiptKind::AccumV1 | ReceiptKind::AccumPeel => {
                    assert!(line.contains("c["), "store line: {line}");
                }
                ReceiptKind::BceVersioned => {
                    assert!(line.contains("while"), "header line: {line}");
                }
            }
        }
        for lint in &module.perf_lints {
            assert!((lint.span.start as usize) < src.len());
        }
    }

    #[test]
    fn perflint_row_present_and_bounded() {
        let mut module = lower_module(&matmul_src());
        let t = crate::opt::optimize_lir_timed(&mut module, 1, "Main");
        let rows = t.sub_rows();
        assert_eq!(rows.len(), 14);
        assert_eq!(rows[13].0, "opt_perflint");
        assert_eq!(rows.iter().map(|(_, d)| *d).sum::<Duration>(), t.total());
        assert!(
            t.perflint < Duration::from_millis(50),
            "perflint took {:?}",
            t.perflint
        );
        eprintln!("opt_perflint on matmul-256: {:?}", t.perflint);
    }

    const VERSION_PROBE: &str = "fn Main(n: Int, a: Array<Int>): Int {\n    let i = 0;\n    let s = 0;\n    while i < n {\n        s = s + a[i];\n        i = i + 1;\n    }\n    return s;\n}\n";

    #[test]
    fn versioned_loop_fires_p102_with_receipt() {
        let module = opt_module(VERSION_PROBE);
        let kinds = receipt_kinds(&module);
        assert!(
            kinds.contains(&ReceiptKind::BceVersioned),
            "no version receipt: {kinds:?}"
        );
        let p102: Vec<&PerfLint> = lints_of(&module)
            .iter()
            .filter(|l| l.code == PerfCode::P102)
            .collect();
        assert_eq!(
            p102.len(),
            kinds
                .iter()
                .filter(|k| **k == ReceiptKind::BceVersioned)
                .count()
        );
        assert!(p102.iter().all(|l| l.message.contains("versioned")));
    }

    const INLINE_VERSION_PROBE: &str = "fn Fill(n: Int, a: Array<Int>): Int {\n    let i = 0;\n    let s = 0;\n    while i < n {\n        s = s + a[i];\n        i = i + 1;\n    }\n    return s;\n}\nfn Main(): Int {\n    let a: Array<Int> = [];\n    a.push(1);\n    a.push(2);\n    a.push(3);\n    return Fill(3, a);\n}\n";

    #[test]
    fn dead_function_records_are_dropped() {
        let module = opt_module(INLINE_VERSION_PROBE);
        assert_eq!(
            module.functions.iter().filter(|f| f.name == "Fill").count(),
            0
        );
        assert!(
            module.transform_receipts.iter().all(|r| r.func == "Main"),
            "{:?}",
            module.transform_receipts
        );
        assert!(module.missed_reductions.iter().all(|r| r.func == "Main"));
        for receipt in &module.transform_receipts {
            assert!(
                module
                    .perf_lints
                    .iter()
                    .any(|l| l.func == receipt.func && l.span == receipt.span),
                "receipt without note: {receipt:?}"
            );
        }
    }
}
