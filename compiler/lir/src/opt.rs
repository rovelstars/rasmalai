use crate::instr::*;
use std::collections::{BTreeMap, BTreeSet};
#[cfg(not(target_arch = "wasm32"))]
use std::time::{Duration, Instant};
#[cfg(target_arch = "wasm32")]
use std::time::Duration;

#[cfg(not(target_arch = "wasm32"))]
fn stamp() -> Instant {
    Instant::now()
}
#[cfg(target_arch = "wasm32")]
fn stamp() -> Stamp {
    Stamp
}
#[cfg(target_arch = "wasm32")]
struct Stamp;
#[cfg(target_arch = "wasm32")]
impl Stamp {
    fn elapsed(&self) -> Duration {
        Duration::default()
    }
}

const MAX_PASSES: usize = 10;

#[derive(Default)]
pub struct OptTimings {
    pub inline: Duration,
    pub escape: Duration,
    pub sroa: Duration,
    pub licm: Duration,
    pub bce: Duration,
    pub arc: Duration,
    pub tco: Duration,
    pub fixpoint: Duration,
    pub sweep: Duration,
    pub deadfn: Duration,
}

pub fn optimize_lir(module: &mut Module, opt_level: u8, entry: &str) {
    let _ = optimize_lir_timed(module, opt_level, entry);
}

pub fn optimize_lir_lib(module: &mut Module, opt_level: u8) {
    let _ = optimize_lir_lib_timed(module, opt_level);
}

pub fn optimize_lir_timed(module: &mut Module, opt_level: u8, entry: &str) -> OptTimings {
    optimize_lir_impl(module, opt_level, entry, false)
}

pub fn optimize_lir_lib_timed(module: &mut Module, opt_level: u8) -> OptTimings {
    optimize_lir_impl(module, opt_level, "", true)
}

fn optimize_lir_impl(
    module: &mut Module,
    opt_level: u8,
    entry: &str,
    is_library: bool,
) -> OptTimings {
    let mut t = OptTimings::default();
    if opt_level == 0 {
        return t;
    }
    let mut step = stamp();
    inline_call_sites(module);
    t.inline = step.elapsed();
    step = stamp();
    for fi in 0..module.functions.len() {
        crate::escape::optimize_stack_allocations(module, fi);
    }
    t.escape = step.elapsed();
    step = stamp();
    for fi in 0..module.functions.len() {
        crate::sroa::scalar_replace_aggregates(module, fi);
    }
    t.sroa = step.elapsed();
    step = stamp();
    for fi in 0..module.functions.len() {
        crate::licm::loop_invariant_code_motion(&mut module.functions[fi]);
    }
    t.licm = step.elapsed();
    step = stamp();
    for fi in 0..module.functions.len() {        let inner = stamp();
        let dom = crate::licm::compute_dominators(&module.functions[fi]);
        let loops = crate::licm::find_natural_loops(&module.functions[fi], &dom);
        crate::bce::eliminate_bounds_checks(&mut module.functions[fi], &dom, &loops);
        t.bce += inner.elapsed();
        // bce only flips `unchecked` flags and appends guard instructions to
        // existing blocks, so the CFG `dom` describes is unchanged and arc
        // reuses it instead of rebuilding a second tree per function.
        let inner = stamp();
        crate::arc_opt::eliminate_redundant_arc(module, fi, &dom);
        t.arc += inner.elapsed();
    }
    step = stamp();
    for fi in 0..module.functions.len() {
        crate::tco::optimize_tail_calls(&mut module.functions[fi], fi);
    }
    t.tco = step.elapsed();
    step = stamp();
    for _ in 0..MAX_PASSES {
        let mut changed = false;
        for f in &mut module.functions {
            changed |= fold_consts(f);
            changed |= fma_synthesis(f);
            changed |= simplify_branches(f);
            changed |= prune_unreachable(f);
            changed |= compact_trampolines(f);
            changed |= dce(f);
        }
        if !changed {
            break;
        }
    }
    for fi in 0..module.functions.len() {
        let dom = crate::licm::compute_dominators(&module.functions[fi]);
        let loops = crate::licm::find_natural_loops(&module.functions[fi], &dom);
        if fma_reassoc_loops(&mut module.functions[fi], &loops) {
            dce(&mut module.functions[fi]);
        }
    }
    t.fixpoint = step.elapsed();
    step = stamp();
    for fi in 0..module.functions.len() {
        crate::temp_sweep::sweep_temps(module, fi);
    }
    t.sweep = step.elapsed();
    step = stamp();
    eliminate_dead_functions(module, entry, is_library);
    t.deadfn = step.elapsed();
    t
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ConstVal {
    Int(i64),
    Bool(bool),
}

fn scan_consts(block: &Block, mut known: BTreeMap<Local, ConstVal>) -> BTreeMap<Local, ConstVal> {
    for ins in &block.instrs {
        match ins {
            Instr::Const {  dst, lit , ..} => match lit {
                Lit::Int(n) => {
                    known.insert(*dst, ConstVal::Int(*n));
                }
                Lit::Bool(b) => {
                    known.insert(*dst, ConstVal::Bool(*b));
                }
                _ => {
                    known.remove(dst);
                }
            },
            Instr::Copy { dst, src, .. } => match known.get(src).copied() {
                Some(v) => {
                    known.insert(*dst, v);
                }
                None => {
                    known.remove(dst);
                }
            },
            Instr::Cast {  dst , ..} => {
                known.remove(dst);
            }
            Instr::Defer { ..} | Instr::RunDefers { ..} => known.clear(),
            _ => {
                if let Some(dst) = instr_dst(ins) {
                    known.remove(&dst);
                }
            }
        }
    }
    known
}

pub(crate) fn instr_dst(ins: &Instr) -> Option<Local> {    match ins {
        Instr::Const {  dst , ..}
        | Instr::Copy {  dst , ..}
        | Instr::Cast {  dst , ..}
        | Instr::Convert {  dst , ..}
        | Instr::Arith {  dst , ..}
        | Instr::Fma {  dst , ..}
        | Instr::Cmp {  dst , ..}
        | Instr::Not {  dst , ..}
        | Instr::Neg {  dst , ..}
        | Instr::Concat {  dst , ..}
        | Instr::ToStr {  dst , ..}
        | Instr::Range {  dst , ..}
        | Instr::Stride {  dst , ..}
        | Instr::ArrayNew {  dst , ..}
        | Instr::ArrayPop {  dst , ..}
        | Instr::ArrayLen {  dst , ..}
        | Instr::ArrayGet {  dst , ..}
        | Instr::EnumNew {  dst , ..}
        | Instr::EnumPayload {  dst , ..}
        | Instr::EnumTag {  dst , ..}
        | Instr::Extract {  dst , ..}
        | Instr::AddrOf {  dst , ..}
        | Instr::PtrLoad {  dst , ..}
        | Instr::RangeLo {  dst , ..}
        | Instr::RangeHi {  dst , ..}
        | Instr::RangeStep {  dst , ..}
        | Instr::GetField {  dst , ..}
        | Instr::GetFieldByName {  dst , ..}
        | Instr::ObjNew {  dst , ..}
        | Instr::StackAlloc {  dst , ..}
        | Instr::ClosureNew {  dst , ..}
        | Instr::GenRefOf {  dst , ..}
        | Instr::GenRefEmpty {  dst , ..}
        | Instr::GenRefGet {  dst , ..}
        | Instr::ThreadSpawn {  dst , ..}
        | Instr::ThreadJoin {  dst , ..}
        | Instr::PoolInit {  dst , ..}
        | Instr::VecNew {  dst , ..}
        | Instr::VecSplat {  dst , ..}
        | Instr::VecExtract {  dst , ..}
        | Instr::VecInsert {  dst , ..}
        | Instr::VecArith {  dst , ..}
        | Instr::VecUnary {  dst , ..}
        | Instr::VecDot {  dst , ..} => Some(*dst),
        Instr::Call { dsts, ..} => {
            if dsts.len() == 1 {
                Some(dsts[0])
            } else {
                None
            }
        }
        _ => None,
    }
}

pub(crate) fn instr_dsts(ins: &Instr, out: &mut Vec<Local>) {
    match ins {
        Instr::Call { dsts, .. } => out.extend(dsts.iter().copied()),
        _ => {
            if let Some(d) = instr_dst(ins) {
                out.push(d);
            }
        }
    }
}

fn fold_consts(f: &mut Function) -> bool {
    let mut preds: Vec<Vec<BlockId>> = vec![Vec::new(); f.blocks.len()];
    for (i, block) in f.blocks.iter().enumerate() {
        for t in term_targets(&block.term) {
            if t < preds.len() {
                preds[t].push(i);
            }
        }
    }
    let mut exits: Vec<BTreeMap<Local, ConstVal>> = vec![BTreeMap::new(); f.blocks.len()];
    let mut changed = false;
    for i in 0..f.blocks.len() {
        let mut known = if preds[i].len() == 1 && preds[i][0] != i && preds[i][0] < i {
            exits[preds[i][0]].clone()
        } else {
            BTreeMap::new()
        };
        let block = &mut f.blocks[i];
        for ins in &mut block.instrs {
            match ins {
                Instr::Const {  dst, lit , ..} => match lit {
                    Lit::Int(n) => {
                        known.insert(*dst, ConstVal::Int(*n));
                    }
                    Lit::Bool(b) => {
                        known.insert(*dst, ConstVal::Bool(*b));
                    }
                    _ => {
                        known.remove(dst);
                    }
                },
                Instr::Copy { dst, src, .. } => {
                    let (dst, src) = (*dst, *src);
                    match known.get(&src).copied() {
                        Some(ConstVal::Int(n)) => {
                            *ins = Instr::Const { span: ins.span(), 
                                dst,
                                lit: Lit::Int(n),
                            };
                            known.insert(dst, ConstVal::Int(n));
                            changed = true;
                        }
                        Some(ConstVal::Bool(b)) => {
                            *ins = Instr::Const { span: ins.span(),  dst, lit: Lit::Bool(b) };
                            known.insert(dst, ConstVal::Bool(b));
                            changed = true;
                        }
                        None => {
                            known.remove(&dst);
                        }
                    }
                }
                Instr::Arith {  op, kind, dst, lhs, rhs , ..}
                    if *kind == NumKind::Int =>
                {
                    let (op, dst, lhs, rhs) = (*op, *dst, *lhs, *rhs);
                    let l = known.get(&lhs).copied();
                    let r = known.get(&rhs).copied();
                    let folded = match (l, r) {
                        (Some(ConstVal::Int(a)), Some(ConstVal::Int(b))) => {
                            arith_const(op, a, b).map(ConstVal::Int)
                        }
                        _ => None,
                    };
                    match folded {
                        Some(ConstVal::Int(n)) => {
                            *ins = Instr::Const { span: ins.span(), 
                                dst,
                                lit: Lit::Int(n),
                            };
                            known.insert(dst, ConstVal::Int(n));
                            changed = true;
                        }
                        Some(ConstVal::Bool(_)) => {}
                        None => {
                            if lhs == rhs && op == ArithOp::Sub {
                                *ins = Instr::Const { span: ins.span(), 
                                    dst,
                                    lit: Lit::Int(0),
                                };
                                known.insert(dst, ConstVal::Int(0));
                                changed = true;
                            } else if op == ArithOp::Mod && is_int_one(rhs, &known) {
                                *ins = Instr::Const { span: ins.span(), 
                                    dst,
                                    lit: Lit::Int(0),
                                };
                                known.insert(dst, ConstVal::Int(0));
                                changed = true;
                            } else if op == ArithOp::Mul
                                && (is_int_zero(lhs, &known) || is_int_zero(rhs, &known))
                            {
                                *ins = Instr::Const { span: ins.span(), 
                                    dst,
                                    lit: Lit::Int(0),
                                };
                                known.insert(dst, ConstVal::Int(0));
                                changed = true;
                            } else if let Some(src) = identity_copy(op, lhs, rhs, &known) {
                                *ins = Instr::Copy { span: ins.span(),  dst, src };
                                match known.get(&src).copied() {
                                    Some(v) => {
                                        known.insert(dst, v);
                                    }
                                    None => {
                                        known.remove(&dst);
                                    }
                                }
                                changed = true;
                            } else {
                                known.remove(&dst);
                            }
                        }
                    }
                }
                Instr::Cmp {  op, kind, dst, lhs, rhs , ..} if *kind == NumKind::Int => {
                    let (op, dst, lhs, rhs) = (*op, *dst, *lhs, *rhs);
                    match (known.get(&lhs).copied(), known.get(&rhs).copied()) {
                        (Some(ConstVal::Int(a)), Some(ConstVal::Int(b))) => {
                            let v = match op {
                                CmpOp::Eq => a == b,
                                CmpOp::NotEq => a != b,
                                CmpOp::Lt => a < b,
                                CmpOp::LtEq => a <= b,
                                CmpOp::Gt => a > b,
                                CmpOp::GtEq => a >= b,
                            };
                            *ins = Instr::Const { span: ins.span(), 
                                dst,
                                lit: Lit::Bool(v),
                            };
                            known.insert(dst, ConstVal::Bool(v));
                            changed = true;
                        }
                        _ => {
                            known.remove(&dst);
                        }
                    }
                }
                Instr::Not {  dst, src , ..} => {
                    let (dst, src) = (*dst, *src);
                    match known.get(&src).copied() {
                        Some(ConstVal::Bool(b)) => {
                            *ins = Instr::Const { span: ins.span(), 
                                dst,
                                lit: Lit::Bool(!b),
                            };
                            known.insert(dst, ConstVal::Bool(!b));
                            changed = true;
                        }
                        _ => {
                            known.remove(&dst);
                        }
                    }
                }
                Instr::Defer { ..} | Instr::RunDefers { ..} => known.clear(),
                _ => {
                    if let Some(dst) = instr_dst(ins) {
                        known.remove(&dst);
                    }
                }
            }
        }
        exits[i] = known;
    }
    changed
}

fn is_int_zero(local: Local, known: &BTreeMap<Local, ConstVal>) -> bool {
    matches!(known.get(&local), Some(ConstVal::Int(0)))
}

fn is_int_one(local: Local, known: &BTreeMap<Local, ConstVal>) -> bool {
    matches!(known.get(&local), Some(ConstVal::Int(1)))
}

fn arith_const(op: ArithOp, a: i64, b: i64) -> Option<i64> {
    match op {
        ArithOp::Add => a.checked_add(b),
        ArithOp::Sub => a.checked_sub(b),
        ArithOp::Mul => a.checked_mul(b),
        ArithOp::Div => a.checked_div(b),
        ArithOp::Mod => a.checked_rem(b),
        ArithOp::BitAnd => Some(a & b),
        ArithOp::BitOr => Some(a | b),
        ArithOp::BitXor => Some(a ^ b),
        ArithOp::Shl => Some(a.wrapping_shl((b as u64 & 63) as u32)),
        ArithOp::Shr => Some(a.wrapping_shr((b as u64 & 63) as u32)),
        ArithOp::Zshr => Some(((a as u64).wrapping_shr((b as u64 & 63) as u32)) as i64),
    }
}

fn identity_copy(
    op: ArithOp,
    lhs: Local,
    rhs: Local,
    known: &BTreeMap<Local, ConstVal>,
) -> Option<Local> {
    match op {
        ArithOp::Add if is_int_zero(rhs, known) => Some(lhs),
        ArithOp::Add if is_int_zero(lhs, known) => Some(rhs),
        ArithOp::Sub if is_int_zero(rhs, known) => Some(lhs),
        ArithOp::Mul if is_int_one(rhs, known) => Some(lhs),
        ArithOp::Mul if is_int_one(lhs, known) => Some(rhs),
        ArithOp::Div if is_int_one(rhs, known) => Some(lhs),
        ArithOp::BitAnd if matches!(known.get(&rhs), Some(ConstVal::Int(-1))) => Some(lhs),
        ArithOp::BitAnd if matches!(known.get(&lhs), Some(ConstVal::Int(-1))) => Some(rhs),
        ArithOp::BitOr if is_int_zero(rhs, known) => Some(lhs),
        ArithOp::BitOr if is_int_zero(lhs, known) => Some(rhs),
        ArithOp::BitXor if is_int_zero(rhs, known) => Some(lhs),
        ArithOp::BitXor if is_int_zero(lhs, known) => Some(rhs),
        _ => None,
    }
}

fn simplify_branches(f: &mut Function) -> bool {
    let mut preds: Vec<Vec<BlockId>> = vec![Vec::new(); f.blocks.len()];
    for (i, block) in f.blocks.iter().enumerate() {
        for t in term_targets(&block.term) {
            if t < preds.len() {
                preds[t].push(i);
            }
        }
    }
    let mut ends: Vec<BTreeMap<Local, ConstVal>> = Vec::with_capacity(f.blocks.len());
    for (i, block) in f.blocks.iter().enumerate() {
        let start = if preds[i].len() == 1 && preds[i][0] != i && preds[i][0] < i {
            ends[preds[i][0]].clone()
        } else {
            BTreeMap::new()
        };
        ends.push(scan_consts(block, start));
    }
    let mut changed = false;
    for i in 0..f.blocks.len() {
        let term = f.blocks[i].term.clone();
        if let Terminator::BrIf { cond, then_bb, else_bb, .. } = term {
            match ends[i].get(&cond).copied() {
                Some(ConstVal::Bool(true)) => {
                    f.blocks[i].term = Terminator::Br(then_bb);
                    changed = true;
                }
                Some(ConstVal::Bool(false)) => {
                    f.blocks[i].term = Terminator::Br(else_bb);
                    changed = true;
                }
                _ => {}
            }
        }
    }
    changed
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

fn remap_term(term: &mut Terminator, map: &[usize]) {
    let go = |b: &mut BlockId| *b = map[*b];
    match term {
        Terminator::Ret(_) | Terminator::Unreachable { .. } => {}
        Terminator::Br(t) => go(t),
        Terminator::BrIf { then_bb, else_bb, .. } => {
            go(then_bb);
            go(else_bb);
        }
        Terminator::BrErr { catch_bb, next_bb, .. } => {
            go(catch_bb);
            go(next_bb);
        }
        Terminator::Switch { cases, default, .. } => {
            for (_, b) in cases {
                go(b);
            }
            go(default);
        }
        Terminator::Throw { catch, .. } => {
            if let Some((b, _, _)) = catch {
                go(b);
            }
        }
        Terminator::Rethrow { catch_bb, .. } => go(catch_bb),
    }
}

fn prune_unreachable(f: &mut Function) -> bool {
    let mut seen = vec![false; f.blocks.len()];
    let mut stack = vec![0];
    seen[0] = true;
    while let Some(b) = stack.pop() {
        for t in term_targets(&f.blocks[b].term) {
            if t < seen.len() && !seen[t] {
                seen[t] = true;
                stack.push(t);
            }
        }
    }
    if seen.iter().all(|s| *s) {
        return false;
    }
    let mut map = vec![usize::MAX; f.blocks.len()];
    let mut kept: Vec<Block> = Vec::new();
    for (i, block) in f.blocks.drain(..).enumerate() {
        if seen[i] {
            map[i] = kept.len();
            kept.push(block);
        }
    }
    for block in &mut kept {
        remap_term(&mut block.term, &map);
    }
    f.blocks = kept;
    true
}

fn compact_trampolines(f: &mut Function) -> bool {
    let mut changed = false;
    for b in 0..f.blocks.len() {
        let target = match &f.blocks[b].term {
            Terminator::Br(t) if f.blocks[b].instrs.is_empty() && *t != b => *t,
            _ => continue,
        };
        for other in 0..f.blocks.len() {
            if rewrite_target(&mut f.blocks[other].term, b, target) {
                changed = true;
            }
        }
    }
    changed
}

pub(crate) fn rewrite_target(term: &mut Terminator, from: BlockId, to: BlockId) -> bool {
    let mut changed = false;
    let mut go = |b: &mut BlockId| {
        if *b == from {
            *b = to;
            changed = true;
        }
    };
    match term {
        Terminator::Ret(_) | Terminator::Unreachable { .. } => {}
        Terminator::Br(t) => go(t),
        Terminator::BrIf { then_bb, else_bb, .. } => {
            go(then_bb);
            go(else_bb);
        }
        Terminator::BrErr { catch_bb, next_bb, .. } => {
            go(catch_bb);
            go(next_bb);
        }
        Terminator::Switch { cases, default, .. } => {
            for (_, b) in cases {
                go(b);
            }
            go(default);
        }
        Terminator::Throw { catch, .. } => {
            if let Some((b, _, _)) = catch {
                go(b);
            }
        }
        Terminator::Rethrow { catch_bb, .. } => go(catch_bb),
    }
    changed
}

pub(crate) fn instr_reads(ins: &Instr, out: &mut Vec<Local>) {
    match ins {
        Instr::Const { ..} | Instr::GenRefEmpty { ..} => {}
        Instr::Copy {  src , ..}
        | Instr::Cast {  src , ..}
        | Instr::Not {  src , ..}
        | Instr::ToStr {  src , ..}
        | Instr::AddrOf {  src , ..}
        | Instr::PtrLoad {  ptr: src , ..}
        | Instr::RangeLo {  range: src , ..}
        | Instr::RangeHi {  range: src , ..}
        | Instr::RangeStep {  range: src , ..}
        | Instr::GenRefOf {  obj: src , ..}
        | Instr::GenRefInvalidate {  obj: src , ..}
        | Instr::Retain {  obj: src , ..}
        | Instr::Release {  obj: src , ..}
        | Instr::ReleaseAs {  obj: src , ..}
        | Instr::ThreadJoin {  handle: src , ..} => out.push(*src),
        Instr::PoolInit {  id, workers , ..} => {
            out.push(*id);
            out.push(*workers);
        }
        Instr::PoolSubmit {  pool, arg , ..} => {
            out.push(*pool);
            if let Some(a) = arg {
                out.push(*a);
            }
        }
        Instr::PoolParallelFor {  pool, start, end, chunk , ..} => {
            out.push(*pool);
            out.push(*start);
            out.push(*end);
            out.push(*chunk);
        }
        Instr::PoolJoin {  pool , ..} | Instr::PoolShutdown { pool , ..} => out.push(*pool),
        Instr::VecNew {  x, y, z, w , ..} => {
            out.push(*x);
            out.push(*y);
            out.push(*z);
            out.push(*w);
        }
        Instr::VecSplat {  val , ..} => out.push(*val),
        Instr::VecExtract {  vec, lane , ..} => {
            out.push(*vec);
            out.push(*lane);
        }
        Instr::VecInsert {  vec, val , ..} => {
            out.push(*vec);
            out.push(*val);
        }
        Instr::VecArith {  lhs, rhs , ..} => {
            out.push(*lhs);
            out.push(*rhs);
        }
        Instr::VecUnary {  src , ..} => out.push(*src),
        Instr::VecDot {  lhs, rhs , ..} => {
            out.push(*lhs);
            out.push(*rhs);
        }
        Instr::Convert {  src , ..}
        | Instr::Neg {  src , ..}
        | Instr::EnumTag {  scrut: src , ..} => out.push(*src),
        Instr::Arith {  lhs, rhs , ..}
        | Instr::Cmp {  lhs, rhs , ..}
        | Instr::Concat {  lhs, rhs , ..} => {
            out.push(*lhs);
            out.push(*rhs);
        }
        Instr::Fma {  a, b, c , ..} => {
            out.push(*a);
            out.push(*b);
            out.push(*c);
        }
        Instr::Range {  lo, hi , ..} => {
            out.push(*lo);
            out.push(*hi);
        }
        Instr::Stride {  range, step , ..} => {
            out.push(*range);
            out.push(*step);
        }
        Instr::ArrayPush {  arr, value , ..} => {
            out.push(*arr);
            out.push(*value);
        }
        Instr::ArrayPop {  arr , ..} | Instr::ArrayLen { arr , ..} => out.push(*arr),
        Instr::ArrayGet {  arr, index , ..} => {
            out.push(*arr);
            out.push(*index);
        }
        Instr::ArraySet {  arr, index, value , ..} => {
            out.push(*arr);
            out.push(*index);
            out.push(*value);
        }
        Instr::EnumNew {  payload , ..} => out.extend(payload.iter().copied()),
        Instr::EnumPayload {  scrut , ..} => out.push(*scrut),
        Instr::Extract {  base , ..} => out.push(*base),
        Instr::GetField {  obj , ..}
        | Instr::GetFieldByName {  obj , ..}
        | Instr::ReleaseField {  obj , ..} => out.push(*obj),
        Instr::SetField {  obj, value , ..} | Instr::SetFieldByName { obj, value , ..} => {
            out.push(*obj);
            out.push(*value);
        }
        Instr::PtrStore {  ptr, val , ..} => {
            out.push(*ptr);
            out.push(*val);
        }
        Instr::PtrLoad {  ptr , ..} => out.push(*ptr),
        Instr::ClosureNew {  captures , ..} => out.extend(captures.iter().copied()),
        Instr::Call {  target, args , ..} => {
            match target {
                CallTarget::Value(l) => out.push(*l),
                CallTarget::Dyn { obj, .. } => out.push(*obj),
                _ => {}
            }
            out.extend(args.iter().copied());
        }
        Instr::GenRefGet {  gref , ..} => out.push(*gref),
        Instr::Defer {  body , ..} => {
            for nested in body {
                instr_reads(nested, out);
            }
        }
        Instr::RunDefers { ..}
        | Instr::ThreadSpawn { ..}
        | Instr::ObjNew { ..}
        | Instr::StackAlloc { ..}
        | Instr::ArrayNew { ..}
        | Instr::Assert { ..}
        | Instr::Panic { ..} => {}
    }
    match ins {
        Instr::Assert {  cond, message , ..} => {
            out.push(*cond);
            out.push(*message);
        }
        Instr::Panic {  message , ..} => out.push(*message),
        _ => {}
    }
}

pub(crate) fn term_reads(term: &Terminator, out: &mut Vec<Local>) {
    match term {
        Terminator::Ret(v) => {
            out.extend(v.iter().copied());
        }
        Terminator::Br(_) | Terminator::Unreachable { .. } => {}
        Terminator::BrIf { cond, .. } => out.push(*cond),
        Terminator::BrErr { err, .. } => out.push(*err),
        Terminator::Switch { scrut, cases, .. } => {
            out.push(*scrut);
            for (pat, _) in cases {
                if let SwitchPat::Is { source, .. } = pat {
                    out.push(*source);
                }
            }
        }
        Terminator::Throw { src, .. } => out.push(*src),
        Terminator::Rethrow { err, .. } => out.push(*err),
    }
}

fn is_pure(ins: &Instr) -> bool {
    matches!(
        ins,
        Instr::Const { ..}
            | Instr::Copy { ..}
            | Instr::Cast { ..}
            | Instr::Arith { ..}
            | Instr::Fma { ..}
            | Instr::Cmp { ..}
            | Instr::Not { ..}
            | Instr::Neg { ..}
            | Instr::Convert { ..}
            | Instr::StackAlloc { ..}
    )
}

fn fma_synthesis(f: &mut Function) -> bool {
    let mut uses: BTreeMap<Local, usize> = BTreeMap::new();
    let mut tmp: Vec<Local> = Vec::new();
    for block in &f.blocks {
        for ins in &block.instrs {
            tmp.clear();
            instr_reads(ins, &mut tmp);
            for l in tmp.iter() {
                *uses.entry(*l).or_insert(0) += 1;
            }
        }
        tmp.clear();
        term_reads(&block.term, &mut tmp);
        for l in tmp.iter() {
            *uses.entry(*l).or_insert(0) += 1;
        }
    }
    let mut changed = false;
    for bi in 0..f.blocks.len() {
        let mut def: BTreeMap<Local, usize> = BTreeMap::new();
        for (ii, ins) in f.blocks[bi].instrs.iter().enumerate() {
            if let Some(dst) = instr_dst(ins) {
                def.insert(dst, ii);
            }
        }
        let mut at = 0;
        while at < f.blocks[bi].instrs.len() {
            let fused = match &f.blocks[bi].instrs[at] {
                Instr::Arith { span, op: ArithOp::Add, kind: NumKind::Float(FloatKind::Fast), dst, lhs, rhs } => {
                    let (t, c) = (*lhs, *rhs);
                    let other = if uses.get(&t).copied().unwrap_or(0) == 1 { Some((t, c)) } else if uses.get(&c).copied().unwrap_or(0) == 1 { Some((c, t)) } else { None };
                    match other {
                        Some((m, cc)) if *dst != m => match def.get(&m) {
                            Some(&mi) if mi < at => match &f.blocks[bi].instrs[mi] {
                                Instr::Arith { op: ArithOp::Mul, kind: NumKind::Float(FloatKind::Fast), dst: md, lhs: a, rhs: b, .. } if *md == m => {
                                    Some((*span, *dst, *a, *b, cc))
                                }
                                _ => None,
                            },
                            _ => None,
                        },
                        _ => None,
                    }
                }
                _ => None,
            };
            if let Some((span, dst, a, b, c)) = fused {
                f.blocks[bi].instrs[at] = Instr::Fma { span, dst, a, b, c };
                changed = true;
            }
            at += 1;
        }
    }
    changed
}

fn fma_reassoc_loops(f: &mut Function, loops: &[crate::licm::NaturalLoop]) -> bool {
    let mut changed = false;
    loop {
        let mut uses: BTreeMap<Local, usize> = BTreeMap::new();
        let mut tmp: Vec<Local> = Vec::new();
        for block in &f.blocks {
            for ins in &block.instrs {
                tmp.clear();
                instr_reads(ins, &mut tmp);
                for l in tmp.iter() {
                    *uses.entry(*l).or_insert(0) += 1;
                }
            }
            tmp.clear();
            term_reads(&block.term, &mut tmp);
            for l in tmp.iter() {
                *uses.entry(*l).or_insert(0) += 1;
            }
        }
        let mut def_block: BTreeMap<Local, BlockId> = BTreeMap::new();
        let mut multi = BTreeSet::new();
        for (bi, block) in f.blocks.iter().enumerate() {
            for ins in &block.instrs {
                if let Some(dst) = instr_dst(ins) {
                    if def_block.insert(dst, bi).is_some() {
                        multi.insert(dst);
                    }
                }
            }
        }
        let sole_home = |l: Local| -> Option<BlockId> {
            if multi.contains(&l) {
                return None;
            }
            def_block.get(&l).copied()
        };
        let mut fired = false;
        'outer: for lp in loops {
            for &bi in lp.blocks.iter() {
                let mut at = 0;
                while at < f.blocks[bi].instrs.len() {
                    let reassoc = match &f.blocks[bi].instrs[at] {
                        Instr::Arith { span, op: ArithOp::Add, kind: NumKind::Float(FloatKind::Fast), dst, lhs, rhs } => {
                            let (t, c) = (*lhs, *rhs);
                            let cand = if uses.get(&t).copied().unwrap_or(0) == 1 { Some((t, c)) } else if uses.get(&c).copied().unwrap_or(0) == 1 { Some((c, t)) } else { None };
                            match cand {
                                Some((m, cc)) if *dst != m => {
                                    let sub_ok = match sole_home(m) {
                                        Some(sbi) if sbi == bi => match f.blocks[sbi].instrs.iter().find(|i| instr_dst(i) == Some(m)) {
                                            Some(Instr::Arith { op: ArithOp::Sub, kind: NumKind::Float(FloatKind::Fast), lhs: a, rhs: b, .. }) => Some((*a, *b)),
                                            _ => None,
                                        },
                                        _ => None,
                                    };
                                    match sub_ok {
                                        Some((a, b)) => {
                                            let c_out = sole_home(cc).map(|h| !lp.blocks.contains(&h)).unwrap_or(false);
                                            let a_in = sole_home(a).map(|h| lp.blocks.contains(&h)).unwrap_or(false);
                                            let b_in = sole_home(b).map(|h| lp.blocks.contains(&h)).unwrap_or(false);
                                            if c_out && (a_in || b_in) && cc < a {
                                                Some((*span, *dst, m, a, b, cc))
                                            } else {
                                                None
                                            }
                                        }
                                        None => None,
                                    }
                                }
                                _ => None,
                            }
                        }
                        _ => None,
                    };
                    if let Some((span, dst, m, a, b, c)) = reassoc {
                        let t2 = f.locals.len() as Local;
                        let mi = f.blocks[bi].instrs.iter().position(|i| instr_dst(i) == Some(m)).unwrap_or(at);
                        f.blocks[bi].instrs[mi] = Instr::Arith { span, op: ArithOp::Sub, kind: NumKind::Float(FloatKind::Fast), dst: t2, lhs: c, rhs: b };
                        f.blocks[bi].instrs[at] = Instr::Arith { span, op: ArithOp::Add, kind: NumKind::Float(FloatKind::Fast), dst, lhs: t2, rhs: a };
                        f.locals.push(LirType::F64(FloatKind::Fast));
                        changed = true;
                        fired = true;
                        break 'outer;
                    }
                    at += 1;
                }
            }
        }
        if !fired {
            break;
        }
    }
    changed
}

fn dce(f: &mut Function) -> bool {
    let mut reads: BTreeSet<Local> = BTreeSet::new();
    let mut tmp: Vec<Local> = Vec::new();
    for block in &f.blocks {
        for ins in &block.instrs {
            tmp.clear();
            instr_reads(ins, &mut tmp);
            reads.extend(tmp.iter().copied());
        }
        tmp.clear();
        term_reads(&block.term, &mut tmp);
        reads.extend(tmp.iter().copied());
    }
    let mut changed = false;
    for block in &mut f.blocks {
        let before = block.instrs.len();
        block.instrs.retain(|ins| {
            if !is_pure(ins) {
                return true;
            }
            match instr_dst(ins) {
                Some(dst) => reads.contains(&dst),
                None => true,
            }
        });
        changed |= block.instrs.len() != before;
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn single_block() -> Function {
        Function {
            name: "compute".to_string(),
            params: Vec::new(),
            sig_params: Vec::new(),
            ret: LirType::I64,
            throws: false,
            is_unsafe: false,
            method_self: false,
            is_pub: false,
            is_closure: false,
            locals: vec![LirType::I64; 8],
            blocks: vec![Block {
                instrs: vec![
                    Instr::Const {span: UNKNOWN_SPAN,    dst: 0, lit: Lit::Int(10) },
                    Instr::Const {span: UNKNOWN_SPAN,    dst: 1, lit: Lit::Int(4) },
                    Instr::Arith {span: UNKNOWN_SPAN, 
                        op: ArithOp::Mul,
                        kind: NumKind::Int,
                        dst: 2,
                        lhs: 0,
                        rhs: 1,
                    },
                    Instr::Const {span: UNKNOWN_SPAN,    dst: 3, lit: Lit::Int(5) },
                    Instr::Const {span: UNKNOWN_SPAN,    dst: 4, lit: Lit::Int(3) },
                    Instr::Arith {span: UNKNOWN_SPAN, 
                        op: ArithOp::Sub,
                        kind: NumKind::Int,
                        dst: 5,
                        lhs: 3,
                        rhs: 4,
                    },
                    Instr::Arith {span: UNKNOWN_SPAN, 
                        op: ArithOp::Add,
                        kind: NumKind::Int,
                        dst: 6,
                        lhs: 2,
                        rhs: 5,
                    },
                    Instr::Copy {span: UNKNOWN_SPAN,   dst: 7, src: 6 },
                ],
                term: Terminator::Ret(vec![7]),
            }],
        }
    }

    #[test]
    fn folds_arithmetic_chain_to_const() {
        let mut module = Module {
            functions: vec![single_block()],
            ..Default::default()
        };
        optimize_lir(&mut module, 1, "Main");
        let arith: usize = module.functions[0]
            .blocks
            .iter()
            .flat_map(|b| b.instrs.iter())
            .filter(|i| matches!(i, Instr::Arith { ..}))
            .count();
        assert_eq!(arith, 0);
        let ints: Vec<i64> = module.functions[0]
            .blocks
            .iter()
            .flat_map(|b| b.instrs.iter())
            .filter_map(|i| match i {
                Instr::Const {  lit: Lit::Int(n) , ..} => Some(*n),
                _ => None,
            })
            .collect();
        assert!(ints.contains(&42), "{ints:?}");
    }

    #[test]
    fn fuses_fast_mul_add_into_fma() {
        let ff = NumKind::Float(FloatKind::Fast);
        let st = NumKind::Float(FloatKind::Strict);
        let mut f = Function {
            name: "fma".to_string(),
            params: Vec::new(),
            sig_params: Vec::new(),
            ret: LirType::I64,
            throws: false,
            is_unsafe: false,
            method_self: false,
            is_pub: false,
            is_closure: false,
            locals: vec![LirType::F64(FloatKind::Fast); 9],
            blocks: vec![Block {
                instrs: vec![
                    Instr::Arith {span: UNKNOWN_SPAN,  op: ArithOp::Mul, kind: ff, dst: 3, lhs: 0, rhs: 1 },
                    Instr::Arith {span: UNKNOWN_SPAN,  op: ArithOp::Add, kind: ff, dst: 4, lhs: 3, rhs: 2 },
                    Instr::Arith {span: UNKNOWN_SPAN,  op: ArithOp::Mul, kind: st, dst: 6, lhs: 0, rhs: 1 },
                    Instr::Arith {span: UNKNOWN_SPAN,  op: ArithOp::Add, kind: st, dst: 7, lhs: 6, rhs: 2 },
                    Instr::Arith {span: UNKNOWN_SPAN,  op: ArithOp::Add, kind: ff, dst: 8, lhs: 4, rhs: 7 },
                ],
                term: Terminator::Ret(vec![8]),
            }],
        };
        assert!(fma_synthesis(&mut f));
        let fmas: Vec<(Local, Local, Local, Local)> = f.blocks[0]
            .instrs
            .iter()
            .filter_map(|i| match i {
                Instr::Fma {  dst, a, b, c , ..} => Some((*dst, *a, *b, *c)),
                _ => None,
            })
            .collect();
        assert_eq!(fmas, vec![(4, 0, 1, 2)]);
        assert!(dce(&mut f));
        assert!(!f.blocks[0].instrs.iter().any(|i| matches!(i, Instr::Arith { op: ArithOp::Mul, kind: NumKind::Float(FloatKind::Fast), .. })));
        assert!(f.blocks[0].instrs.iter().any(|i| matches!(i, Instr::Arith { op: ArithOp::Mul, kind: NumKind::Float(FloatKind::Strict), .. })));
    }

    #[test]
    fn reassociates_invariant_addend_outward() {
        use crate::licm::NaturalLoop;
        use std::collections::BTreeSet;
        let ff = NumKind::Float(FloatKind::Fast);
        let mut f = Function {
            name: "reassoc".to_string(),
            params: vec![LirType::F64(FloatKind::Fast)],
            sig_params: vec![],
            ret: LirType::I64,
            throws: false,
            is_unsafe: false,
            method_self: false,
            is_pub: false,
            is_closure: false,
            locals: vec![LirType::F64(FloatKind::Fast); 7],
            blocks: vec![
                Block {
                    instrs: vec![
                        Instr::Copy {span: UNKNOWN_SPAN,  dst: 1, src: 0 },
                    ],
                    term: Terminator::Br(1),
                },
                Block {
                    instrs: vec![
                        Instr::Copy {span: UNKNOWN_SPAN,  dst: 2, src: 0 },
                        Instr::Copy {span: UNKNOWN_SPAN,  dst: 5, src: 0 },
                        Instr::Arith {span: UNKNOWN_SPAN,  op: ArithOp::Sub, kind: ff, dst: 3, lhs: 5, rhs: 2 },
                        Instr::Arith {span: UNKNOWN_SPAN,  op: ArithOp::Add, kind: ff, dst: 4, lhs: 3, rhs: 1 },
                    ],
                    term: Terminator::Br(1),
                },
            ],
        };
        let loops = vec![NaturalLoop { header: 1, blocks: BTreeSet::from([1]) }];
        assert!(fma_reassoc_loops(&mut f, &loops));
        let sub = f.blocks[1].instrs[2].clone();
        let add = f.blocks[1].instrs[3].clone();
        assert!(matches!(sub, Instr::Arith { op: ArithOp::Sub, dst: 7, lhs: 1, rhs: 2, .. }), "{sub:?}");
        assert!(matches!(add, Instr::Arith { op: ArithOp::Add, dst: 4, lhs: 7, rhs: 5, .. }), "{add:?}");
        assert!(!fma_reassoc_loops(&mut f, &loops), "reassoc must reach a fixpoint");
    }

    #[test]
    fn prunes_constant_false_branch() {
        let mut f = Function {
            name: "pick".to_string(),
            params: Vec::new(),
            sig_params: Vec::new(),
            ret: LirType::I64,
            throws: false,
            is_unsafe: false,
            method_self: false,
            is_pub: false,
            is_closure: false,
            locals: vec![LirType::Bool, LirType::I64, LirType::I64],
            blocks: vec![
                Block {
                    instrs: vec![Instr::Const {span: UNKNOWN_SPAN,    dst: 0, lit: Lit::Bool(false) }],
                    term: Terminator::BrIf {span: UNKNOWN_SPAN,  cond: 0, then_bb: 1, else_bb: 2 },
                },
                Block {
                    instrs: vec![Instr::Const {span: UNKNOWN_SPAN,    dst: 1, lit: Lit::Int(999) }],
                    term: Terminator::Ret(vec![1]),
                },
                Block {
                    instrs: vec![Instr::Const {span: UNKNOWN_SPAN,    dst: 2, lit: Lit::Int(42) }],
                    term: Terminator::Ret(vec![2]),
                },
            ],
        };
        let mut module = Module { functions: vec![f], ..Default::default() };
        optimize_lir(&mut module, 1, "Main");
        f = module.functions.pop().unwrap();
        assert_eq!(f.blocks.len(), 2);
        assert!(matches!(f.blocks[0].term, Terminator::Br(1)));
        let ints: Vec<i64> = f
            .blocks
            .iter()
            .flat_map(|b| b.instrs.iter())
            .filter_map(|i| match i {
                Instr::Const {  lit: Lit::Int(n) , ..} => Some(*n),
                _ => None,
            })
            .collect();
        assert!(!ints.contains(&999), "{ints:?}");
        assert!(ints.contains(&42), "{ints:?}");
    }

    #[test]
    fn level_zero_is_identity() {
        let mut module = Module {
            functions: vec![single_block()],
            ..Default::default()
        };
        let before = format!("{:?}", module.functions[0]);
        optimize_lir(&mut module, 0, "Main");
        assert_eq!(format!("{:?}", module.functions[0]), before);
    }
}

pub fn is_inline_candidate(func: &Function) -> bool {
    if func.throws {
        return false;
    }
    let mut count = 0;
    for block in &func.blocks {
        for ins in &block.instrs {
            match ins {
                Instr::Call { ..}
                | Instr::Assert { ..}
                | Instr::Panic { ..}
                | Instr::Defer { ..}
                | Instr::RunDefers { ..} => return false,
                _ => {}
            }
            count += 1;
            if count > 16 {
                return false;
            }
        }
        match &block.term {
            Terminator::Ret(_)
            | Terminator::Br(_)
            | Terminator::BrIf { .. }
            | Terminator::Switch { .. } => {}
            _ => return false,
        }
    }
    true
}

fn remap_local(base: Local, local: Local) -> Local {
    base + local
}

fn remap_instr(ins: &Instr, base: Local) -> Instr {
    let r = |l: Local| remap_local(base, l);
    let rv = |v: &[Local]| v.iter().map(|l| r(*l)).collect::<Vec<Local>>();
    match ins {
        Instr::Const { span,  dst, lit } => Instr::Const { span: *span,  dst: r(*dst), lit: lit.clone() },
        Instr::Copy { span,  dst, src } => Instr::Copy { span: *span,  dst: r(*dst), src: r(*src) },
        Instr::Cast { span,  dst, src } => Instr::Cast { span: *span,  dst: r(*dst), src: r(*src) },
        Instr::Convert { span,  dst, src, kind } => Instr::Convert { span: *span,  dst: r(*dst), src: r(*src), kind: *kind },
        Instr::Fma { span,  dst, a, b, c } => Instr::Fma { span: *span,  dst: r(*dst), a: r(*a), b: r(*b), c: r(*c) },
        Instr::Arith { span,  op, kind, dst, lhs, rhs } => Instr::Arith { span: *span, 
            op: *op,
            kind: *kind,
            dst: r(*dst),
            lhs: r(*lhs),
            rhs: r(*rhs),
        },
        Instr::Cmp { span,  op, kind, dst, lhs, rhs } => Instr::Cmp { span: *span, 
            op: *op,
            kind: *kind,
            dst: r(*dst),
            lhs: r(*lhs),
            rhs: r(*rhs),
        },
        Instr::Not { span,  dst, src } => Instr::Not { span: *span,  dst: r(*dst), src: r(*src) },
        Instr::Neg { span,  kind, dst, src } => Instr::Neg { span: *span,  kind: *kind, dst: r(*dst), src: r(*src) },
        Instr::Concat { span,  dst, lhs, rhs } => Instr::Concat { span: *span,  dst: r(*dst), lhs: r(*lhs), rhs: r(*rhs) },
        Instr::ToStr { span,  dst, src } => Instr::ToStr { span: *span,  dst: r(*dst), src: r(*src) },
        Instr::Range { span,  dst, lo, hi, inclusive } => Instr::Range { span: *span, 
            dst: r(*dst),
            lo: r(*lo),
            hi: r(*hi),
            inclusive: *inclusive,
        },
        Instr::Stride { span,  dst, range, step } => Instr::Stride { span: *span, 
            dst: r(*dst),
            range: r(*range),
            step: r(*step),
        },
        Instr::ArrayNew { span,  dst, cap, elem_size } => Instr::ArrayNew { span: *span, 
            dst: r(*dst),
            cap: *cap,
            elem_size: *elem_size,
        },
        Instr::ArrayPush { span,  arr, value, elem_size } => Instr::ArrayPush { span: *span, 
            arr: r(*arr),
            value: r(*value),
            elem_size: *elem_size,
        },
        Instr::ArrayPop { span,  dst, arr } => Instr::ArrayPop { span: *span,  dst: r(*dst), arr: r(*arr) },
        Instr::ArrayLen { span,  dst, arr } => Instr::ArrayLen { span: *span,  dst: r(*dst), arr: r(*arr) },
        Instr::ArrayGet { span,  dst, arr, index, elem_size, unchecked } => Instr::ArrayGet { span: *span, 
            dst: r(*dst),
            arr: r(*arr),
            index: r(*index),
            elem_size: *elem_size,
            unchecked: *unchecked,
        },
        Instr::ArraySet { span,  arr, index, value, elem_size, unchecked } => Instr::ArraySet { span: *span, 
            arr: r(*arr),
            index: r(*index),
            value: r(*value),
            elem_size: *elem_size,
            unchecked: *unchecked,
        },
        Instr::ObjNew { span,  dst, class, instance_size } => Instr::ObjNew { span: *span, 
            dst: r(*dst),
            class: *class,
            instance_size: *instance_size,
        },
        Instr::StackAlloc { span,  dst, class, instance_size } => Instr::StackAlloc { span: *span, 
            dst: r(*dst),
            class: *class,
            instance_size: *instance_size,
        },
        Instr::EnumNew { span,  dst, enu, variant, payload } => Instr::EnumNew { span: *span, 
            dst: r(*dst),
            enu: *enu,
            variant: *variant,
            payload: rv(payload),
        },
        Instr::EnumPayload { span,  dst, scrut, index } => Instr::EnumPayload { span: *span, 
            dst: r(*dst),
            scrut: r(*scrut),
            index: *index,
        },
        Instr::EnumTag { span,  dst, scrut } => Instr::EnumTag { span: *span,  dst: r(*dst), scrut: r(*scrut) },
        Instr::Extract { span,  dst, base: b, index, field } => Instr::Extract { span: *span, 
            dst: r(*dst),
            base: r(*b),
            index: *index,
            field: field.clone(),
        },
        Instr::AddrOf { span,  dst, src } => Instr::AddrOf { span: *span,  dst: r(*dst), src: r(*src) },
        Instr::PtrLoad { span,  dst, ptr, volatile } => Instr::PtrLoad { span: *span,  dst: r(*dst), ptr: r(*ptr), volatile: *volatile },
        Instr::PtrStore { span,  ptr, val, volatile } => Instr::PtrStore { span: *span,  ptr: r(*ptr), val: r(*val), volatile: *volatile },
        Instr::RangeLo { span,  dst, range } => Instr::RangeLo { span: *span,  dst: r(*dst), range: r(*range) },
        Instr::RangeHi { span,  dst, range } => Instr::RangeHi { span: *span,  dst: r(*dst), range: r(*range) },
        Instr::RangeStep { span,  dst, range } => Instr::RangeStep { span: *span,  dst: r(*dst), range: r(*range) },
        Instr::GetField { span,  dst, obj, field } => Instr::GetField { span: *span, 
            dst: r(*dst),
            obj: r(*obj),
            field: *field,
        },
        Instr::GetFieldByName { span,  dst, obj, field } => Instr::GetFieldByName { span: *span, 
            dst: r(*dst),
            obj: r(*obj),
            field: field.clone(),
        },
        Instr::SetField { span,  obj, field, value } => Instr::SetField { span: *span, 
            obj: r(*obj),
            field: *field,
            value: r(*value),
        },
        Instr::SetFieldByName { span,  obj, field, value } => Instr::SetFieldByName { span: *span, 
            obj: r(*obj),
            field: field.clone(),
            value: r(*value),
        },
        Instr::ClosureNew { span,  dst, func, captures, decay, decay_this } => Instr::ClosureNew { span: *span, 
            dst: r(*dst),
            func: *func,
            captures: rv(captures),
            decay: *decay,
            decay_this: *decay_this,
        },
        Instr::Call { span,  dsts, err, target, args } => Instr::Call { span: *span, 
            dsts: rv(dsts),
            err: err.map(r),
            target: target.clone(),
            args: rv(args),
        },
        Instr::GenRefOf { span,  dst, obj } => Instr::GenRefOf { span: *span,  dst: r(*dst), obj: r(*obj) },
        Instr::GenRefEmpty { span,  dst } => Instr::GenRefEmpty { span: *span,  dst: r(*dst) },
        Instr::GenRefGet { span,  dst, gref } => Instr::GenRefGet { span: *span,  dst: r(*dst), gref: r(*gref) },
        Instr::GenRefInvalidate { span,  obj } => Instr::GenRefInvalidate { span: *span,  obj: r(*obj) },
        Instr::ThreadSpawn { span,  dst, func, closure, ret_tag } => Instr::ThreadSpawn { span: *span,  dst: r(*dst), func: *func, closure: closure.map(r), ret_tag: *ret_tag },
        Instr::ThreadJoin { span,  dst, handle } => Instr::ThreadJoin { span: *span,  dst: r(*dst), handle: r(*handle) },
        Instr::PoolInit { span,  dst, id, workers } => {
            Instr::PoolInit { span: *span,  dst: r(*dst), id: r(*id), workers: r(*workers) }
        }
        Instr::PoolSubmit { span,  dst, pool, func, arg, closure, ret_tag } => {
            Instr::PoolSubmit { span: *span,  dst: r(*dst), pool: r(*pool), func: *func, arg: arg.map(r), closure: closure.map(r), ret_tag: *ret_tag }
        }
        Instr::PoolParallelFor { span,  pool, start, end, chunk, func, closure } => Instr::PoolParallelFor { span: *span, 
            pool: r(*pool),
            start: r(*start),
            end: r(*end),
            chunk: r(*chunk),
            func: *func,
            closure: closure.map(r),
        },
        Instr::PoolJoin { span,  pool } => Instr::PoolJoin { span: *span,  pool: r(*pool) },
        Instr::PoolShutdown { span,  pool } => Instr::PoolShutdown { span: *span,  pool: r(*pool) },
        Instr::VecNew { span,  dst, kind, x, y, z, w } => Instr::VecNew { span: *span, 
            dst: r(*dst),
            kind: *kind,
            x: r(*x),
            y: r(*y),
            z: r(*z),
            w: r(*w),
        },
        Instr::VecSplat { span,  dst, kind, val } => {
            Instr::VecSplat { span: *span,  dst: r(*dst), kind: *kind, val: r(*val) }
        }
        Instr::VecExtract { span,  dst, vec, lane } => {
            Instr::VecExtract { span: *span,  dst: r(*dst), vec: r(*vec), lane: r(*lane) }
        }
        Instr::VecInsert { span,  dst, vec, lane, val } => Instr::VecInsert { span: *span, 
            dst: r(*dst),
            vec: r(*vec),
            lane: *lane,
            val: r(*val),
        },
        Instr::VecArith { span,  dst, op, kind, lhs, rhs } => Instr::VecArith { span: *span, 
            dst: r(*dst),
            op: *op,
            kind: *kind,
            lhs: r(*lhs),
            rhs: r(*rhs),
        },
        Instr::VecUnary { span,  dst, op, src } => {
            Instr::VecUnary { span: *span,  dst: r(*dst), op: *op, src: r(*src) }
        }
        Instr::VecDot { span,  dst, lhs, rhs } => {
            Instr::VecDot { span: *span,  dst: r(*dst), lhs: r(*lhs), rhs: r(*rhs) }
        }
        Instr::ReleaseField { span,  obj, field } => Instr::ReleaseField { span: *span,  obj: r(*obj), field: *field },
        Instr::Retain { span,  obj } => Instr::Retain { span: *span,  obj: r(*obj) },
        Instr::Release { span,  obj } => Instr::Release { span: *span,  obj: r(*obj) },
        Instr::ReleaseAs { span,  obj, class } => Instr::ReleaseAs { span: *span,  obj: r(*obj), class: *class },
        Instr::Defer { span,  body } => Instr::Defer { span: *span, 
            body: body.iter().map(|i| remap_instr(i, base)).collect(),
        },
        Instr::RunDefers { span,  keep } => Instr::RunDefers { span: *span,  keep: *keep },
        Instr::Assert { span,  cond, message } => Instr::Assert { span: *span,  cond: r(*cond), message: r(*message) },
        Instr::Panic { span,  message } => Instr::Panic { span: *span,  message: r(*message) },
    }
}

fn shift_term(term: &Terminator, block_base: BlockId, local_base: Local) -> Terminator {
    let rb = |b: BlockId| block_base + b;
    let rl = |l: Local| local_base + l;
    match term {
        Terminator::Ret(v) => Terminator::Ret(v.iter().map(|l| rl(*l)).collect()),
        Terminator::Br(t) => Terminator::Br(rb(*t)),
        Terminator::BrIf { span, cond, then_bb, else_bb } => Terminator::BrIf {
            span: *span,
            cond: rl(*cond),
            then_bb: rb(*then_bb),
            else_bb: rb(*else_bb),
        },
        Terminator::BrErr { span, err, catch_bb, catch_bind, next_bb, depth } => Terminator::BrErr {
            span: *span,
            err: rl(*err),
            catch_bb: rb(*catch_bb),
            catch_bind: rl(*catch_bind),
            next_bb: rb(*next_bb),
            depth: *depth,
        },
        Terminator::Switch { span, scrut, cases, default } => Terminator::Switch {
            span: *span,
            scrut: rl(*scrut),
            cases: cases.iter().map(|(p, b)| (p.clone(), rb(*b))).collect(),
            default: rb(*default),
        },
        Terminator::Throw { span, src, catch, .. } => Terminator::Throw {
            span: *span,
            src: rl(*src),
            catch: catch.map(|(b, l, d)| (rb(b), rl(l), d)),
        },
        Terminator::Rethrow { span, catch_bb, err, depth } => Terminator::Rethrow {
            span: *span,
            catch_bb: rb(*catch_bb),
            err: rl(*err),
            depth: *depth,
        },
        Terminator::Unreachable { span } => Terminator::Unreachable { span: *span },
    }
}

fn is_move_pair(param_ty: &LirType, arg_ty: &LirType) -> bool {
    matches!(
        (param_ty, arg_ty),
        (LirType::Obj(_), LirType::Obj(_))
            | (LirType::Array(_), LirType::Array(_))
            | (LirType::Enum(_), LirType::Enum(_))
    )
}

fn splice_call(
    module: &mut Module,
    caller: usize,
    bi: usize,
    ii: usize,
    callee: usize,
    dsts: &[Local],
    args: &[Local],
) {
    let callee_fn = module.functions[callee].clone();
    let caller_fn = &mut module.functions[caller];
    let base = caller_fn.locals.len() as Local;
    for ty in &callee_fn.locals {
        caller_fn.locals.push(ty.clone());
    }
    let slots = crate::instr::flat_sig(&callee_fn.ret);
    let mut res: Vec<Local> = Vec::new();
    if !dsts.is_empty() {
        for ty in &slots {
            let r = caller_fn.locals.len() as Local;
            caller_fn.locals.push(ty.clone());
            res.push(r);
        }
    }
    let mut tail = caller_fn.blocks[bi].instrs.split_off(ii);
    let call_span = tail.first().map(|ins| ins.span()).unwrap_or(UNKNOWN_SPAN);
    tail.remove(0);
    let orig_term = caller_fn.blocks[bi].term.clone();
    let cont_id = caller_fn.blocks.len();
    let mut cont_instrs = Vec::new();
    let mut restore: Vec<(Local, Local)> = Vec::new();
    for (p, a) in args.iter().enumerate() {
        let param_ty = callee_fn.params.get(p).cloned().unwrap_or(LirType::Any);
        let arg_ty = caller_fn.locals.get(*a as usize).cloned().unwrap_or(LirType::Any);
        if !is_move_pair(&param_ty, &arg_ty) || dsts.contains(a) {
            continue;
        }
        restore.push((*a, base + p as Local));
    }
    // Move-backs restore caller ownership moved into param slots. Dropping
    // them leaks (arg stays unowned); replacing them with Retain double-frees.
    for (a, p) in &restore {
        cont_instrs.push(Instr::Copy { span: call_span, dst: *a, src: *p });
    }
    for (d, r) in dsts.iter().zip(res.iter()) {
        cont_instrs.push(Instr::Copy { span: call_span, dst: *d, src: *r });
    }
    cont_instrs.extend(tail);
    caller_fn.blocks.push(Block { instrs: cont_instrs, term: orig_term });
    for (p, a) in args.iter().enumerate() {
        caller_fn.blocks[bi].instrs.push(Instr::Copy {
            span: call_span,
            dst: base + p as Local,
            src: *a,
        });
    }
    let entry_id = caller_fn.blocks.len();
    caller_fn.blocks[bi].term = Terminator::Br(entry_id);
    for cb in &callee_fn.blocks {
        let mut instrs: Vec<Instr> =
            cb.instrs.iter().map(|ins| remap_instr(ins, base)).collect();
        let term = match &cb.term {
            Terminator::Ret(v) => {
                for (r, val) in res.iter().zip(v.iter()) {
                    instrs.push(Instr::Copy { span: call_span, dst: *r, src: remap_local(base, *val) });
                }
                Terminator::Br(cont_id)
            }
            _ => shift_term(&cb.term, entry_id, base),
        };
        caller_fn.blocks.push(Block { instrs, term });
    }
}

pub fn inline_call_sites(module: &mut Module) -> bool {
    let candidates: BTreeSet<usize> = (0..module.functions.len())
        .filter(|i| is_inline_candidate(&module.functions[*i]))
        .collect();
    if candidates.is_empty() {
        return false;
    }
    let mut changed = false;
    let mut fi = 0;
    while fi < module.functions.len() {
        if candidates.contains(&fi) {
            fi += 1;
            continue;
        }
        let mut bi = 0;
        while bi < module.functions[fi].blocks.len() {
            let mut ii = 0;
            while ii < module.functions[fi].blocks[bi].instrs.len() {
                let site = match &module.functions[fi].blocks[bi].instrs[ii] {
                    Instr::Call {  dsts, err: None, target, args , ..} => {
                        let callee = match target {
                            CallTarget::Fn(id) => Some(*id),
                            CallTarget::Method { method, .. } => Some(*method),
                            _ => None,
                        };
                        match callee {
                            Some(id)
                                if id != fi
                                    && id < module.functions.len()
                                    && candidates.contains(&id)
                                    && args.len() == module.functions[id].params.len()
                                    && (dsts.is_empty()
                                        || dsts.len()
                                            == crate::instr::flat_sig(
                                                &module.functions[id].ret,
                                            )
                                            .len()) =>
                            {
                                Some((id, dsts.clone(), args.clone()))
                            }
                            _ => None,
                        }
                    }
                    _ => None,
                };
                match site {
                    Some((callee, dsts, args)) => {
                        let callee_params = module.functions[callee].params.clone();
                        let caller_locals = module.functions[fi].locals.clone();
                        let mut seen: BTreeSet<Local> = BTreeSet::new();
                        let mut aliased = false;
                        for (p, a) in args.iter().enumerate() {
                            let pt =
                                callee_params.get(p).cloned().unwrap_or(LirType::Any);
                            let at =
                                caller_locals.get(*a as usize).cloned().unwrap_or(LirType::Any);
                            if !is_move_pair(&pt, &at) {
                                continue;
                            }
                            // Same local twice would double-own one reference.
                            if !seen.insert(*a) {
                                aliased = true;
                                break;
                            }
                        }
                        if aliased {
                            ii += 1;
                            continue;
                        }
                        splice_call(module, fi, bi, ii, callee, &dsts, &args);
                        changed = true;
                    }
                    None => ii += 1,
                }
            }
            bi += 1;
        }
        fi += 1;
    }
    changed
}

fn mark_one(
    reachable: &mut BTreeSet<usize>,
    worklist: &mut Vec<usize>,
    nfuncs: usize,
    id: usize,
) {
    if id < nfuncs && reachable.insert(id) {
        worklist.push(id);
    }
}

fn mark_targets(
    ins: &Instr,
    reachable: &mut BTreeSet<usize>,
    worklist: &mut Vec<usize>,
    nfuncs: usize,
) {
    match ins {
        Instr::Call {  target , ..} => match target {
            CallTarget::Fn(id) => mark_one(reachable, worklist, nfuncs, *id),
            CallTarget::Method { method, .. } => mark_one(reachable, worklist, nfuncs, *method),
            _ => {}
        },
        Instr::ThreadSpawn {  func , ..} => mark_one(reachable, worklist, nfuncs, *func),
        Instr::PoolSubmit {  func , ..} | Instr::PoolParallelFor { func , ..} => {
            mark_one(reachable, worklist, nfuncs, *func)
        }
        Instr::ClosureNew {  func , ..} => mark_one(reachable, worklist, nfuncs, *func),
        Instr::Defer {  body , ..} => {
            for nested in body {
                mark_targets(nested, reachable, worklist, nfuncs);
            }
        }
        _ => {}
    }
}

fn remap_targets(ins: &mut Instr, map: &[usize]) {
    match ins {
        Instr::Call {  target , ..} => match target {
            CallTarget::Fn(id) => *id = map[*id],
            CallTarget::Method { method, .. } => *method = map[*method],
            _ => {}
        },
        Instr::ThreadSpawn {  func , ..} => {
            if *func != usize::MAX {
                *func = map[*func];
            }
        }
        Instr::PoolSubmit {  func , ..} | Instr::PoolParallelFor { func , ..} => {
            if *func != usize::MAX {
                *func = map[*func];
            }
        }
        Instr::ClosureNew {  func , ..} => *func = map[*func],
        Instr::Defer {  body , ..} => {
            for nested in body {
                remap_targets(nested, map);
            }
        }
        _ => {}
    }
}

fn collect_type_refs(
    module: &Module,
    ty: &LirType,
    classes: &mut BTreeSet<usize>,
    enums: &mut BTreeSet<usize>,
    arrays: &mut BTreeSet<String>,
) {
    match ty {
        LirType::Obj(n) => {
            if let Some(&ci) = module.class_index.get(n) {
                classes.insert(ci);
            }
        }
        LirType::Enum(ei) => {
            if *ei < module.enums.len() {
                enums.insert(*ei);
            }
        }
        LirType::Array(inner) => {
            arrays.insert(type_key(inner));
            collect_type_refs(module, inner, classes, enums, arrays);
        }
        LirType::Tuple(items) => {
            for t in items {
                collect_type_refs(module, t, classes, enums, arrays);
            }
        }
        LirType::GenRef(Some(n)) => {
            if let Some(&ci) = module.class_index.get(n) {
                classes.insert(ci);
            }
        }
        _ => {}
    }
}

fn scan_reachable_fn(
    module: &Module,
    fi: usize,
    classes: &mut BTreeSet<usize>,
    enums: &mut BTreeSet<usize>,
    arrays: &mut BTreeSet<String>,
    dyn_names: &mut BTreeSet<String>,
) {
    let f = &module.functions[fi];
    for t in f.params.iter().chain(&f.sig_params).chain(std::iter::once(&f.ret)).chain(&f.locals) {
        collect_type_refs(module, t, classes, enums, arrays);
    }
    for block in &f.blocks {
        for ins in &block.instrs {
            match ins {
                Instr::ObjNew { class, .. } => {
                    if *class < module.classes.len() {
                        classes.insert(*class);
                    }
                }
                Instr::EnumNew { enu, .. } => {
                    if *enu < module.enums.len() {
                        enums.insert(*enu);
                    }
                }
                Instr::Call { target: CallTarget::Method { class, .. }, .. } => {
                    if *class < module.classes.len() {
                        classes.insert(*class);
                    }
                }
                Instr::Call { target: CallTarget::Dyn { obj, method }, .. } => {
                    dyn_names.insert(method.clone());
                    if let Some(LirType::Obj(n)) = f.locals.get(*obj as usize) {
                        if let Some(&ci) = module.class_index.get(n) {
                            classes.insert(ci);
                        }
                    }
                }
                _ => {}
            }
        }
        if let Terminator::Switch { cases, .. } = &block.term {
            for (pat, _) in cases {
                match pat {
                    SwitchPat::Enum { enu, .. } => {
                        if *enu < module.enums.len() {
                            enums.insert(*enu);
                        }
                    }
                    SwitchPat::Is { tag, .. } => {
                        if let Some(&ci) = module.class_index.get(tag) {
                            classes.insert(ci);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

pub fn eliminate_dead_functions(module: &mut Module, entry: &str, is_library: bool) {
    let mut reachable: BTreeSet<usize> = BTreeSet::new();
    let mut worklist: Vec<usize> = Vec::new();
    let nfuncs = module.functions.len();
    if is_library {
        for (i, f) in module.functions.iter().enumerate() {
            if f.is_pub {
                mark_one(&mut reachable, &mut worklist, nfuncs, i);
            }
        }
    } else {
        let entry_id = match module.fn_id(entry) {
            Some(id) => id,
            None => return,
        };
        mark_one(&mut reachable, &mut worklist, nfuncs, entry_id);
    }
    // Methods, deinits, dtors, and array dtors survive only through the
    // types reachable functions actually reference. Drop glue (interpreter
    // `machine.rs` drop paths, JIT `dtor_addr` family) resolves destructors
    // through class/enum metadata, so a pruned type's metadata is cleared in
    // the sweep below; a type no reachable instruction references can never
    // produce a value that reaches those paths.
    let mut scanned_fns: BTreeSet<usize> = BTreeSet::new();
    let mut scanned_classes: BTreeSet<usize> = BTreeSet::new();
    let mut scanned_enums: BTreeSet<usize> = BTreeSet::new();
    let mut ref_classes: BTreeSet<usize> = BTreeSet::new();
    let mut ref_enums: BTreeSet<usize> = BTreeSet::new();
    let mut ref_arrays: BTreeSet<String> = BTreeSet::new();
    let mut dyn_names: BTreeSet<String> = BTreeSet::new();
    loop {
        let marked = reachable.len();
        while let Some(fi) = worklist.pop() {
            for block in &module.functions[fi].blocks {
                for ins in &block.instrs {
                    mark_targets(ins, &mut reachable, &mut worklist, nfuncs);
                }
            }
        }
        let pending: Vec<usize> =
            reachable.difference(&scanned_fns).copied().collect();
        for fi in pending {
            scanned_fns.insert(fi);
            scan_reachable_fn(
                module,
                fi,
                &mut ref_classes,
                &mut ref_enums,
                &mut ref_arrays,
                &mut dyn_names,
            );
        }
        let pending_classes: Vec<usize> =
            ref_classes.difference(&scanned_classes).copied().collect();
        for ci in pending_classes {
            scanned_classes.insert(ci);
            let class = &module.classes[ci];
            if let Some(parent) = class.parent {
                if parent < module.classes.len() {
                    ref_classes.insert(parent);
                }
            }
            for fld in &class.fields {
                collect_type_refs(module, &fld.ty, &mut ref_classes, &mut ref_enums, &mut ref_arrays);
            }
        }
        let pending_enums: Vec<usize> =
            ref_enums.difference(&scanned_enums).copied().collect();
        for ei in pending_enums {
            scanned_enums.insert(ei);
            for v in &module.enums[ei].variants {
                for t in &v.payload {
                    collect_type_refs(module, t, &mut ref_classes, &mut ref_enums, &mut ref_arrays);
                }
            }
        }
        for &ci in &ref_classes {
            let class = &module.classes[ci];
            if let Some(id) = class.deinit {
                mark_one(&mut reachable, &mut worklist, nfuncs, id);
            }
            if let Some(id) = class.dtor {
                mark_one(&mut reachable, &mut worklist, nfuncs, id);
            }
            for m in &dyn_names {
                if let Some(mr) = class.methods.get(m) {
                    mark_one(&mut reachable, &mut worklist, nfuncs, mr.id);
                }
            }
        }
        for &ei in &ref_enums {
            if let Some(id) = module.enums[ei].dtor {
                mark_one(&mut reachable, &mut worklist, nfuncs, id);
            }
        }
        for key in &ref_arrays {
            if let Some(&id) = module.array_dtors.get(key) {
                mark_one(&mut reachable, &mut worklist, nfuncs, id);
            }
        }
        if reachable.len() == marked && worklist.is_empty() {
            break;
        }
    }
    if reachable.len() == module.functions.len() {
        return;
    }
    let mut map = vec![usize::MAX; module.functions.len()];
    let mut kept: Vec<Function> = Vec::new();
    for (i, func) in module.functions.drain(..).enumerate() {
        if reachable.contains(&i) {
            map[i] = kept.len();
            kept.push(func);
        }
    }
    module.functions = kept;
    for func in &mut module.functions {
        for block in &mut func.blocks {
            for ins in &mut block.instrs {
                remap_targets(ins, &map);
            }
        }
    }
    for class in &mut module.classes {
        class.methods.retain(|_, m| map[m.id] != usize::MAX);
        for method in class.methods.values_mut() {
            method.id = map[method.id];
        }
        for slot in [&mut class.deinit, &mut class.dtor] {
            *slot = slot.and_then(|id| {
                let mapped = map[id];
                (mapped != usize::MAX).then_some(mapped)
            });
        }
    }
    for enu in &mut module.enums {
        enu.dtor = enu.dtor.and_then(|id| {
            let mapped = map[id];
            (mapped != usize::MAX).then_some(mapped)
        });
    }
    module.array_dtors.retain(|_, id| map[*id] != usize::MAX);
    for id in module.array_dtors.values_mut() {
        *id = map[*id];
    }
    module.fn_index.clear();
    for (i, func) in module.functions.iter().enumerate() {
        module.fn_index.insert(func.name.clone(), i);
    }
}

#[cfg(test)]
mod dce_tests {
    use super::*;

    fn func(name: &str, calls: &[usize]) -> Function {
        let instrs: Vec<Instr> = calls
            .iter()
            .map(|id| Instr::Call {span: UNKNOWN_SPAN, 
                dsts: vec![],
                err: None,
                target: CallTarget::Fn(*id),
                args: Vec::new(),
            })
            .collect();
        Function {
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
            blocks: vec![Block { instrs, term: Terminator::Ret(vec![]) }],
        }
    }

    fn indexed(functions: Vec<Function>) -> Module {
        let mut module = Module { functions, ..Default::default() };
        module.fn_index.clear();
        for (i, f) in module.functions.iter().enumerate() {
            module.fn_index.insert(f.name.clone(), i);
        }
        module
    }

    fn names(module: &Module) -> Vec<String> {
        module.functions.iter().map(|f| f.name.clone()).collect()
    }

    #[test]
    fn keeps_transitive_chain_drops_orphan() {
        let mut module = indexed(vec![
            func("Main", &[1]),
            func("A", &[2]),
            func("B", &[]),
            func("orphan", &[]),
        ]);
        eliminate_dead_functions(&mut module, "Main", false);
        assert_eq!(names(&module), vec!["Main", "A", "B"]);
        assert_eq!(module.fn_index.get("B"), Some(&2));
    }

    #[test]
    fn drops_unreferenced_class_roots() {
        // No instruction references K: its method, deinit, and dtor are dead.
        let mut module = indexed(vec![func("Main", &[]), func("hidden", &[]), func("kd", &[])]);
        module.class_index.insert("K".to_string(), 0);
        module.classes.push(ClassDesc {
            name: "K".to_string(),
            type_params: Vec::new(),
            is_struct: false,
            fields: Vec::new(),
            field_index: BTreeMap::new(),
            methods: BTreeMap::from([(
                "m".to_string(),
                MethodRef { id: 1, private: false, owner: "K".to_string() },
            )]),
            deinit: Some(2),
            dtor: Some(2),
            ifaces: Vec::new(),
            parent: None,
        });
        eliminate_dead_functions(&mut module, "Main", false);
        assert_eq!(names(&module), vec!["Main"]);
        assert!(module.classes[0].methods.is_empty());
        assert_eq!(module.classes[0].deinit, None);
        assert_eq!(module.classes[0].dtor, None);
    }

    fn obj_user(class: usize) -> Function {
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
            locals: vec![LirType::Obj("K".to_string())],
            blocks: vec![Block {
                instrs: vec![
                    Instr::ObjNew { span: UNKNOWN_SPAN, dst: 0, class, instance_size: 8 },
                    Instr::Call { span: UNKNOWN_SPAN,
                        dsts: vec![],
                        err: None,
                        target: CallTarget::Dyn { obj: 0, method: "m".to_string() },
                        args: Vec::new(),
                    },
                ],
                term: Terminator::Ret(vec![]),
            }],
        }
    }

    #[test]
    fn keeps_referenced_type_dtors_and_dyn_methods() {
        // Main instantiates K and dynamically calls `m`: dtor, deinit, and
        // `m` survive, the never-called `other` does not.
        let mut module = indexed(vec![
            obj_user(0),
            func("K.m", &[]),
            func("K.other", &[]),
            func("K.deinit", &[]),
            func("K.dtor", &[]),
        ]);
        module.class_index.insert("K".to_string(), 0);
        module.classes.push(ClassDesc {
            name: "K".to_string(),
            type_params: Vec::new(),
            is_struct: false,
            fields: Vec::new(),
            field_index: BTreeMap::new(),
            methods: BTreeMap::from([
                ("m".to_string(), MethodRef { id: 1, private: false, owner: "K".to_string() }),
                ("other".to_string(), MethodRef { id: 2, private: false, owner: "K".to_string() }),
            ]),
            deinit: Some(3),
            dtor: Some(4),
            ifaces: Vec::new(),
            parent: None,
        });
        eliminate_dead_functions(&mut module, "Main", false);
        assert_eq!(names(&module), vec!["Main", "K.m", "K.deinit", "K.dtor"]);
        assert_eq!(module.classes[0].methods.len(), 1);
        assert!(module.classes[0].dtor.is_some());
    }

    #[test]
    fn keeps_implicit_roots_and_library() {
        // K is never referenced, so its method no longer roots `hidden`.
        let mut module = indexed(vec![func("Main", &[]), func("hidden", &[])]);
        module.classes.push(ClassDesc {
            name: "K".to_string(),
            type_params: Vec::new(),
            is_struct: false,
            fields: Vec::new(),
            field_index: BTreeMap::new(),
            methods: BTreeMap::from([(
                "m".to_string(),
                MethodRef { id: 1, private: false, owner: "C".to_string() },
            )]),
            deinit: None,
            dtor: None,
            ifaces: Vec::new(),
            parent: None,
        });
        eliminate_dead_functions(&mut module, "Main", false);
        assert_eq!(names(&module), vec!["Main"]);
        assert!(module.classes[0].methods.is_empty());
        let mut module = indexed(vec![func("Main", &[]), func("hidden", &[])]);
        module.functions[0].is_pub = true;
        eliminate_dead_functions(&mut module, "Main", true);
        assert_eq!(names(&module), vec!["Main"]);
        let mut module = indexed(vec![func("Main", &[]), func("hidden", &[])]);
        eliminate_dead_functions(&mut module, "Missing", false);
        assert_eq!(names(&module).len(), 2);
    }
}

pub fn single_exit(func: &Function) -> Function {
    // One return site keeps backend owned-set tracking complete; extra
    // returns lowered earlier would otherwise release an incomplete set.
    let ret_blocks: Vec<usize> = func
        .blocks
        .iter()
        .enumerate()
        .filter(|(_, b)| matches!(b.term, Terminator::Ret(_)))
        .map(|(i, _)| i)
        .collect();
    if ret_blocks.len() < 2 {
        return func.clone();
    }
    let ret_vals: Vec<Vec<Local>> = ret_blocks
        .iter()
        .map(|&bi| match &func.blocks[bi].term {
            Terminator::Ret(v) => v.clone(),
            _ => Vec::new(),
        })
        .collect();
    let width = ret_vals.first().map(|v| v.len()).unwrap_or(0);
    if !ret_vals.iter().all(|v| v.len() == width) {
        return func.clone();
    }
    let mut out = func.clone();
    let mut slots: Vec<Local> = Vec::new();
    if width > 0 {
        let first = &ret_vals[0];
        let sig = crate::instr::flat_sig(&func.ret);
        for (i, val) in first.iter().enumerate() {
            let ty = match sig.get(i).cloned() {
                Some(t) if !matches!(t, LirType::Any) => t,
                _ => {
                    let vt = out.locals.get(*val as usize).cloned().unwrap_or(LirType::Any);
                    if matches!(vt, LirType::Null) {
                        LirType::Any
                    } else {
                        vt
                    }
                }
            };
            out.locals.push(ty);
            slots.push(out.locals.len() as Local - 1);
        }
    }
    let exit_id = out.blocks.len();
    for (bi, vals) in ret_blocks.iter().zip(ret_vals.iter()) {
        for (s, val) in slots.iter().zip(vals.iter()) {
            out.blocks[*bi].instrs.push(Instr::Copy {span: UNKNOWN_SPAN,  dst: *s, src: *val });
        }
        out.blocks[*bi].term = Terminator::Br(exit_id);
    }
    out.blocks.push(Block { instrs: Vec::new(), term: Terminator::Ret(slots) });
    out
}

#[cfg(test)]
mod single_exit_tests {
    use super::*;

    fn retty(name: &str, rets: Vec<Vec<Local>>) -> Function {
        let mut f = Function {
            name: name.to_string(),
            params: Vec::new(),
            sig_params: Vec::new(),
            ret: LirType::I64,
            throws: false,
            is_unsafe: false,
            method_self: false,
            is_pub: false,
            is_closure: false,
            locals: vec![LirType::I64, LirType::I64],
            blocks: Vec::new(),
        };
        for r in rets {
            f.blocks.push(Block { instrs: Vec::new(), term: Terminator::Ret(r) });
        }
        f
    }

    #[test]
    fn merges_two_returns() {
        let f = retty("F", vec![vec![0], vec![1]]);
        let out = single_exit(&f);
        let rets = out
            .blocks
            .iter()
            .filter(|b| matches!(b.term, Terminator::Ret(_)))
            .count();
        assert_eq!(rets, 1);
        assert!(matches!(out.blocks.last().unwrap().term, Terminator::Ret(_)));
        assert!(out.blocks.iter().take(2).all(|b| matches!(b.term, Terminator::Br(2))));
        assert!(out.blocks.iter().take(2).all(|b| b.instrs.len() == 1));
    }

    #[test]
    fn single_return_unchanged() {
        let f = retty("F", vec![vec![0]]);
        let out = single_exit(&f);
        assert_eq!(out.blocks.len(), f.blocks.len());
    }
}

pub fn rpo_order(func: &Function) -> Vec<BlockId> {
    // Backends track ownership flow-insensitively, so blocks must lower in
    // execution order. Numeric order breaks on backward control edges.
    let n = func.blocks.len();
    let mut pushed = vec![false; n];
    let mut seen = vec![false; n];
    let mut post = Vec::with_capacity(n);
    let mut stack = Vec::new();
    if n > 0 {
        pushed[0] = true;
        stack.push((0usize, false));
    }
    while let Some((b, expanded)) = stack.pop() {
        if b >= n || seen[b] {
            continue;
        }
        if expanded {
            seen[b] = true;
            post.push(b);
            continue;
        }
        stack.push((b, true));
        for t in term_targets(&func.blocks[b].term) {
            if t < n && !pushed[t] {
                pushed[t] = true;
                stack.push((t, false));
            }
        }
    }
    post.reverse();
    for (i, s) in seen.iter().enumerate() {
        if !s {
            post.push(i);
        }
    }
    post
}

#[cfg(test)]
mod rpo_tests {
    use super::*;

    fn branched() -> Function {
        let block = |term| Block { instrs: Vec::new(), term };
        Function {
            name: "F".to_string(),
            params: Vec::new(),
            sig_params: Vec::new(),
            ret: LirType::I64,
            throws: false,
            is_unsafe: false,
            method_self: false,
            is_pub: false,
            is_closure: false,
            locals: Vec::new(),
            blocks: vec![
                block(Terminator::Br(2)),
                block(Terminator::Ret(vec![])),
                block(Terminator::BrIf {span: UNKNOWN_SPAN,  cond: 0, then_bb: 3, else_bb: 1 }),
                block(Terminator::Br(1)),
            ],
        }
    }

    fn loopy() -> Function {
        let block = |term| Block { instrs: Vec::new(), term };
        Function {
            name: "G".to_string(),
            params: Vec::new(),
            sig_params: Vec::new(),
            ret: LirType::I64,
            throws: false,
            is_unsafe: false,
            method_self: false,
            is_pub: false,
            is_closure: false,
            locals: Vec::new(),
            blocks: vec![
                block(Terminator::Br(1)),
                block(Terminator::BrIf {span: UNKNOWN_SPAN,  cond: 0, then_bb: 1, else_bb: 2 }),
                block(Terminator::Ret(vec![])),
            ],
        }
    }

    #[test]
    fn terminates_on_cycles() {
        let order = rpo_order(&loopy());
        assert_eq!(order.len(), 3);
        assert_eq!(order[0], 0);
    }

    #[test]
    fn respects_control_flow() {
        let order = rpo_order(&branched());
        assert_eq!(order[0], 0);
        let pos = |b: usize| order.iter().position(|x| *x == b).unwrap();
        assert!(pos(2) < pos(1));
        assert!(pos(2) < pos(3));
        assert!(pos(3) < pos(1));
        assert_eq!(order.len(), 4);
    }
}
