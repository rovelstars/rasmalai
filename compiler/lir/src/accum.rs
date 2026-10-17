use crate::bce::{
    all_defs, analyze_counted, array_aliases, external_preds, idx_affine, idx_interval,
    loop_latches, nest_has_exits, AffineForm, CountedLoop,
};
use crate::instr::*;
use crate::licm::{compute_dominators, find_natural_loops, DominatorTree, NaturalLoop};
use crate::opt::{instr_dst, instr_reads, term_reads};
use diagnostics::Span;
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

#[cfg(not(target_arch = "wasm32"))]
fn stamp() -> Instant {
    Instant::now()
}

#[cfg(target_arch = "wasm32")]
fn stamp() -> WasmStamp {
    WasmStamp
}

#[cfg(target_arch = "wasm32")]
#[derive(Clone, Copy)]
struct WasmStamp;
#[cfg(target_arch = "wasm32")]
impl WasmStamp {
    fn elapsed(&self) -> Duration {
        Duration::default()
    }
}

pub const MIN_ACCUM_TRIP: i64 = 16;
pub const MAX_REWRITE_BODY_INSTRS: usize = 64;
const MAX_DEF_WALK: usize = 64;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AccumAction {
    Untouched,
    V1,
    Peel,
}

#[derive(Default, Clone, Copy, Debug)]
pub struct AccumTimes {
    pub shape: Duration,
    pub zero: Duration,
    pub alias: Duration,
    pub rewrite: Duration,
}

impl AccumTimes {
    pub fn total(&self) -> Duration {
        self.shape + self.zero + self.alias + self.rewrite
    }
}

#[derive(Clone, Debug)]
pub(crate) struct LoopFacts {
    pub(crate) header: BlockId,
    pub(crate) then_bb: BlockId,
    pub(crate) exit_bb: BlockId,
    pub(crate) latch: BlockId,
    pub(crate) iv: Local,
    pub(crate) cond: Local,
    pub(crate) bound_local: Local,
    pub(crate) init_const: i64,
    pub(crate) bound_const: i64,
    pub(crate) pre: BlockId,
    pub(crate) init_instrs: Vec<(BlockId, usize)>,
    pub(crate) blocks: BTreeSet<BlockId>,
    pub(crate) counted: CountedLoop,
}

#[derive(Clone, Debug)]
struct Triple {
    s: Local,
    init_instrs: Vec<(BlockId, usize)>,
    add_block: BlockId,
    add_idx: usize,
    update_copy: Option<(BlockId, usize)>,
    m: Local,
    mul_block: BlockId,
    mul_idx: usize,
    mul_span: Span,
    load_a_block: BlockId,
    load_a_idx: usize,
    arr_a: Local,
    idx_a: Local,
    avec: Local,
    load_b_block: BlockId,
    load_b_idx: usize,
    arr_b: Local,
    idx_b: Local,
    load_b_dst: Local,
    store_block: BlockId,
    store_idx: usize,
    store_span: Span,
    store_elem: usize,
    arr_c: Local,
    idx_c: Local,
    form_a: AffineForm,
    form_b: AffineForm,
    form_c: AffineForm,
    y_incr: IncrMove,
    z_incr: IncrMove,
}

#[derive(Clone, Debug)]
struct IncrMove {
    main: Vec<(BlockId, usize)>,
    konst: IncrConst,
}

#[derive(Clone, Debug)]
enum IncrConst {
    Move(BlockId, usize),
    Fresh(i64),
}

#[derive(Clone, Debug)]
struct NestMatch {
    x: LoopFacts,
    y: LoopFacts,
    z: LoopFacts,
    triple: Triple,
}

fn header_cmp(func: &Function, header: BlockId) -> Option<(Local, Local, Local, BlockId)> {
    let block = func.blocks.get(header)?;
    let (cond, then_bb) = match &block.term {
        Terminator::BrIf { cond, then_bb, .. } => (*cond, *then_bb),
        _ => return None,
    };
    for ins in &block.instrs {
        match ins {
            Instr::Cmp { op: CmpOp::Lt, kind: NumKind::Int, dst, lhs, rhs, .. } if *dst == cond => {
                return Some((*lhs, *rhs, cond, then_bb));
            }
            Instr::Cmp { op: CmpOp::Gt, kind: NumKind::Int, dst, lhs, rhs, .. } if *dst == cond => {
                return Some((*rhs, *lhs, cond, then_bb));
            }
            _ => {}
        }
    }
    None
}

fn exit_of(func: &Function, header: BlockId) -> Option<BlockId> {
    match func.blocks.get(header)?.term {
        Terminator::BrIf { else_bb, .. } => Some(else_bb),
        _ => None,
    }
}

pub(crate) fn counted_facts(func: &Function, lp: &NaturalLoop) -> Option<LoopFacts> {
    let counted = analyze_counted(func, lp)?;
    let (iv, bound_local, cond, then_bb) = header_cmp(func, lp.header)?;
    if iv != counted.iv || then_bb != counted.then_bb {
        return None;
    }
    let init_const = match &counted.init {
        crate::bce::InitExpr::Const(c) => *c,
        _ => return None,
    };
    let latches = loop_latches(func, &lp.blocks, lp.header);
    if latches.len() != 1 {
        return None;
    }
    let externals = external_preds(func, lp.header, &lp.blocks);
    if externals.len() != 1 {
        return None;
    }
    let pre = externals[0];
    if !matches!(func.blocks.get(pre).map(|b| &b.term), Some(Terminator::Br(t)) if *t == lp.header)
    {
        return None;
    }
    let outside: Vec<(BlockId, usize)> = all_defs(func, iv)
        .into_iter()
        .filter(|(bi, _)| !lp.blocks.contains(bi))
        .collect();
    if outside.len() != 1 {
        return None;
    }
    let (obi, oii) = outside[0];
    if oii == usize::MAX || obi != pre {
        return None;
    }
    let mut init_instrs: Vec<(BlockId, usize)> = Vec::new();
    match func.blocks.get(obi)?.instrs.get(oii)? {
        Instr::Const { lit: Lit::Int(c), .. } if *c == init_const => {
            init_instrs.push((obi, oii));
        }
        Instr::Copy { src, .. } => {
            let (tbi, tii) = single_def(func, *src)?;
            if tbi != obi {
                return None;
            }
            match func.blocks.get(tbi)?.instrs.get(tii)? {
                Instr::Const { lit: Lit::Int(c), .. } if *c == init_const => {
                    if use_count(func, *src) != 1 {
                        return None;
                    }
                    init_instrs.push((tbi, tii));
                    init_instrs.push((obi, oii));
                    init_instrs.sort();
                }
                _ => return None,
            }
        }
        _ => return None,
    }
    Some(LoopFacts {
        header: lp.header,
        then_bb: counted.then_bb,
        exit_bb: exit_of(func, lp.header)?,
        latch: latches[0],
        iv,
        cond,
        bound_local,
        init_const,
        bound_const: counted.bound,
        pre,
        init_instrs,
        blocks: lp.blocks.clone(),
        counted,
    })
}

fn use_count(func: &Function, local: Local) -> usize {
    let mut n = 0usize;
    let mut tmp: Vec<Local> = Vec::new();
    for b in &func.blocks {
        for ins in &b.instrs {
            tmp.clear();
            instr_reads(ins, &mut tmp);
            if tmp.contains(&local) {
                n += 1;
            }
        }
        tmp.clear();
        term_reads(&b.term, &mut tmp);
        if tmp.contains(&local) {
            n += 1;
        }
    }
    n
}

fn use_sites(func: &Function) -> BTreeMap<Local, Vec<(BlockId, usize)>> {
    let mut out: BTreeMap<Local, Vec<(BlockId, usize)>> = BTreeMap::new();
    let mut tmp: Vec<Local> = Vec::new();
    for (bi, block) in func.blocks.iter().enumerate() {
        for (ii, ins) in block.instrs.iter().enumerate() {
            tmp.clear();
            instr_reads(ins, &mut tmp);
            for l in tmp.iter() {
                out.entry(*l).or_default().push((bi, ii));
            }
        }
        tmp.clear();
        term_reads(&block.term, &mut tmp);
        for l in tmp.iter() {
            out.entry(*l).or_default().push((bi, usize::MAX));
        }
    }
    out
}

fn single_def(func: &Function, local: Local) -> Option<(BlockId, usize)> {
    let defs = all_defs(func, local);
    if defs.len() != 1 || defs[0].1 == usize::MAX {
        return None;
    }
    Some(defs[0])
}

pub(crate) fn union_banned(func: &Function, union_blocks: &BTreeSet<BlockId>) -> bool {
    for b in union_blocks {
        let block = match func.blocks.get(*b) {
            Some(x) => x,
            None => return true,
        };
        let mut banned = false;
        crate::instr::walk_instrs(&block.instrs, &mut |ins| {
            if banned {
                return;
            }
            match ins {
                Instr::Call { .. } | Instr::ArrayPush { .. } | Instr::ArrayPop { .. } => {
                    banned = true;
                }
                Instr::Arith { kind: NumKind::Float(_), .. } | Instr::Fma { .. } => {
                    banned = true;
                }
                _ => {}
            }
        });
        if banned {
            return true;
        }
    }
    false
}

fn take_incr(func: &Function, lp: &LoopFacts) -> Option<IncrMove> {
    let in_loop: Vec<(BlockId, usize)> = all_defs(func, lp.iv)
        .into_iter()
        .filter(|(bi, _)| lp.blocks.contains(bi))
        .collect();
    if in_loop.len() != 1 {
        return None;
    }
    let (cb, ci) = in_loop[0];
    if ci == usize::MAX {
        return None;
    }
    match func.blocks.get(cb)?.instrs.get(ci)?.clone() {
        Instr::Copy { dst, src, .. } if dst == lp.iv => {
            let (ab, ai) = single_def(func, src)?;
            let other = match func.blocks.get(ab)?.instrs.get(ai)?.clone() {
                Instr::Arith { op: ArithOp::Add, kind: NumKind::Int, dst, lhs, rhs, .. }
                    if dst == src && (lhs == lp.iv || rhs == lp.iv) =>
                {
                    if lhs == lp.iv {
                        rhs
                    } else {
                        lhs
                    }
                }
                _ => return None,
            };
            let v = const_eval(func, other)?;
            let konst = incr_const(func, other, v)?;
            Some(IncrMove { main: vec![(ab, ai), (cb, ci)], konst })
        }
        Instr::Arith { op: ArithOp::Add, kind: NumKind::Int, dst, lhs, rhs, .. }
            if dst == lp.iv && (lhs == lp.iv || rhs == lp.iv) =>
        {
            let other = if lhs == lp.iv { rhs } else { lhs };
            let v = const_eval(func, other)?;
            let konst = incr_const(func, other, v)?;
            Some(IncrMove { main: vec![(cb, ci)], konst })
        }
        _ => None,
    }
}

fn incr_const(func: &Function, other: Local, v: i64) -> Option<IncrConst> {
    match single_def(func, other) {
        Some((kb, ki)) => match func.blocks.get(kb)?.instrs.get(ki)? {
            Instr::Const { lit: Lit::Int(n), .. } if *n == v => {
                if use_count(func, other) == 1 {
                    Some(IncrConst::Move(kb, ki))
                } else {
                    Some(IncrConst::Fresh(v))
                }
            }
            _ => Some(IncrConst::Fresh(v)),
        },
        None => Some(IncrConst::Fresh(v)),
    }
}

fn find_candidate(func: &Function, dom: &DominatorTree) -> Option<NestMatch> {
    find_candidate_locating(func, dom, None)
}

fn find_candidate_locating(
    func: &Function,
    dom: &DominatorTree,
    mut located: Option<&mut Vec<[LoopFacts; 3]>>,
) -> Option<NestMatch> {
    let loops = find_natural_loops(func, dom);
    let mut counted: Vec<LoopFacts> =
        loops.iter().filter_map(|lp| counted_facts(func, lp)).collect();
    counted.sort_by_key(|l| l.header);
    let mut headers: Vec<BlockId> = counted.iter().map(|l| l.header).collect();
    headers.sort();
    headers.dedup();
    for inner_header in headers {
        let inner = match counted.iter().find(|l| l.header == inner_header) {
            Some(v) => v.clone(),
            None => continue,
        };
        let mut nest: Vec<LoopFacts> = counted
            .iter()
            .filter(|o| o.blocks.is_superset(&inner.blocks))
            .cloned()
            .collect();
        nest.sort_by(|a, b| {
            b.blocks.len().cmp(&a.blocks.len()).then(a.header.cmp(&b.header))
        });
        if nest.len() != 3 {
            continue;
        }
        let mut sizes: Vec<usize> = nest.iter().map(|l| l.blocks.len()).collect();
        sizes.sort_by(|a, b| b.cmp(a));
        let mut strict = true;
        for w in sizes.windows(2) {
            if w[0] <= w[1] {
                strict = false;
            }
        }
        if !strict {
            continue;
        }
        let union_blocks: BTreeSet<BlockId> =
            nest.iter().flat_map(|l| l.blocks.iter().copied()).collect();
        let hdrs: BTreeSet<BlockId> = nest.iter().map(|l| l.header).collect();
        if nest_has_exits(func, &union_blocks, &hdrs) {
            continue;
        }
        if union_banned(func, &union_blocks) {
            continue;
        }
        let ivs: BTreeSet<Local> = nest.iter().map(|l| l.iv).collect();
        if ivs.len() != 3 {
            continue;
        }
        if let [x, y, z] = &nest[..] {
            if let Some(out) = located.as_mut() {
                out.push([x.clone(), y.clone(), z.clone()]);
            }
        }
        let refs: Vec<&CountedLoop> =
            vec![&nest[0].counted, &nest[1].counted, &nest[2].counted];
        let uses = use_sites(func);
        if let Some(triple) = reduction_shape(func, dom, &refs, &nest, &uses) {
            return Some(NestMatch { x: nest[0].clone(), y: nest[1].clone(), z: nest[2].clone(), triple });
        }
    }
    None
}

fn is_array_i64(func: &Function, local: Local) -> bool {
    matches!(func.locals.get(local as usize), Some(LirType::Array(inner)) if matches!(inner.as_ref(), LirType::I64))
}

fn form_keys_subset(form: &AffineForm, allowed: &[Local]) -> bool {
    form.coeffs.keys().all(|k| allowed.contains(k))
}

fn mem_sites(func: &Function, union_blocks: &BTreeSet<BlockId>) -> Vec<(BlockId, usize)> {
    let mut out = Vec::new();
    let mut ordered: Vec<BlockId> = union_blocks.iter().copied().collect();
    ordered.sort();
    for b in ordered {
        if let Some(block) = func.blocks.get(b) {
            for (ii, ins) in block.instrs.iter().enumerate() {
                if matches!(ins, Instr::ArrayGet { .. } | Instr::ArraySet { .. }) {
                    out.push((b, ii));
                }
            }
        }
    }
    out
}

fn reduction_shape(
    func: &Function,
    dom: &DominatorTree,
    nest: &[&CountedLoop],
    facts: &[LoopFacts],
    uses: &BTreeMap<Local, Vec<(BlockId, usize)>>,
) -> Option<Triple> {
    let (x, y, z) = (&facts[0], &facts[1], &facts[2]);
    let union_blocks: BTreeSet<BlockId> =
        facts.iter().flat_map(|l| l.blocks.iter().copied()).collect();
    let z_body: BTreeSet<BlockId> =
        z.blocks.iter().copied().filter(|b| *b != z.header).collect();
    let y_outer: BTreeSet<BlockId> =
        y.blocks.iter().copied().filter(|b| !z.blocks.contains(b)).collect();
    let mut ordered_body: Vec<BlockId> = z_body.iter().copied().collect();
    ordered_body.sort();
    for ab in ordered_body {
        let count = func.blocks.get(ab).map(|b| b.instrs.len()).unwrap_or(0);
        for ai in 0..count {
            let (t, lhs, rhs) = match func.blocks.get(ab).and_then(|b| b.instrs.get(ai)) {
                Some(Instr::Arith { op: ArithOp::Add, kind: NumKind::Int, dst, lhs, rhs, .. }) => {
                    (*dst, *lhs, *rhs)
                }
                _ => continue,
            };
            for (s_cand, m) in [(lhs, rhs), (rhs, lhs)] {
                if let Some(tri) = check_update(
                    func, dom, nest, facts, uses, &union_blocks, &z_body, &y_outer, ab, ai, t,
                    s_cand, m,
                ) {
                    let _ = x;
                    return Some(tri);
                }
            }
        }
    }
    None
}

#[allow(clippy::too_many_arguments)]
fn check_update(
    func: &Function,
    dom: &DominatorTree,
    nest: &[&CountedLoop],
    facts: &[LoopFacts],
    uses: &BTreeMap<Local, Vec<(BlockId, usize)>>,
    union_blocks: &BTreeSet<BlockId>,
    z_body: &BTreeSet<BlockId>,
    y_outer: &BTreeSet<BlockId>,
    ab: BlockId,
    ai: usize,
    t: Local,
    s_cand: Local,
    m: Local,
) -> Option<Triple> {
    let (x, y, z) = (&facts[0], &facts[1], &facts[2]);
    if !matches!(func.locals.get(s_cand as usize), Some(LirType::I64)) {
        return None;
    }
    let (mb, mi) = single_def(func, m)?;
    if !z_body.contains(&mb) {
        return None;
    }
    let (p, q, mul_span) = match func.blocks.get(mb)?.instrs.get(mi)? {
        Instr::Arith { op: ArithOp::Mul, kind: NumKind::Int, dst, lhs, rhs, span, .. }
            if *dst == m =>
        {
            (*lhs, *rhs, *span)
        }
        _ => return None,
    };
    if ab != mb {
        return None;
    }
    if uses.get(&m).map(|v| v.len()).unwrap_or(0) != 1 {
        return None;
    }
    if t != s_cand && uses.get(&t).map(|v| v.len()).unwrap_or(0) != 1 {
        return None;
    }
    let (load_a_block, load_a_idx, arr_a, idx_a, avec) = single_use_get(func, uses, p, z_body)?;
    let (load_b_block, load_b_idx, arr_b, idx_b, load_b_dst) = single_use_get(func, uses, q, z_body)?;
    if load_a_block != ab || load_b_block != ab {
        return None;
    }
    if !is_array_i64(func, arr_a) || !is_array_i64(func, arr_b) {
        return None;
    }
    let s_defs = all_defs(func, s_cand);
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
    let mut init_temp: Option<(BlockId, usize)> = None;
    for (db, di) in &s_defs {
        if (*db, *di) == update_def {
            continue;
        }
        if init_def.is_some() {
            return None;
        }
        match func.blocks.get(*db)?.instrs.get(*di)? {
            Instr::Const { lit: Lit::Int(0), dst, .. } if *dst == s_cand => {
                init_def = Some((*db, *di));
            }
            Instr::Copy { dst, src, .. } if *dst == s_cand => {
                let (tb, ti) = single_def(func, *src)?;
                match func.blocks.get(tb)?.instrs.get(ti)? {
                    Instr::Const { lit: Lit::Int(0), .. } => {
                        if uses.get(src).map(|v| v.len()).unwrap_or(0) != 1 {
                            return None;
                        }
                        if tb != *db {
                            return None;
                        }
                        init_temp = Some((tb, ti));
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
    let mut init_instrs = vec![init_def];
    if let Some(tp) = init_temp {
        init_instrs.push(tp);
        init_instrs.sort();
    }
    let mut store_site: Option<(BlockId, usize, Span, usize, Local, Local)> = None;
    let mut ordered_outer: Vec<BlockId> = y_outer.iter().copied().collect();
    ordered_outer.sort();
    for sb in ordered_outer {
        let count = func.blocks.get(sb).map(|b| b.instrs.len()).unwrap_or(0);
        for si in 0..count {
            match func.blocks.get(sb)?.instrs.get(si)? {
                Instr::ArraySet { arr, index, value, elem_size, span, .. }
                    if *value == s_cand =>
                {
                    if store_site.is_some() {
                        return None;
                    }
                    store_site = Some((sb, si, *span, *elem_size, *arr, *index));
                }
                _ => {}
            }
        }
    }
    let (store_block, store_idx, store_span, store_elem, arr_c, idx_c) = store_site?;
    if !is_array_i64(func, arr_c) {
        return None;
    }
    if !dom.dominates(z.exit_bb, store_block) {
        return None;
    }
    if !dom.dominates(store_block, y.latch) {
        return None;
    }
    let s_use_sites: BTreeSet<(BlockId, usize)> = uses
        .get(&s_cand)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .collect();
    if s_use_sites.iter().any(|(_, ui)| *ui == usize::MAX) {
        return None;
    }
    let expected: BTreeSet<(BlockId, usize)> =
        BTreeSet::from([(ab, ai), (store_block, store_idx)]);
    if s_use_sites != expected {
        return None;
    }
    let form_a = idx_affine(func, nest, dom, load_a_block, load_a_idx, idx_a, 0)?;
    let form_b = idx_affine(func, nest, dom, load_b_block, load_b_idx, idx_b, 0)?;
    let form_c = idx_affine(func, nest, dom, store_block, store_idx, idx_c, 0)?;
    if !form_keys_subset(&form_a, &[x.iv, z.iv]) {
        return None;
    }
    if !form_keys_subset(&form_b, &[z.iv, y.iv]) {
        return None;
    }
    if !form_keys_subset(&form_c, &[x.iv, y.iv]) {
        return None;
    }
    let mut sites = mem_sites(func, union_blocks);
    sites.sort();
    let mut triple_sites =
        vec![(load_a_block, load_a_idx), (load_b_block, load_b_idx), (store_block, store_idx)];
    triple_sites.sort();
    if sites != triple_sites {
        return None;
    }
    if z.then_bb != ab || z.latch != ab {
        return None;
    }
    let y_incr = take_incr(func, y)?;
    let z_incr = take_incr(func, z)?;
    if y_incr.main.iter().any(|(b, _)| *b != y.latch) {
        return None;
    }
    if z_incr.main.iter().any(|(b, _)| *b != ab) {
        return None;
    }
    if let IncrConst::Move(kb, _) = &y_incr.konst {
        if *kb != y.latch {
            return None;
        }
    }
    if let IncrConst::Move(kb, _) = &z_incr.konst {
        if *kb != ab {
            return None;
        }
    }
    Some(Triple {
        s: s_cand,
        init_instrs,
        add_block: ab,
        add_idx: ai,
        update_copy: if update_def != (ab, ai) { Some(update_def) } else { None },
        m,
        mul_block: mb,
        mul_idx: mi,
        mul_span,
        load_a_block,
        load_a_idx,
        arr_a,
        idx_a,
        avec,
        load_b_block,
        load_b_idx,
        arr_b,
        idx_b,
        load_b_dst,
        store_block,
        store_idx,
        store_span,
        store_elem,
        arr_c,
        idx_c,
        form_a,
        form_b,
        form_c,
        y_incr,
        z_incr,
    })
}

fn single_use_get(
    func: &Function,
    uses: &BTreeMap<Local, Vec<(BlockId, usize)>>,
    local: Local,
    z_body: &BTreeSet<BlockId>,
) -> Option<(BlockId, usize, Local, Local, Local)> {
    if uses.get(&local).map(|v| v.len()).unwrap_or(0) != 1 {
        return None;
    }
    let (db, di) = single_def(func, local)?;
    if !z_body.contains(&db) {
        return None;
    }
    match func.blocks.get(db)?.instrs.get(di)? {
        Instr::ArrayGet { dst, arr, index, .. } if *dst == local => {
            Some((db, di, *arr, *index, local))
        }
        _ => return None,
    }
}

fn const_eval(func: &Function, local: Local) -> Option<i64> {
    let mut seen: BTreeSet<Local> = BTreeSet::new();
    const_eval_depth(func, local, 0, &mut seen)
}

fn const_eval_depth(
    func: &Function,
    local: Local,
    depth: usize,
    seen: &mut BTreeSet<Local>,
) -> Option<i64> {
    if depth > 8 || !seen.insert(local) {
        return None;
    }
    let out = const_eval_inner(func, local, depth, seen);
    seen.remove(&local);
    out
}

fn const_eval_inner(
    func: &Function,
    local: Local,
    depth: usize,
    seen: &mut BTreeSet<Local>,
) -> Option<i64> {
    let defs = all_defs(func, local);
    if defs.len() != 1 || defs[0].1 == usize::MAX {
        return None;
    }
    let (bi, ii) = defs[0];
    match func.blocks.get(bi)?.instrs.get(ii)? {
        Instr::Const { lit: Lit::Int(n), .. } => Some(*n),
        Instr::Copy { src, .. } => const_eval_depth(func, *src, depth + 1, seen),
        Instr::Arith { op: ArithOp::Add, kind: NumKind::Int, lhs, rhs, .. } => {
            const_eval_depth(func, *lhs, depth + 1, seen)?
                .checked_add(const_eval_depth(func, *rhs, depth + 1, seen)?)
        }
        Instr::Arith { op: ArithOp::Sub, kind: NumKind::Int, lhs, rhs, .. } => {
            const_eval_depth(func, *lhs, depth + 1, seen)?
                .checked_sub(const_eval_depth(func, *rhs, depth + 1, seen)?)
        }
        Instr::Arith { op: ArithOp::Mul, kind: NumKind::Int, lhs, rhs, .. } => {
            const_eval_depth(func, *lhs, depth + 1, seen)?
                .checked_mul(const_eval_depth(func, *rhs, depth + 1, seen)?)
        }
        _ => None,
    }
}

fn alias_safe(
    func: &Function,
    dom: &DominatorTree,
    nest: &[&CountedLoop],
    m: &NestMatch,
) -> bool {
    let t = &m.triple;
    let fa = array_aliases(func, t.arr_a);
    let fc = array_aliases(func, t.arr_c);
    if fa.is_disjoint(&fc) {
        let fresh = crate::escape::fresh_array_roots(func);
        match (fresh.get(&t.arr_a), fresh.get(&t.arr_c)) {
            (Some(ra), Some(rb)) => return ra != rb,
            _ => return false,
        }
    }
    let (alo, ahi) = match idx_interval(func, nest, dom, t.load_a_block, t.load_a_idx, t.idx_a, 0)
    {
        Some((lo, hi, _)) => (lo, hi),
        None => return false,
    };
    let (clo, chi) =
        match idx_interval(func, nest, dom, t.store_block, t.store_idx, t.idx_c, 0) {
            Some((lo, hi, _)) => (lo, hi),
            None => return false,
        };
    ahi < clo || chi < alo
}

fn fill_trip(func: &Function, lp: &NaturalLoop) -> Option<i128> {
    let (iv, bound_local, _, then_bb) = header_cmp(func, lp.header)?;
    let n = func.blocks.len();
    let mut then_preds: Vec<BlockId> = Vec::new();
    for (i, block) in func.blocks.iter().enumerate() {
        if i >= n {
            continue;
        }
        let mut tgts: Vec<BlockId> = Vec::new();
        match &block.term {
            Terminator::Br(t) => tgts.push(*t),
            Terminator::BrIf { then_bb: t, else_bb: e, .. } => {
                tgts.push(*t);
                tgts.push(*e);
            }
            _ => {}
        }
        if tgts.contains(&then_bb) {
            then_preds.push(i);
        }
    }
    if then_preds != vec![lp.header] {
        return None;
    }
    let bound = const_eval(func, bound_local)? as i128;
    if bound <= 0 {
        return None;
    }
    let in_loop: Vec<(BlockId, usize)> = all_defs(func, iv)
        .into_iter()
        .filter(|(bi, _)| lp.blocks.contains(bi))
        .collect();
    if in_loop.len() != 1 || in_loop[0].1 == usize::MAX {
        return None;
    }
    let (cb, ci) = in_loop[0];
    let step_ok = match func.blocks.get(cb)?.instrs.get(ci)? {
        Instr::Copy { dst, src, .. } if *dst == iv => match single_def(func, *src) {
            Some((ab, ai)) => match func.blocks.get(ab)?.instrs.get(ai)? {
                Instr::Arith { op: ArithOp::Add, kind: NumKind::Int, dst, lhs, rhs, .. }
                    if *dst == *src && (*lhs == iv || *rhs == iv) =>
                {
                    const_eval(func, if *lhs == iv { *rhs } else { *lhs }) == Some(1)
                }
                _ => false,
            },
            None => false,
        },
        Instr::Arith { op: ArithOp::Add, kind: NumKind::Int, dst, lhs, rhs, .. }
            if *dst == iv && (*lhs == iv || *rhs == iv) =>
        {
            const_eval(func, if *lhs == iv { *rhs } else { *lhs }) == Some(1)
        }
        _ => false,
    };
    if !step_ok {
        return None;
    }
    let outside: Vec<(BlockId, usize)> = all_defs(func, iv)
        .into_iter()
        .filter(|(bi, _)| !lp.blocks.contains(bi))
        .collect();
    if outside.len() != 1 || outside[0].1 == usize::MAX {
        return None;
    }
    let (obi, oii) = outside[0];
    let init = match func.blocks.get(obi)?.instrs.get(oii)? {
        Instr::Const { lit: Lit::Int(c), .. } => *c as i128,
        Instr::Copy { src, .. } => const_eval(func, *src)? as i128,
        _ => return None,
    };
    let trip = bound.checked_sub(init)?;
    if trip <= 0 {
        return None;
    }
    Some(trip)
}

fn zero_proven(func: &Function, dom: &DominatorTree, m: &NestMatch) -> bool {
    let t = &m.triple;
    let fc = array_aliases(func, t.arr_c);
    let mut defs: Vec<(BlockId, usize)> = Vec::new();
    for local in fc.iter() {
        for d in all_defs(func, *local) {
            if d.1 == usize::MAX {
                return false;
            }
            defs.push(d);
        }
    }
    defs.sort();
    defs.dedup();
    if defs.len() > MAX_DEF_WALK {
        return false;
    }
    let mut new_sites: Vec<(BlockId, usize)> = Vec::new();
    for (db, di) in &defs {
        if matches!(func.blocks.get(*db).and_then(|b| b.instrs.get(*di)), Some(Instr::ArrayNew { .. }))
        {
            new_sites.push((*db, *di));
        }
    }
    if new_sites.len() != 1 {
        return false;
    }
    let (new_b, _) = new_sites[0];
    let nest_refs: Vec<&CountedLoop> = vec![&m.x.counted, &m.y.counted, &m.z.counted];
    let (clo, chi) =
        match idx_interval(func, &nest_refs, dom, t.store_block, t.store_idx, t.idx_c, 0) {
            Some((lo, hi, _)) => (lo, hi),
            None => return false,
        };
    if clo != 0 {
        return false;
    }
    let need: i128 = match chi.checked_add(1) {
        Some(v) if v > 0 => v,
        _ => return false,
    };
    let union_blocks: BTreeSet<BlockId> = m.x.blocks.iter().copied().collect();
    let mut loops = find_natural_loops(func, dom);
    loops.sort_by_key(|l| l.header);
    for lp in &loops {
        if !lp.blocks.is_disjoint(&union_blocks) {
            continue;
        }
        let trip = match fill_trip(func, lp) {
            Some(v) => v,
            None => continue,
        };
        if loop_latches(func, &lp.blocks, lp.header).len() != 1 {
            continue;
        }
        let headers: BTreeSet<BlockId> = BTreeSet::from([lp.header]);
        if nest_has_exits(func, &lp.blocks, &headers) {
            continue;
        }
        if external_preds(func, lp.header, &lp.blocks).len() != 1 {
            continue;
        }
        let mut straight = true;
        for b in &lp.blocks {
            if *b == lp.header {
                continue;
            }
            if !matches!(func.blocks.get(*b).map(|x| &x.term), Some(Terminator::Br(_))) {
                straight = false;
                break;
            }
        }
        if !straight {
            continue;
        }
        let mut pushes = 0i128;
        let mut ok = true;
        for b in &lp.blocks {
            let count = func.blocks.get(*b).map(|x| x.instrs.len()).unwrap_or(0);
            for ii in 0..count {
                match func.blocks.get(*b).and_then(|x| x.instrs.get(ii)) {
                    Some(Instr::ArrayPush { arr, value, .. }) if fc.contains(arr) => {
                        if const_eval(func, *value) != Some(0) {
                            ok = false;
                            break;
                        }
                        pushes += 1;
                    }
                    Some(Instr::ArraySet { arr, .. }) | Some(Instr::ArrayPop { arr, .. })
                        if fc.contains(arr) =>
                    {
                        ok = false;
                        break;
                    }
                    _ => {}
                }
            }
            if !ok {
                break;
            }
        }
        if !ok || pushes < 1 {
            continue;
        }
        let covered = match trip.checked_mul(pushes) {
            Some(v) => v,
            None => continue,
        };
        if covered != need {
            continue;
        }
        let exit_bb = match exit_of(func, lp.header) {
            Some(v) => v,
            None => continue,
        };
        if !dom.dominates(exit_bb, m.x.header) {
            continue;
        }
        if !dom.dominates(new_b, lp.header) {
            continue;
        }
        let mut foreign = false;
        for (bi, block) in func.blocks.iter().enumerate() {
            for (ii, ins) in block.instrs.iter().enumerate() {
                let hits = match ins {
                    Instr::ArraySet { arr, .. }
                    | Instr::ArrayPush { arr, .. }
                    | Instr::ArrayPop { arr, .. } => fc.contains(arr),
                    _ => false,
                };
                if hits && !lp.blocks.contains(&bi) && (bi, ii) != (t.store_block, t.store_idx) {
                    foreign = true;
                    break;
                }
            }
            if foreign {
                break;
            }
        }
        if foreign {
            continue;
        }
        return true;
    }
    false
}

fn cost_gate(m: &NestMatch) -> bool {
    let t = &m.triple;
    if t.form_b.coeffs.get(&m.y.iv) != Some(&1) {
        return false;
    }
    if t.form_c.coeffs.get(&m.y.iv) != Some(&1) {
        return false;
    }
    let z = m.z.iv;
    let strided = [&t.form_a, &t.form_b, &t.form_c]
        .iter()
        .any(|f| f.coeffs.get(&z).map(|c| c.abs() > 1).unwrap_or(false));
    if !strided {
        return false;
    }
    if m.y.bound_const - m.y.init_const < MIN_ACCUM_TRIP {
        return false;
    }
    true
}

pub fn match_fires(func: &Function) -> bool {
    matches_plan(func).is_some()
}

fn matches_plan(func: &Function) -> Option<(NestMatch, bool)> {
    let dom = compute_dominators(func);
    let m = find_candidate(func, &dom)?;
    if !alias_safe(func, &dom, &[&m.x.counted, &m.y.counted, &m.z.counted], &m) {
        return None;
    }
    if !cost_gate(&m) {
        return None;
    }
    if zero_proven(func, &dom, &m) {
        return Some((m, true));
    }
    if m.z.bound_const - m.z.init_const >= 1 {
        return Some((m, false));
    }
    None
}

pub fn accumulate_interchange(func: &mut Function) -> (AccumAction, AccumTimes) {
    let (action, times, _, _) = accumulate_interchange_tracked(func);
    (action, times)
}

pub(crate) fn accumulate_interchange_tracked(
    func: &mut Function,
) -> (
    AccumAction,
    AccumTimes,
    Option<Span>,
    Option<(crate::perflint::MissedReason, Span, i64, Option<Span>)>,
) {
    let mut times = AccumTimes::default();
    let start = stamp();
    let dom = compute_dominators(func);
    let mut located: Vec<[LoopFacts; 3]> = Vec::new();
    let found = find_candidate_locating(func, &dom, Some(&mut located));
    times.shape = start.elapsed();
    let m = match found {
        Some(v) => v,
        None => {
            let missed = crate::perflint::subscript_missed(func, &dom, &located);
            return (AccumAction::Untouched, times, None, missed);
        }
    };
    let start = stamp();
    let nest_refs = [&m.x.counted, &m.y.counted, &m.z.counted];
    let alias_ok = alias_safe(func, &dom, &nest_refs, &m);
    times.alias = start.elapsed();
    if !alias_ok {
        let missed = Some((
            crate::perflint::MissedReason::Alias,
            m.triple.store_span,
            inner_trip(&m),
            None,
        ));
        return (AccumAction::Untouched, times, None, missed);
    }
    if !cost_gate(&m) {
        let missed = Some((
            crate::perflint::MissedReason::CostGate,
            m.triple.store_span,
            inner_trip(&m),
            None,
        ));
        return (AccumAction::Untouched, times, None, missed);
    }
    let start = stamp();
    let zero_ok = zero_proven(func, &dom, &m);
    times.zero = start.elapsed();
    let start = stamp();
    let action = if zero_ok {
        let mut work = func.clone();
        let mut moved: Vec<Local> = Vec::new();
        if apply_v1(&mut work, &m, &mut moved) && check_v1(&work, &m, &moved) {
            *func = work;
            AccumAction::V1
        } else {
            AccumAction::Untouched
        }
    } else if m.z.bound_const - m.z.init_const >= 1 {
        let mut work = func.clone();
        let mut moved: Vec<Local> = Vec::new();
        let m2 = apply_peel(&mut work, &m, &mut moved);
        match m2 {
            Some(ref rem) if check_peel(&work, &m, rem, &moved) => {
                *func = work;
                AccumAction::Peel
            }
            _ => AccumAction::Untouched,
        }
    } else {
        AccumAction::Untouched
    };
    times.rewrite = start.elapsed();
    let span = match action {
        AccumAction::V1 | AccumAction::Peel => Some(m.triple.store_span),
        AccumAction::Untouched => None,
    };
    let missed = match action {
        AccumAction::V1 => None,
        AccumAction::Peel => Some((
            crate::perflint::MissedReason::ZeroInit,
            m.triple.store_span,
            inner_trip(&m),
            zero_alloc_span(func, &m),
        )),
        AccumAction::Untouched
            if !zero_ok && m.z.bound_const - m.z.init_const < 1 =>
        {
            Some((
                crate::perflint::MissedReason::ZeroInit,
                m.triple.store_span,
                inner_trip(&m),
                zero_alloc_span(func, &m),
            ))
        }
        AccumAction::Untouched => None,
    };
    (action, times, span, missed)
}

fn inner_trip(m: &NestMatch) -> i64 {
    m.z.bound_const - m.z.init_const
}

fn zero_alloc_span(func: &Function, m: &NestMatch) -> Option<Span> {
    let fc = array_aliases(func, m.triple.arr_c);
    let mut defs: Vec<(BlockId, usize)> = Vec::new();
    for local in fc.iter() {
        for d in all_defs(func, *local) {
            if d.1 == usize::MAX {
                return None;
            }
            defs.push(d);
        }
    }
    defs.sort();
    defs.dedup();
    if defs.len() > MAX_DEF_WALK {
        return None;
    }
    let mut found: Option<Span> = None;
    for (db, di) in &defs {
        match func.blocks.get(*db).and_then(|b| b.instrs.get(*di)) {
            Some(Instr::ArrayNew { span, .. }) => {
                if found.is_some() {
                    return None;
                }
                found = Some(*span);
            }
            _ => {}
        }
    }
    found
}

fn chain_in_block(
    func: &Function,
    local: Local,
    b: BlockId,
    ivs: &BTreeSet<Local>,
) -> Option<Vec<usize>> {
    let mut ordered: Vec<usize> = Vec::new();
    let mut stack = vec![local];
    let mut seen: BTreeSet<Local> = BTreeSet::new();
    let mut guard = 0usize;
    while let Some(cur) = stack.pop() {
        guard += 1;
        if guard > 64 {
            return None;
        }
        if ivs.contains(&cur) || !seen.insert(cur) {
            continue;
        }
        let defs = all_defs(func, cur);
        if defs.len() != 1 || defs[0].1 == usize::MAX {
            return None;
        }
        let (db, di) = defs[0];
        if db != b {
            continue;
        }
        match func.blocks.get(db)?.instrs.get(di)? {
            Instr::Const { .. } => {
                ordered.push(di);
            }
            Instr::Copy { src, .. } => {
                ordered.push(di);
                stack.push(*src);
            }
            Instr::Arith { lhs, rhs, .. } => {
                ordered.push(di);
                stack.push(*lhs);
                stack.push(*rhs);
            }
            _ => return None,
        }
    }
    ordered.sort();
    ordered.dedup();
    Some(ordered)
}

fn incr_sites(mv: &IncrMove) -> Vec<(BlockId, usize)> {
    let mut out = mv.main.clone();
    if let IncrConst::Move(b, i) = &mv.konst {
        out.push((*b, *i));
    }
    out
}

fn push_instr(work: &mut Function, at: BlockId, ins: Instr, moved: &mut Vec<Local>) {
    if let Some(dst) = instr_dst(&ins) {
        moved.push(dst);
    }
    if let Instr::Call { dsts, err, .. } = &ins {
        for d in dsts {
            moved.push(*d);
        }
        if let Some(e) = err {
            moved.push(*e);
        }
    }
    if let Some(block) = work.blocks.get_mut(at) {
        block.instrs.push(ins);
    }
}

fn emit_incr(
    taken: &BTreeMap<(BlockId, usize), Instr>,
    work: &mut Function,
    mv: &IncrMove,
    at: BlockId,
    moved: &mut Vec<Local>,
) -> bool {
    match &mv.konst {
        IncrConst::Move(kb, ki) => {
            let ins = match taken.get(&(*kb, *ki)).cloned() {
                Some(v) => v,
                None => return false,
            };
            if !matches!(ins, Instr::Const { .. }) {
                return false;
            }
            push_instr(work, at, ins, moved);
        }
        IncrConst::Fresh(v) => {
            let dst = work.locals.len() as Local;
            work.locals.push(LirType::I64);
            push_instr(
                work,
                at,
                Instr::Const { span: UNKNOWN_SPAN, dst, lit: Lit::Int(*v) },
                moved,
            );
        }
    }
    for site in mv.main.iter() {
        let ins = match taken.get(site).cloned() {
            Some(v) => v,
            None => return false,
        };
        match &ins {
            Instr::Arith { op: ArithOp::Add, kind: NumKind::Int, .. } | Instr::Copy { .. } => {}
            _ => return false,
        }
        push_instr(work, at, ins, moved);
    }
    true
}

fn rewrite_header_cmp(work: &mut Function, header: BlockId, iv: Local, bound: Local) -> bool {
    let block = match work.blocks.get_mut(header) {
        Some(b) => b,
        None => return false,
    };
    let (cond, then_bb, else_bb, span) = match &block.term {
        Terminator::BrIf { cond, then_bb, else_bb, span } => (*cond, *then_bb, *else_bb, *span),
        _ => return false,
    };
    let _ = (then_bb, else_bb, span);
    let mut done = false;
    for ins in block.instrs.iter_mut() {
        match ins {
            Instr::Cmp { op, kind: NumKind::Int, dst, lhs, rhs, .. } if *dst == cond => {
                *op = CmpOp::Lt;
                *lhs = iv;
                *rhs = bound;
                done = true;
            }
            _ => {}
        }
    }
    done
}

fn apply_v1(work: &mut Function, m: &NestMatch, moved: &mut Vec<Local>) -> bool {
    let t = &m.triple;
    let ivs: BTreeSet<Local> = BTreeSet::from([m.x.iv, m.y.iv, m.z.iv]);
    let zb = t.add_block;
    let chain_a = match chain_in_block(work, t.idx_a, zb, &ivs) {
        Some(v) => v,
        None => return false,
    };
    let chain_c = match chain_in_block(work, t.idx_c, t.store_block, &ivs) {
        Some(v) => v,
        None => return false,
    };
    let mut take: Vec<(BlockId, usize)> = Vec::new();
    take.extend(m.y.init_instrs.iter().copied());
    take.extend(m.z.init_instrs.iter().copied());
    take.extend(t.init_instrs.iter().copied());
    take.extend(chain_a.iter().map(|i| (zb, *i)));
    take.push((t.load_a_block, t.load_a_idx));
    take.push((t.add_block, t.add_idx));
    if let Some(cp) = t.update_copy {
        take.push(cp);
    }
    take.push((t.mul_block, t.mul_idx));
    take.extend(chain_c.iter().map(|i| (t.store_block, *i)));
    take.push((t.store_block, t.store_idx));
    take.extend(incr_sites(&m.triple.y_incr));
    take.extend(incr_sites(&m.triple.z_incr));
    take.sort();
    let raw_len = take.len();
    take.dedup();
    let want = take.len();
    if want != raw_len {
        return false;
    }
    let mut taken: BTreeMap<(BlockId, usize), Instr> = BTreeMap::new();
    {
        let mut by_block: BTreeMap<BlockId, Vec<usize>> = BTreeMap::new();
        for (b, i) in take {
            by_block.entry(b).or_default().push(i);
        }
        for (b, mut idxs) in by_block {
            idxs.sort();
            idxs.dedup();
            for i in idxs.into_iter().rev() {
                if let Some(block) = work.blocks.get_mut(b) {
                    if i < block.instrs.len() {
                        taken.insert((b, i), block.instrs.remove(i));
                    }
                }
            }
        }
    }
    if taken.len() != want {
        return false;
    }
    let get = |site: &(BlockId, usize)| taken.get(site).cloned();
    let mut y_init: Vec<Instr> = Vec::new();
    for s in m.y.init_instrs.iter() {
        match get(s) {
            Some(v) => y_init.push(v),
            None => return false,
        }
    }
    let mut z_init: Vec<Instr> = Vec::new();
    for s in m.z.init_instrs.iter() {
        match get(s) {
            Some(v) => z_init.push(v),
            None => return false,
        }
    }
    for s in t.init_instrs.iter() {
        if get(s).is_none() {
            return false;
        }
    }
    let mut chain_a_ins: Vec<Instr> = Vec::new();
    for i in chain_a.iter() {
        match get(&(zb, *i)) {
            Some(v) => chain_a_ins.push(v),
            None => return false,
        }
    }
    let load_a = match get(&(t.load_a_block, t.load_a_idx)) {
        Some(Instr::ArrayGet { span, dst, arr, index, elem_size, .. }) => {
            Instr::ArrayGet { span, dst, arr, index, elem_size, unchecked: false }
        }
        _ => return false,
    };
    let mut chain_c_ins: Vec<Instr> = Vec::new();
    for i in chain_c.iter() {
        match get(&(t.store_block, *i)) {
            Some(v) => chain_c_ins.push(v),
            None => return false,
        }
    }
    let (store_span, store_arr, store_idx_l, store_elem) =
        match get(&(t.store_block, t.store_idx)) {
            Some(Instr::ArraySet { span, arr, index, elem_size, .. }) => {
                (span, arr, index, elem_size)
            }
            _ => return false,
        };
    for s in m.triple.y_incr.main.iter().chain(m.triple.z_incr.main.iter()) {
        if get(s).is_none() {
            return false;
        }
    }
    if let IncrConst::Move(kb, ki) = &m.triple.y_incr.konst {
        if get(&(*kb, *ki)).is_none() {
            return false;
        }
    }
    if let IncrConst::Move(kb, ki) = &m.triple.z_incr.konst {
        if get(&(*kb, *ki)).is_none() {
            return false;
        }
    }
    let cv = work.locals.len() as Local;
    work.locals.push(LirType::I64);
    let pr = work.locals.len() as Local;
    work.locals.push(LirType::I64);
    let sm = work.locals.len() as Local;
    work.locals.push(LirType::I64);
    moved.push(cv);
    moved.push(pr);
    moved.push(sm);
    let ypre = m.y.pre;
    for ins in z_init {
        push_instr(work, ypre, ins, moved);
    }
    let zpre = m.z.pre;
    for ins in y_init {
        push_instr(work, zpre, ins, moved);
    }
    for ins in chain_a_ins {
        push_instr(work, zpre, ins, moved);
    }
    push_instr(work, zpre, load_a, moved);
    if !rewrite_header_cmp(work, m.y.header, m.z.iv, m.z.bound_local) {
        return false;
    }
    if !rewrite_header_cmp(work, m.z.header, m.y.iv, m.y.bound_local) {
        return false;
    }
    let cload = Instr::ArrayGet {
        span: store_span,
        dst: cv,
        arr: store_arr,
        index: store_idx_l,
        elem_size: store_elem,
        unchecked: false,
    };
    let mul = Instr::Arith {
        span: t.mul_span,
        op: ArithOp::Mul,
        kind: NumKind::Int,
        dst: pr,
        lhs: t.avec,
        rhs: t.load_b_dst,
    };
    let add = Instr::Arith {
        span: store_span,
        op: ArithOp::Add,
        kind: NumKind::Int,
        dst: sm,
        lhs: cv,
        rhs: pr,
    };
    let set = Instr::ArraySet {
        span: store_span,
        arr: store_arr,
        index: store_idx_l,
        value: sm,
        elem_size: store_elem,
        unchecked: false,
    };
    for ins in chain_c_ins {
        push_instr(work, zb, ins, moved);
    }
    push_instr(work, zb, cload, moved);
    push_instr(work, zb, mul, moved);
    push_instr(work, zb, add, moved);
    push_instr(work, zb, set, moved);
    if !emit_incr(&taken, work, &m.triple.y_incr, zb, moved) {
        return false;
    }
    if !emit_incr(&taken, work, &m.triple.z_incr, m.y.latch, moved) {
        return false;
    }
    let body_len = work.blocks.get(zb).map(|b| b.instrs.len()).unwrap_or(usize::MAX);
    if body_len > MAX_REWRITE_BODY_INSTRS {
        return false;
    }
    moved.retain(|l| *l != m.x.iv && *l != m.y.iv && *l != m.z.iv);
    true
}

fn exact_pairs(func: &Function) -> BTreeSet<(BlockId, Local)> {
    crate::bce::exact_count_loops(func).iter().map(|l| (l.header, l.iv)).collect()
}

fn check_moved_defs(work: &Function, moved: &[Local]) -> bool {
    let dom = compute_dominators(work);
    let uses = use_sites(work);
    let mut seen: BTreeSet<Local> = BTreeSet::new();
    for d in moved {
        if !seen.insert(*d) {
            continue;
        }
        let def = match single_def(work, *d) {
            Some(v) => v,
            None => return false,
        };
        for (ub, ui) in uses.get(d).cloned().unwrap_or_default() {
            if ui == usize::MAX {
                return false;
            }
            if ub == def.0 {
                if ui < def.1 {
                    return false;
                }
            } else if !dom.dominates(def.0, ub) {
                return false;
            }
        }
    }
    true
}

fn check_v1(work: &Function, m: &NestMatch, moved: &[Local]) -> bool {
    if !check_moved_defs(work, moved) {
        return false;
    }
    let pairs = exact_pairs(work);
    if !pairs.contains(&(m.x.header, m.x.iv)) {
        return false;
    }
    if !pairs.contains(&(m.y.header, m.z.iv)) {
        return false;
    }
    if !pairs.contains(&(m.z.header, m.y.iv)) {
        return false;
    }
    let facts = crate::loopdep::analyze_function(work);
    facts.iter().any(|f| f.ivs.len() == 3 && f.ivs == vec![m.x.iv, m.z.iv, m.y.iv])
}

fn remap_local(map: &BTreeMap<Local, Local>, l: Local) -> Local {
    map.get(&l).copied().unwrap_or(l)
}

fn remap_opt(map: &BTreeMap<Local, Local>, l: Option<Local>) -> Option<Local> {
    l.map(|v| remap_local(map, v))
}

fn remap_vec(map: &BTreeMap<Local, Local>, v: &[Local]) -> Vec<Local> {
    v.iter().map(|l| remap_local(map, *l)).collect()
}

fn remap_instr(ins: &Instr, map: &BTreeMap<Local, Local>) -> Instr {
    let r = |l: Local| remap_local(map, l);
    let rv = |v: &[Local]| remap_vec(map, v);
    match ins {
        Instr::Const { span, dst, lit } => Instr::Const { span: *span, dst: r(*dst), lit: lit.clone() },
        Instr::Copy { span, dst, src } => Instr::Copy { span: *span, dst: r(*dst), src: r(*src) },
        Instr::Cast { span, dst, src } => Instr::Cast { span: *span, dst: r(*dst), src: r(*src) },
        Instr::Convert { span, dst, src, kind } => {
            Instr::Convert { span: *span, dst: r(*dst), src: r(*src), kind: *kind }
        }
        Instr::Arith { span, op, kind, dst, lhs, rhs } => {
            Instr::Arith { span: *span, op: *op, kind: *kind, dst: r(*dst), lhs: r(*lhs), rhs: r(*rhs) }
        }
        Instr::Fma { span, dst, a, b, c } => {
            Instr::Fma { span: *span, dst: r(*dst), a: r(*a), b: r(*b), c: r(*c) }
        }
        Instr::Cmp { span, op, kind, dst, lhs, rhs } => {
            Instr::Cmp { span: *span, op: *op, kind: *kind, dst: r(*dst), lhs: r(*lhs), rhs: r(*rhs) }
        }
        Instr::Not { span, dst, src } => Instr::Not { span: *span, dst: r(*dst), src: r(*src) },
        Instr::Neg { span, kind, dst, src } => {
            Instr::Neg { span: *span, kind: *kind, dst: r(*dst), src: r(*src) }
        }
        Instr::Concat { span, dst, lhs, rhs } => {
            Instr::Concat { span: *span, dst: r(*dst), lhs: r(*lhs), rhs: r(*rhs) }
        }
        Instr::ToStr { span, dst, src } => Instr::ToStr { span: *span, dst: r(*dst), src: r(*src) },
        Instr::Range { span, dst, lo, hi, inclusive } => {
            Instr::Range { span: *span, dst: r(*dst), lo: r(*lo), hi: r(*hi), inclusive: *inclusive }
        }
        Instr::Stride { span, dst, range, step } => {
            Instr::Stride { span: *span, dst: r(*dst), range: r(*range), step: r(*step) }
        }
        Instr::ArrayNew { span, dst, cap, elem_size } => {
            Instr::ArrayNew { span: *span, dst: r(*dst), cap: *cap, elem_size: *elem_size }
        }
        Instr::ArrayPush { span, arr, value, elem_size } => {
            Instr::ArrayPush { span: *span, arr: r(*arr), value: r(*value), elem_size: *elem_size }
        }
        Instr::ArrayPop { span, dst, arr } => {
            Instr::ArrayPop { span: *span, dst: r(*dst), arr: r(*arr) }
        }
        Instr::ArrayLen { span, dst, arr } => {
            Instr::ArrayLen { span: *span, dst: r(*dst), arr: r(*arr) }
        }
        Instr::ArrayGet { span, dst, arr, index, elem_size, unchecked } => {
            Instr::ArrayGet { span: *span, dst: r(*dst), arr: r(*arr), index: r(*index), elem_size: *elem_size, unchecked: *unchecked }
        }
        Instr::ArraySet { span, arr, index, value, elem_size, unchecked } => {
            Instr::ArraySet { span: *span, arr: r(*arr), index: r(*index), value: r(*value), elem_size: *elem_size, unchecked: *unchecked }
        }
        Instr::ObjNew { span, dst, class, instance_size } => {
            Instr::ObjNew { span: *span, dst: r(*dst), class: *class, instance_size: *instance_size }
        }
        Instr::StackAlloc { span, dst, class, instance_size } => {
            Instr::StackAlloc { span: *span, dst: r(*dst), class: *class, instance_size: *instance_size }
        }
        Instr::EnumNew { span, dst, enu, variant, payload } => {
            Instr::EnumNew { span: *span, dst: r(*dst), enu: *enu, variant: *variant, payload: rv(payload) }
        }
        Instr::EnumPayload { span, dst, scrut, index } => {
            Instr::EnumPayload { span: *span, dst: r(*dst), scrut: r(*scrut), index: *index }
        }
        Instr::EnumTag { span, dst, scrut } => {
            Instr::EnumTag { span: *span, dst: r(*dst), scrut: r(*scrut) }
        }
        Instr::Extract { span, dst, base, index, field } => {
            Instr::Extract { span: *span, dst: r(*dst), base: r(*base), index: *index, field: field.clone() }
        }
        Instr::AddrOf { span, dst, src } => Instr::AddrOf { span: *span, dst: r(*dst), src: r(*src) },
        Instr::PtrLoad { span, dst, ptr, volatile } => {
            Instr::PtrLoad { span: *span, dst: r(*dst), ptr: r(*ptr), volatile: *volatile }
        }
        Instr::PtrStore { span, ptr, val, volatile } => {
            Instr::PtrStore { span: *span, ptr: r(*ptr), val: r(*val), volatile: *volatile }
        }
        Instr::RangeLo { span, dst, range } => {
            Instr::RangeLo { span: *span, dst: r(*dst), range: r(*range) }
        }
        Instr::RangeHi { span, dst, range } => {
            Instr::RangeHi { span: *span, dst: r(*dst), range: r(*range) }
        }
        Instr::RangeStep { span, dst, range } => {
            Instr::RangeStep { span: *span, dst: r(*dst), range: r(*range) }
        }
        Instr::GetField { span, dst, obj, field } => {
            Instr::GetField { span: *span, dst: r(*dst), obj: r(*obj), field: *field }
        }
        Instr::GetFieldByName { span, dst, obj, field } => {
            Instr::GetFieldByName { span: *span, dst: r(*dst), obj: r(*obj), field: field.clone() }
        }
        Instr::SetField { span, obj, field, value } => {
            Instr::SetField { span: *span, obj: r(*obj), field: *field, value: r(*value) }
        }
        Instr::SetFieldByName { span, obj, field, value } => {
            Instr::SetFieldByName { span: *span, obj: r(*obj), field: field.clone(), value: r(*value) }
        }
        Instr::ClosureNew { span, dst, func, captures, decay, decay_this } => {
            Instr::ClosureNew { span: *span, dst: r(*dst), func: *func, captures: rv(captures), decay: *decay, decay_this: *decay_this }
        }
        Instr::Call { span, dsts, err, target, args } => {
            let nt = match target {
                CallTarget::Dyn { obj, method } => CallTarget::Dyn { obj: r(*obj), method: method.clone() },
                CallTarget::Value(l) => CallTarget::Value(r(*l)),
                other => other.clone(),
            };
            Instr::Call { span: *span, dsts: rv(dsts), err: remap_opt(map, *err), target: nt, args: rv(args) }
        }
        Instr::GenRefOf { span, dst, obj } => {
            Instr::GenRefOf { span: *span, dst: r(*dst), obj: r(*obj) }
        }
        Instr::GenRefEmpty { span, dst } => Instr::GenRefEmpty { span: *span, dst: r(*dst) },
        Instr::GenRefGet { span, dst, gref } => {
            Instr::GenRefGet { span: *span, dst: r(*dst), gref: r(*gref) }
        }
        Instr::GenRefInvalidate { span, obj } => {
            Instr::GenRefInvalidate { span: *span, obj: r(*obj) }
        }
        Instr::ThreadSpawn { span, dst, func, closure, ret_tag } => {
            Instr::ThreadSpawn { span: *span, dst: r(*dst), func: *func, closure: remap_opt(map, *closure), ret_tag: *ret_tag }
        }
        Instr::ThreadJoin { span, dst, handle } => {
            Instr::ThreadJoin { span: *span, dst: r(*dst), handle: r(*handle) }
        }
        Instr::PoolInit { span, dst, id, workers } => {
            Instr::PoolInit { span: *span, dst: r(*dst), id: r(*id), workers: r(*workers) }
        }
        Instr::PoolSubmit { span, dst, pool, func, arg, closure, ret_tag } => {
            Instr::PoolSubmit { span: *span, dst: r(*dst), pool: r(*pool), func: *func, arg: remap_opt(map, *arg), closure: remap_opt(map, *closure), ret_tag: *ret_tag }
        }
        Instr::PoolParallelFor { span, pool, start, end, chunk, func, closure } => {
            Instr::PoolParallelFor { span: *span, pool: r(*pool), start: r(*start), end: r(*end), chunk: r(*chunk), func: *func, closure: remap_opt(map, *closure) }
        }
        Instr::PoolJoin { span, pool } => Instr::PoolJoin { span: *span, pool: r(*pool) },
        Instr::PoolShutdown { span, pool } => Instr::PoolShutdown { span: *span, pool: r(*pool) },
        Instr::VecNew { span, dst, kind, x, y, z, w } => {
            Instr::VecNew { span: *span, dst: r(*dst), kind: *kind, x: r(*x), y: r(*y), z: r(*z), w: r(*w) }
        }
        Instr::VecSplat { span, dst, kind, val } => {
            Instr::VecSplat { span: *span, dst: r(*dst), kind: *kind, val: r(*val) }
        }
        Instr::VecExtract { span, dst, vec, lane } => {
            Instr::VecExtract { span: *span, dst: r(*dst), vec: r(*vec), lane: r(*lane) }
        }
        Instr::VecInsert { span, dst, vec, lane, val } => {
            Instr::VecInsert { span: *span, dst: r(*dst), vec: r(*vec), lane: *lane, val: r(*val) }
        }
        Instr::VecArith { span, dst, op, kind, lhs, rhs } => {
            Instr::VecArith { span: *span, dst: r(*dst), op: *op, kind: *kind, lhs: r(*lhs), rhs: r(*rhs) }
        }
        Instr::VecUnary { span, dst, op, src } => {
            Instr::VecUnary { span: *span, dst: r(*dst), op: *op, src: r(*src) }
        }
        Instr::VecDot { span, dst, lhs, rhs } => {
            Instr::VecDot { span: *span, dst: r(*dst), lhs: r(*lhs), rhs: r(*rhs) }
        }
        Instr::ReleaseField { span, obj, field } => {
            Instr::ReleaseField { span: *span, obj: r(*obj), field: *field }
        }
        Instr::Retain { span, obj } => Instr::Retain { span: *span, obj: r(*obj) },
        Instr::Release { span, obj } => Instr::Release { span: *span, obj: r(*obj) },
        Instr::ReleaseAs { span, obj, class } => {
            Instr::ReleaseAs { span: *span, obj: r(*obj), class: *class }
        }
        Instr::Defer { span, body } => {
            Instr::Defer { span: *span, body: body.iter().map(|n| remap_instr(n, map)).collect() }
        }
        Instr::RunDefers { span, keep } => Instr::RunDefers { span: *span, keep: *keep },
        Instr::Assert { span, cond, message } => {
            Instr::Assert { span: *span, cond: r(*cond), message: r(*message) }
        }
        Instr::Panic { span, message } => Instr::Panic { span: *span, message: r(*message) },
    }
}

fn collect_defs(ins: &Instr, out: &mut BTreeSet<Local>) {
    if let Some(dst) = instr_dst(ins) {
        out.insert(dst);
    }
    if let Instr::Call { dsts, err, .. } = ins {
        for d in dsts {
            out.insert(*d);
        }
        if let Some(e) = err {
            out.insert(*e);
        }
    }
    if let Instr::Defer { body, .. } = ins {
        for nested in body {
            collect_defs(nested, out);
        }
    }
}

fn remap_loopfacts(
    lf: &LoopFacts,
    bmap: &BTreeMap<BlockId, BlockId>,
    lmap: &BTreeMap<Local, Local>,
) -> Option<LoopFacts> {
    let rb = |b: BlockId| bmap.get(&b).copied();
    let mut init_instrs: Vec<(BlockId, usize)> = Vec::new();
    for (b, i) in lf.init_instrs.iter() {
        init_instrs.push((rb(*b).unwrap_or(*b), *i));
    }
    Some(LoopFacts {
        header: rb(lf.header)?,
        then_bb: rb(lf.then_bb)?,
        exit_bb: rb(lf.exit_bb).unwrap_or(lf.exit_bb),
        latch: rb(lf.latch)?,
        iv: remap_local(lmap, lf.iv),
        cond: remap_local(lmap, lf.cond),
        bound_local: remap_local(lmap, lf.bound_local),
        init_const: lf.init_const,
        bound_const: lf.bound_const,
        pre: rb(lf.pre).unwrap_or(lf.pre),
        init_instrs,
        blocks: lf.blocks.iter().filter_map(|b| rb(*b)).collect(),
        counted: lf.counted.clone(),
    })
}

fn remap_incr(
    mv: &IncrMove,
    bmap: &BTreeMap<BlockId, BlockId>,
    lmap: &BTreeMap<Local, Local>,
) -> Option<IncrMove> {
    let _ = lmap;
    let mut main: Vec<(BlockId, usize)> = Vec::new();
    for (b, i) in mv.main.iter() {
        main.push((bmap.get(b).copied()?, *i));
    }
    let konst = match &mv.konst {
        IncrConst::Move(b, i) => IncrConst::Move(bmap.get(b).copied()?, *i),
        IncrConst::Fresh(v) => IncrConst::Fresh(*v),
    };
    Some(IncrMove { main, konst })
}

fn remap_match(
    m: &NestMatch,
    bmap: &BTreeMap<BlockId, BlockId>,
    lmap: &BTreeMap<Local, Local>,
) -> Option<NestMatch> {
    let rb = |b: BlockId| bmap.get(&b).copied();
    let rl = |l: Local| remap_local(lmap, l);
    let t = &m.triple;
    let mut form_a = t.form_a.clone();
    let mut form_b = t.form_b.clone();
    let mut form_c = t.form_c.clone();
    for form in [&mut form_a, &mut form_b, &mut form_c] {
        let keys: Vec<Local> = form.coeffs.keys().copied().collect();
        let mut next: BTreeMap<Local, i128> = BTreeMap::new();
        for k in keys {
            next.insert(rl(k), form.coeffs[&k]);
        }
        form.coeffs = next;
    }
    let mut init_instrs: Vec<(BlockId, usize)> = Vec::new();
    for (b, i) in t.init_instrs.iter() {
        init_instrs.push((rb(*b)?, *i));
    }
    let triple = Triple {
        s: rl(t.s),
        init_instrs,
        add_block: rb(t.add_block)?,
        add_idx: t.add_idx,
        update_copy: t.update_copy.map(|(b, i)| (rb(b).unwrap_or(usize::MAX), i)),
        m: rl(t.m),
        mul_block: rb(t.mul_block)?,
        mul_idx: t.mul_idx,
        mul_span: t.mul_span,
        load_a_block: rb(t.load_a_block)?,
        load_a_idx: t.load_a_idx,
        arr_a: rl(t.arr_a),
        idx_a: rl(t.idx_a),
        avec: rl(t.avec),
        load_b_block: rb(t.load_b_block)?,
        load_b_idx: t.load_b_idx,
        arr_b: rl(t.arr_b),
        idx_b: rl(t.idx_b),
        load_b_dst: rl(t.load_b_dst),
        store_block: rb(t.store_block)?,
        store_idx: t.store_idx,
        store_span: t.store_span,
        store_elem: t.store_elem,
        arr_c: rl(t.arr_c),
        idx_c: rl(t.idx_c),
        form_a,
        form_b,
        form_c,
        y_incr: remap_incr(&t.y_incr, bmap, lmap)?,
        z_incr: remap_incr(&t.z_incr, bmap, lmap)?,
    };
    if triple.update_copy.map(|(b, _)| b).unwrap_or(0) == usize::MAX {
        return None;
    }
    Some(NestMatch {
        x: remap_loopfacts(&m.x, bmap, lmap)?,
        y: remap_loopfacts(&m.y, bmap, lmap)?,
        z: remap_loopfacts(&m.z, bmap, lmap)?,
        triple,
    })
}

fn duplicate_region(
    work: &mut Function,
    region: &BTreeSet<BlockId>,
) -> Option<(BTreeMap<BlockId, BlockId>, BTreeMap<Local, Local>)> {
    for b in region {
        if work.blocks.get(*b).is_none() {
            return None;
        }
    }
    let mut defs: BTreeSet<Local> = BTreeSet::new();
    for b in region {
        for ins in &work.blocks[*b].instrs {
            collect_defs(ins, &mut defs);
        }
        if let Terminator::BrErr { catch_bind, .. } = work.blocks[*b].term {
            defs.insert(catch_bind);
        }
    }
    let mut lmap: BTreeMap<Local, Local> = BTreeMap::new();
    for d in defs {
        let ty = work.locals.get(d as usize).cloned().unwrap_or(LirType::Any);
        let f = work.locals.len() as Local;
        work.locals.push(ty);
        lmap.insert(d, f);
    }
    let ordered: Vec<BlockId> = region.iter().copied().collect();
    let base = work.blocks.len();
    let mut bmap: BTreeMap<BlockId, BlockId> = BTreeMap::new();
    for (k, b) in ordered.iter().enumerate() {
        bmap.insert(*b, base + k);
    }
    for b in ordered {
        let src_instrs = work.blocks[b].instrs.clone();
        let src_term = work.blocks[b].term.clone();
        let instrs: Vec<Instr> = src_instrs.iter().map(|i| remap_instr(i, &lmap)).collect();
        let term = remap_term(&src_term, &bmap, &lmap);
        work.blocks.push(Block { instrs, term });
    }
    Some((bmap, lmap))
}

fn remap_term(
    term: &Terminator,
    bmap: &BTreeMap<BlockId, BlockId>,
    lmap: &BTreeMap<Local, Local>,
) -> Terminator {
    let b = |id: BlockId| bmap.get(&id).copied().unwrap_or(id);
    let l = |v: Local| remap_local(lmap, v);
    match term {
        Terminator::Ret(v) => Terminator::Ret(v.iter().map(|x| l(*x)).collect()),
        Terminator::Br(t) => Terminator::Br(b(*t)),
        Terminator::BrIf { span, cond, then_bb, else_bb } => Terminator::BrIf {
            span: *span,
            cond: l(*cond),
            then_bb: b(*then_bb),
            else_bb: b(*else_bb),
        },
        Terminator::BrErr { span, err, catch_bb, catch_bind, next_bb, depth } => {
            Terminator::BrErr {
                span: *span,
                err: l(*err),
                catch_bb: b(*catch_bb),
                catch_bind: l(*catch_bind),
                next_bb: b(*next_bb),
                depth: *depth,
            }
        }
        Terminator::Switch { span, scrut, cases, default } => Terminator::Switch {
            span: *span,
            scrut: l(*scrut),
            cases: cases
                .iter()
                .map(|(p, t)| {
                    let np = match p {
                        SwitchPat::Is { tag, source, check } => SwitchPat::Is {
                            tag: tag.clone(),
                            source: l(*source),
                            check: *check,
                        },
                        other => other.clone(),
                    };
                    (np, b(*t))
                })
                .collect(),
            default: b(*default),
        },
        Terminator::Throw { src, catch, span } => Terminator::Throw {
            span: *span,
            src: l(*src),
            catch: catch.map(|(t, bind, d)| (b(t), l(bind), d)),
        },
        Terminator::Rethrow { catch_bb, err, depth, span } => {
            Terminator::Rethrow { span: *span, catch_bb: b(*catch_bb), err: l(*err), depth: *depth }
        }
        Terminator::Unreachable { span } => Terminator::Unreachable { span: *span },
    }
}

fn apply_slab(work: &mut Function, m: &NestMatch, moved: &mut Vec<Local>) -> bool {
    let t = &m.triple;
    let ivs: BTreeSet<Local> = BTreeSet::from([m.x.iv, m.y.iv, m.z.iv]);
    let zb = t.add_block;
    let chain_c = match chain_in_block(work, t.idx_c, t.store_block, &ivs) {
        Some(v) => v,
        None => return false,
    };
    let mut take: Vec<(BlockId, usize)> = Vec::new();
    take.extend(t.init_instrs.iter().copied());
    take.push((t.add_block, t.add_idx));
    if let Some(cp) = t.update_copy {
        take.push(cp);
    }
    take.extend(chain_c.iter().map(|i| (t.store_block, *i)));
    take.push((t.store_block, t.store_idx));
    take.extend(incr_sites(&m.triple.y_incr));
    take.extend(incr_sites(&m.triple.z_incr));
    take.sort();
    let raw_len = take.len();
    take.dedup();
    let want = take.len();
    let mut taken: BTreeMap<(BlockId, usize), Instr> = BTreeMap::new();
    {
        let mut by_block: BTreeMap<BlockId, Vec<usize>> = BTreeMap::new();
        for (b, i) in take {
            by_block.entry(b).or_default().push(i);
        }
        for (b, mut idxs) in by_block {
            idxs.sort();
            idxs.dedup();
            for i in idxs.into_iter().rev() {
                if let Some(block) = work.blocks.get_mut(b) {
                    if i < block.instrs.len() {
                        taken.insert((b, i), block.instrs.remove(i));
                    }
                }
            }
        }
    }
    if taken.len() != want || want != raw_len {
        return false;
    }
    let get = |site: &(BlockId, usize)| taken.get(site).cloned();
    for s in t.init_instrs.iter() {
        if get(s).is_none() {
            return false;
        }
    }
    let mut chain_c_ins: Vec<Instr> = Vec::new();
    for i in chain_c.iter() {
        match get(&(t.store_block, *i)) {
            Some(v) => chain_c_ins.push(v),
            None => return false,
        }
    }
    let (store_span, store_arr, store_idx_l, store_elem) =
        match get(&(t.store_block, t.store_idx)) {
            Some(Instr::ArraySet { span, arr, index, elem_size, .. }) => {
                (span, arr, index, elem_size)
            }
            _ => return false,
        };
    for s in m.triple.y_incr.main.iter() {
        if get(s).is_none() {
            return false;
        }
    }
    let set = Instr::ArraySet {
        span: store_span,
        arr: store_arr,
        index: store_idx_l,
        value: t.m,
        elem_size: store_elem,
        unchecked: false,
    };
    for ins in chain_c_ins {
        push_instr(work, zb, ins, moved);
    }
    push_instr(work, zb, set, moved);
    if !emit_incr(&taken, work, &m.triple.y_incr, zb, moved) {
        return false;
    }
    let zpre_term = work.blocks.get(m.z.pre).map(|b| b.term.clone());
    match zpre_term {
        Some(Terminator::Br(_)) => {
            if let Some(block) = work.blocks.get_mut(m.z.pre) {
                block.term = Terminator::Br(zb);
            }
        }
        _ => return false,
    }
    let zb_term = work.blocks.get(zb).map(|b| b.term.clone());
    match zb_term {
        Some(Terminator::Br(tgt)) if tgt == m.z.header => {
            if let Some(block) = work.blocks.get_mut(zb) {
                block.term = Terminator::Br(m.y.header);
            }
        }
        _ => return false,
    }
    if !kill_if_dead(work, &[m.z.header, m.y.latch]) {
        return false;
    }
    moved.retain(|l| *l != m.x.iv && *l != m.y.iv && *l != m.z.iv);
    true
}

fn reachable_from(func: &Function, entry: BlockId) -> BTreeSet<BlockId> {
    let mut seen: BTreeSet<BlockId> = BTreeSet::new();
    let mut stack = vec![entry];
    while let Some(b) = stack.pop() {
        if b >= func.blocks.len() || !seen.insert(b) {
            continue;
        }
        let mut tgts: Vec<BlockId> = Vec::new();
        match &func.blocks[b].term {
            Terminator::Ret(_) | Terminator::Unreachable { .. } => {}
            Terminator::Br(t) => tgts.push(*t),
            Terminator::BrIf { then_bb, else_bb, .. } => {
                tgts.push(*then_bb);
                tgts.push(*else_bb);
            }
            Terminator::BrErr { catch_bb, next_bb, .. } => {
                tgts.push(*catch_bb);
                tgts.push(*next_bb);
            }
            Terminator::Switch { cases, default, .. } => {
                for (_, t) in cases {
                    tgts.push(*t);
                }
                tgts.push(*default);
            }
            Terminator::Throw { catch, .. } => {
                if let Some((t, _, _)) = catch {
                    tgts.push(*t);
                }
            }
            Terminator::Rethrow { catch_bb, .. } => tgts.push(*catch_bb),
        }
        for t in tgts {
            stack.push(t);
        }
    }
    seen
}

fn kill_if_dead(work: &mut Function, blocks: &[BlockId]) -> bool {
    let live = reachable_from(work, 0);
    for b in blocks {
        if live.contains(b) {
            return false;
        }
        if let Some(block) = work.blocks.get_mut(*b) {
            block.term = Terminator::Unreachable { span: UNKNOWN_SPAN };
        }
    }
    true
}

fn apply_peel(work: &mut Function, m: &NestMatch, moved: &mut Vec<Local>) -> Option<NestMatch> {
    let region: BTreeSet<BlockId> = m.x.blocks.iter().copied().collect();
    let mut need: Vec<BlockId> = vec![
        m.x.header,
        m.y.header,
        m.y.pre,
        m.y.then_bb,
        m.y.latch,
        m.z.header,
        m.z.pre,
        m.z.then_bb,
        m.z.latch,
        m.triple.add_block,
        m.triple.store_block,
    ];
    need.sort();
    need.dedup();
    for b in need {
        if !region.contains(&b) {
            return None;
        }
    }
    let (bmap, lmap) = duplicate_region(work, &region)?;
    let rem_xh = bmap.get(&m.x.header).copied()?;
    let rx_init_tmp = work.locals.len() as Local;
    work.locals.push(LirType::I64);
    moved.push(rx_init_tmp);
    let rx_pre = work.blocks.len();
    work.blocks.push(Block {
        instrs: vec![
            Instr::Const { span: UNKNOWN_SPAN, dst: rx_init_tmp, lit: Lit::Int(m.x.init_const) },
            Instr::Copy { span: UNKNOWN_SPAN, dst: remap_local(&lmap, m.x.iv), src: rx_init_tmp },
        ],
        term: Terminator::Br(rem_xh),
    });
    match work.blocks.get(m.x.header).map(|b| b.term.clone()) {
        Some(Terminator::BrIf { span, cond, then_bb, else_bb }) => {
            let _ = else_bb;
            if let Some(block) = work.blocks.get_mut(m.x.header) {
                block.term =
                    Terminator::BrIf { span, cond, then_bb, else_bb: rx_pre };
            }
        }
        _ => return None,
    }
    if !apply_slab(work, m, moved) {
        return None;
    }
    let m2 = remap_match(m, &bmap, &lmap)?;
    let mut zinit_is_one = false;
    for (b, i) in m2.z.init_instrs.iter() {
        match work.blocks.get(*b).and_then(|x| x.instrs.get(*i)).cloned() {
            Some(Instr::Const { lit: Lit::Int(0), dst, span }) => {
                if let Some(block) = work.blocks.get_mut(*b) {
                    if let Some(ins) = block.instrs.get_mut(*i) {
                        *ins = Instr::Const { span, dst, lit: Lit::Int(1) };
                        zinit_is_one = true;
                    }
                }
            }
            Some(Instr::Copy { .. }) => {}
            _ => return None,
        }
    }
    if !zinit_is_one {
        return None;
    }
    if !apply_v1(work, &m2, moved) {
        return None;
    }
    Some(m2)
}

fn check_peel(work: &Function, m: &NestMatch, m2: &NestMatch, moved: &[Local]) -> bool {
    if !check_moved_defs(work, moved) {
        return false;
    }
    let pairs = exact_pairs(work);
    if !pairs.contains(&(m.x.header, m.x.iv)) {
        return false;
    }
    if !pairs.contains(&(m.y.header, m.y.iv)) {
        return false;
    }
    if !pairs.contains(&(m2.x.header, m2.x.iv)) {
        return false;
    }
    if !pairs.contains(&(m2.y.header, m2.z.iv)) {
        return false;
    }
    if !pairs.contains(&(m2.z.header, m2.y.iv)) {
        return false;
    }
    let facts = crate::loopdep::analyze_function(work);
    let has_slab = facts
        .iter()
        .any(|f| f.ivs.len() == 2 && f.ivs == vec![m.x.iv, m.y.iv]);
    if !has_slab {
        return false;
    }
    facts.iter().any(|f| {
        f.ivs.len() == 3 && f.ivs == vec![m2.x.iv, m2.z.iv, m2.y.iv]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lower_src(src: &str) -> Function {
        let mut m = frontend::parser::Parser::parse_module(src).expect("parse");
        assert!(frontend::desugar::desugar(&mut m).is_empty());
        let module = crate::lower::lower(&m).expect("lower");
        module.functions.iter().find(|f| f.name == "Main").expect("Main").clone()
    }

    fn matmul_src() -> String {
        std::fs::read_to_string(
            concat!(env!("CARGO_MANIFEST_DIR"), "/../../benches/compute/matmul/main.rnx"),
        )
        .expect("matmul bench")
    }

    const SMALL_ZERO: &str = "fn Main(): Int {\n    let n = 16;\n    let a: Array<Int> = [];\n    let b: Array<Int> = [];\n    let c: Array<Int> = [];\n    let i = 0;\n    while i < n * n {\n        a.push(0);\n        b.push(0);\n        c.push(0);\n        i = i + 1;\n    }\n    let r = 0;\n    while r < n {\n        let k = 0;\n        while k < n {\n            a[r * n + k] = (r + k) % 8;\n            b[r * n + k] = (r * k) % 8;\n            k = k + 1;\n        }\n        r = r + 1;\n    }\n    let x = 0;\n    while x < n {\n        let y = 0;\n        while y < n {\n            let s = 0;\n            let z = 0;\n            while z < n {\n                s = s + a[x * n + z] * b[z * n + y];\n                z = z + 1;\n            }\n            c[x * n + y] = s;\n            y = y + 1;\n        }\n        x = x + 1;\n    }\n    let total = 0;\n    let t = 0;\n    while t < n * n {\n        total = total + c[t];\n        t = t + 1;\n    }\n    return total;\n}\n";

    const SMALL_GARBAGE: &str = "fn Main(): Int {\n    let n = 16;\n    let a: Array<Int> = [];\n    let b: Array<Int> = [];\n    let c: Array<Int> = [];\n    let i = 0;\n    while i < n * n {\n        a.push(0);\n        b.push(0);\n        c.push(7);\n        i = i + 1;\n    }\n    let r = 0;\n    while r < n {\n        let k = 0;\n        while k < n {\n            a[r * n + k] = (r + k) % 8;\n            b[r * n + k] = (r * k) % 8;\n            k = k + 1;\n        }\n        r = r + 1;\n    }\n    let x = 0;\n    while x < n {\n        let y = 0;\n        while y < n {\n            let s = 0;\n            let z = 0;\n            while z < n {\n                s = s + a[x * n + z] * b[z * n + y];\n                z = z + 1;\n            }\n            c[x * n + y] = s;\n            y = y + 1;\n        }\n        x = x + 1;\n    }\n    let total = 0;\n    let t = 0;\n    while t < n * n {\n        total = total + c[t];\n        t = t + 1;\n    }\n    return total;\n}\n";

    const SMALL_ALIASED: &str = "fn Main(): Int {\n    let n = 16;\n    let a: Array<Int> = [];\n    let b: Array<Int> = [];\n    let c: Array<Int> = [];\n    let i = 0;\n    while i < n * n {\n        a.push(0);\n        b.push(0);\n        c.push(0);\n        i = i + 1;\n    }\n    let r = 0;\n    while r < n {\n        let k = 0;\n        while k < n {\n            a[r * n + k] = (r + k) % 8;\n            b[r * n + k] = (r * k) % 8;\n            k = k + 1;\n        }\n        r = r + 1;\n    }\n    let x = 0;\n    while x < n {\n        let y = 0;\n        while y < n {\n            let s = 0;\n            let z = 0;\n            while z < n {\n                s = s + a[x * n + z] * b[z * n + y];\n                z = z + 1;\n            }\n            a[x * n + y] = s;\n            y = y + 1;\n        }\n        x = x + 1;\n    }\n    let total = 0;\n    let t = 0;\n    while t < n * n {\n        total = total + c[t];\n        t = t + 1;\n    }\n    return total;\n}\n";

    fn accumulation_store(func: &Function) -> bool {
        for block in &func.blocks {
            for ins in &block.instrs {
                if let Instr::ArraySet { value, .. } = ins {
                    let (ab, ai) = match single_def(func, *value) {
                        Some(v) => v,
                        None => continue,
                    };
                    if !matches!(func.blocks.get(ab).and_then(|b| b.instrs.get(ai)), Some(Instr::Arith { op: ArithOp::Add, kind: NumKind::Int, .. })) {
                        continue;
                    }
                    return true;
                }
            }
        }
        false
    }

    #[test]
    fn fires_on_matmul_256_lir() {
        let f = lower_src(&matmul_src());
        assert!(match_fires(&f));
        let mut work = f.clone();
        let (action, _) = accumulate_interchange(&mut work);
        assert_eq!(action, AccumAction::V1);
    }

    #[test]
    fn zero_filled_small_fires_v1_with_accumulation_shape() {
        let f = lower_src(SMALL_ZERO);
        let mut work = f.clone();
        let (action, _) = accumulate_interchange(&mut work);
        assert_eq!(action, AccumAction::V1);
        assert!(accumulation_store(&work));
        let pairs = exact_pairs(&work);
        assert_eq!(pairs.len(), 5);
    }

    #[test]
    fn garbage_prefilled_small_fires_peel_not_v1() {
        let f = lower_src(SMALL_GARBAGE);
        let mut work = f.clone();
        let (action, _) = accumulate_interchange(&mut work);
        assert_eq!(action, AccumAction::Peel);
        assert!(accumulation_store(&work));
        let pairs = exact_pairs(&work);
        assert!(pairs.len() >= 5);
    }

    #[test]
    fn aliased_roots_fire_nothing_and_stay_identical() {
        let f = lower_src(SMALL_ALIASED);
        assert!(!match_fires(&f));
        let before = format!("{:?}", f);
        let mut work = f.clone();
        let (action, _) = accumulate_interchange(&mut work);
        assert_eq!(action, AccumAction::Untouched);
        assert_eq!(format!("{:?}", work), before);
    }

    #[test]
    fn non_target_sources_stay_byte_identical() {
        let sources = [
            "fn Main(): Int {\n    let s = 0;\n    let i = 0;\n    while i < 100 {\n        s = s + i;\n        i = i + 1;\n    }\n    return s;\n}\n",
            "fn Main(): Int {\n    let a = [1, 2, 3, 4, 5, 6, 7, 8];\n    let s = 0;\n    let i = 0;\n    while i < 8 {\n        s = s + a[i] * 2;\n        i = i + 1;\n    }\n    return s;\n}\n",
        ];
        for src in sources {
            let f = lower_src(src);
            let before = format!("{:?}", f);
            let mut work = f.clone();
            let (action, _) = accumulate_interchange(&mut work);
            assert_eq!(action, AccumAction::Untouched);
            assert_eq!(format!("{:?}", work), before);
        }
    }

    fn lower_bench(path: &str) -> Option<crate::instr::Module> {
        let dir = crate::scratch::dir("accum");
        std::fs::create_dir_all(&dir).ok()?;
        let src = std::fs::read_to_string(path).ok()?;
        std::fs::write(dir.join("main.rnx"), &src).ok()?;
        let g = frontend::modules::ModuleGraph::build(&dir.join("main.rnx")).ok()?;
        let mut m = g.resolve().ok()?;
        if !frontend::desugar::desugar(&mut m).is_empty() {
            return None;
        }
        let out = crate::lower::lower(&m).ok()?;
        Some(out)
    }

    #[test]
    fn bench_corpus_fire_set_is_matmul_only() {
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../benches");
        let subs = ["compute", "memory"];
        let mut firing: Vec<String> = Vec::new();
        let mut checked = 0usize;
        for sub in subs {
            let dir = format!("{root}/{sub}");
            let mut kinds: Vec<String> = Vec::new();
            for e in std::fs::read_dir(&dir).expect("read benches") {
                let e = e.expect("entry");
                if e.path().is_dir() {
                    kinds.push(e.file_name().to_string_lossy().to_string());
                }
            }
            kinds.sort();
            for kind in kinds {
                let path = format!("{dir}/{kind}/main.rnx");
                let module = lower_bench(&path).expect("bench lowers");
                checked += 1;
                let mut hit = false;
                for f in &module.functions {
                    if match_fires(f) {
                        hit = true;
                        break;
                    }
                }
                if hit {
                    firing.push(format!("{sub}/{kind}"));
                }
            }
        }
        assert!(checked >= 7, "corpus too small: {checked}");
        assert_eq!(firing, vec!["compute/matmul".to_string()], "{firing:?}");
    }

    #[test]
    fn pass_time_stays_within_debug_budget() {
        let f = lower_src(&matmul_src());
        let mut work = f.clone();
        let (_, times) = accumulate_interchange(&mut work);
        assert!(
            times.total() < Duration::from_millis(50),
            "accum pass took {:?}",
            times.total()
        );
    }
}

