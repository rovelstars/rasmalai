use crate::instr::*;
use crate::opt::{instr_dst, instr_reads, term_reads};
use std::collections::{BTreeMap, BTreeSet};

pub struct DominatorTree {
    dom: Vec<BTreeSet<BlockId>>,
}

impl DominatorTree {
    pub fn dominates(&self, a: BlockId, b: BlockId) -> bool {
        self.dom.get(b).map(|s| s.contains(&a)).unwrap_or(false)
    }
}

pub fn compute_dominators(func: &Function) -> DominatorTree {
    let n = func.blocks.len();
    let mut preds: Vec<Vec<BlockId>> = vec![Vec::new(); n];
    for (i, block) in func.blocks.iter().enumerate() {
        for t in term_targets(&block.term) {
            if t < n {
                preds[t].push(i);
            }
        }
    }
    let all: BTreeSet<BlockId> = (0..n).collect();
    let mut dom: Vec<BTreeSet<BlockId>> = vec![all; n];
    if n > 0 {
        dom[0] = BTreeSet::from([0]);
    }
    let mut changed = true;
    while changed {
        changed = false;
        for b in 1..n {
            let mut next: Option<BTreeSet<BlockId>> = None;
            for p in &preds[b] {
                next = Some(match next {
                    Some(acc) => acc.intersection(&dom[*p]).copied().collect(),
                    None => dom[*p].clone(),
                });
            }
            let mut next = next.unwrap_or_default();
            next.insert(b);
            if next != dom[b] {
                dom[b] = next;
                changed = true;
            }
        }
    }
    DominatorTree { dom }
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

#[derive(Clone, Debug)]
pub struct NaturalLoop {
    pub header: BlockId,
    pub blocks: BTreeSet<BlockId>,
}

pub fn find_natural_loops(func: &Function, dom: &DominatorTree) -> Vec<NaturalLoop> {
    let n = func.blocks.len();
    let mut preds: Vec<Vec<BlockId>> = vec![Vec::new(); n];
    for (i, block) in func.blocks.iter().enumerate() {
        for t in term_targets(&block.term) {
            if t < n {
                preds[t].push(i);
            }
        }
    }
    let mut by_header: BTreeMap<BlockId, Vec<BlockId>> = BTreeMap::new();
    for (latch, block) in func.blocks.iter().enumerate() {
        for t in term_targets(&block.term) {
            if t < n && dom.dominates(t, latch) {
                by_header.entry(t).or_default().push(latch);
            }
        }
    }
    let mut loops: Vec<NaturalLoop> = Vec::new();
    for (header, latches) in by_header {
        let mut blocks: BTreeSet<BlockId> = BTreeSet::from([header]);
        let mut stack: Vec<BlockId> = Vec::new();
        for latch in latches {
            if blocks.insert(latch) {
                stack.push(latch);
            }
        }
        while let Some(b) = stack.pop() {
            for p in &preds[b] {
                if blocks.insert(*p) {
                    stack.push(*p);
                }
            }
        }
        loops.push(NaturalLoop { header, blocks });
    }
    loops.sort_by_key(|l| l.blocks.len());
    loops
}

fn loop_defs(func: &Function, blocks: &BTreeSet<BlockId>) -> BTreeMap<Local, usize> {
    let mut defs: BTreeMap<Local, usize> = BTreeMap::new();
    for b in blocks {
        for ins in &func.blocks[*b].instrs {
            if let Some(dst) = instr_dst(ins) {
                *defs.entry(dst).or_default() += 1;
            }
            if let Instr::Call {  dsts, err , ..} = ins {
                for d in dsts {
                    *defs.entry(*d).or_default() += 1;
                }
                if let Some(e) = err {
                    *defs.entry(*e).or_default() += 1;
                }
            }
            if let Instr::Defer {  body , ..} = ins {
                for nested in body {
                    if let Some(dst) = instr_dst(nested) {
                        *defs.entry(dst).or_default() += 1;
                    }
                }
            }
        }
        if let Terminator::BrErr { catch_bind, .. } = &func.blocks[*b].term {
            *defs.entry(*catch_bind).or_default() += 1;
        }
    }
    defs
}

fn divisor_nonzero(func: &Function, blocks: &BTreeSet<BlockId>, rhs: Local) -> bool {
    let mut count = 0;
    let mut nonzero = false;
    for (bi, block) in func.blocks.iter().enumerate() {
        for ins in &block.instrs {
            if instr_dst(ins) == Some(rhs) {
                count += 1;
                if !blocks.contains(&bi) {
                    if let Instr::Const {  lit: Lit::Int(n) , ..} = ins {
                        if *n != 0 {
                            nonzero = true;
                        }
                    }
                }
            }
        }
    }
    count == 1 && nonzero
}

fn invariant_candidates(
    func: &Function,
    blocks: &BTreeSet<BlockId>,
    defs: &BTreeMap<Local, usize>,
) -> Vec<(BlockId, usize)> {
    let mut marked: BTreeSet<(BlockId, usize)> = BTreeSet::new();
    let mut changed = true;
    while changed {
        changed = false;
        for b in blocks {
            for (i, ins) in func.blocks[*b].instrs.iter().enumerate() {
                if marked.contains(&(*b, i)) {
                    continue;
                }
                if is_invariant(func, blocks, defs, &marked, *b, i, ins) {
                    marked.insert((*b, i));
                    changed = true;
                }
            }
        }
    }
    marked.into_iter().collect()
}

fn operands_defined_outside(
    func: &Function,
    blocks: &BTreeSet<BlockId>,
    defs: &BTreeMap<Local, usize>,
    marked: &BTreeSet<(BlockId, usize)>,
    bi: BlockId,
    ii: usize,
    operands: &[Local],
) -> bool {
    for o in operands {
        if defs.get(o).copied().unwrap_or(0) > 0 {
            let mut ok = false;
            for b in blocks {
                for (i, ins) in func.blocks[*b].instrs.iter().enumerate() {
                    if instr_dst(ins) == Some(*o) && marked.contains(&(*b, i)) {
                        if (*b, i) < (bi, ii) {
                            ok = true;
                        }
                    }
                }
            }
            if !ok {
                return false;
            }
        }
    }
    true
}

fn is_invariant(
    func: &Function,
    blocks: &BTreeSet<BlockId>,
    defs: &BTreeMap<Local, usize>,
    marked: &BTreeSet<(BlockId, usize)>,
    bi: BlockId,
    ii: usize,
    ins: &Instr,
) -> bool {
    match ins {
        Instr::Const { dst, ..} => defs.get(dst).copied().unwrap_or(0) == 1,
        Instr::Copy {  dst, src , ..} => {
            defs.get(dst).copied().unwrap_or(0) <= 1
                && operands_defined_outside(func, blocks, defs, marked, bi, ii, &[*src])
        }
        Instr::Arith {  op, dst, lhs, rhs , ..} => {
            if defs.get(dst).copied().unwrap_or(0) != 1 {
                return false;
            }
            if matches!(op, ArithOp::Div | ArithOp::Mod) && !divisor_nonzero(func, blocks, *rhs)
            {
                return false;
            }
            operands_defined_outside(func, blocks, defs, marked, bi, ii, &[*lhs, *rhs])
        }
        _ => false,
    }
}

fn rewrite_target(term: &mut Terminator, from: BlockId, to: BlockId) -> bool {
    crate::opt::rewrite_target(term, from, to)
}

fn hoist_block(
    func: &mut Function,
    preds: &[Vec<BlockId>],
    header: BlockId,
    blocks: &BTreeSet<BlockId>,
) -> Option<BlockId> {
    let mut external: Vec<BlockId> = preds[header]
        .iter()
        .copied()
        .filter(|p| !blocks.contains(p))
        .collect();
    external.sort();
    external.dedup();
    if external.is_empty() {
        return None;
    }
    if external.len() == 1 {
        let p = external[0];
        let dedicated = match &func.blocks[p].term {
            Terminator::Br(t) => *t == header,
            _ => false,
        };
        if dedicated {
            return Some(p);
        }
    }
    if header == 0 {
        return None;
    }
    let pre = func.blocks.len();
    func.blocks.push(Block { instrs: Vec::new(), term: Terminator::Br(header) });
    for p in external {
        rewrite_target(&mut func.blocks[p].term, header, pre);
    }
    Some(pre)
}

fn uses_of(func: &Function, blocks: &BTreeSet<BlockId>, local: Local) -> Vec<BlockId> {
    let mut out: Vec<BlockId> = Vec::new();
    for b in blocks {
        let mut found = false;
        for ins in &func.blocks[*b].instrs {
            let mut tmp: Vec<Local> = Vec::new();
            instr_reads(ins, &mut tmp);
            if tmp.contains(&local) {
                found = true;
                break;
            }
        }
        if !found {
            let mut tmp: Vec<Local> = Vec::new();
            term_reads(&func.blocks[*b].term, &mut tmp);
            if tmp.contains(&local) {
                found = true;
            }
        }
        if found {
            out.push(*b);
        }
    }
    out
}

pub fn loop_invariant_code_motion(func: &mut Function) -> bool {
    let dom = compute_dominators(func);
    let loops = find_natural_loops(func, &dom);
    if loops.is_empty() {
        return false;
    }
    let mut changed = false;
    for lp in loops {
        if hoist_loop(func, &dom, &lp) {
            changed = true;
        }
    }
    changed
}

fn defined_outside(func: &Function, blocks: &BTreeSet<BlockId>) -> BTreeSet<Local> {
    let mut out: BTreeSet<Local> = BTreeSet::new();
    for p in 0..func.params.len() {
        out.insert(p as Local);
    }
    for (bi, block) in func.blocks.iter().enumerate() {
        if blocks.contains(&bi) {
            continue;
        }
        for ins in &block.instrs {
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
                    if let Some(dst) = instr_dst(nested) {
                        out.insert(dst);
                    }
                }
            }
        }
        if let Terminator::BrErr { catch_bind, .. } = &block.term {
            out.insert(*catch_bind);
        }
    }
    out
}

fn hoist_loop(func: &mut Function, dom: &DominatorTree, lp: &NaturalLoop) -> bool {
    let n = func.blocks.len();
    let mut preds: Vec<Vec<BlockId>> = vec![Vec::new(); n];
    for (i, block) in func.blocks.iter().enumerate() {
        for t in term_targets(&block.term) {
            if t < n {
                preds[t].push(i);
            }
        }
    }
    let pre = match hoist_block(func, &preds, lp.header, &lp.blocks) {
        Some(p) => p,
        None => return false,
    };
    let defs = loop_defs(func, &lp.blocks);
    let outside = defined_outside(func, &lp.blocks);
    let candidates = invariant_candidates(func, &lp.blocks, &defs);
    if candidates.is_empty() {
        return false;
    }
    let mut hoisted: Vec<(BlockId, usize)> = Vec::new();
    for (bi, ii) in candidates {
        let dst = match instr_dst(&func.blocks[bi].instrs[ii]) {
            Some(d) => d,
            None => continue,
        };
        if outside.contains(&dst) {
            continue;
        }
        let dominated = uses_of(func, &lp.blocks, dst)
            .iter()
            .all(|u| dom.dominates(bi, *u));
        if dominated {
            hoisted.push((bi, ii));
        }
    }
    if hoisted.is_empty() {
        return false;
    }
    hoisted.sort();
    let mut moving: BTreeMap<BlockId, Vec<usize>> = BTreeMap::new();
    for (bi, ii) in hoisted {
        moving.entry(bi).or_default().push(ii);
    }
    let mut stash: Vec<Instr> = Vec::new();
    for (bi, indices) in moving.iter().rev() {
        for ii in indices.iter().rev() {
            stash.push(func.blocks[*bi].instrs.remove(*ii));
        }
    }
    stash.reverse();
    let pos = func.blocks[pre].instrs.len();
    for (k, ins) in stash.into_iter().enumerate() {
        func.blocks[pre].instrs.insert(pos + k, ins);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loop_func() -> Function {
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
            locals: vec![LirType::I64; 8],
            blocks: vec![
                Block {
                    instrs: vec![
                        Instr::Const {span: UNKNOWN_SPAN,  dst: 2, lit: Lit::Int(10) },
                        Instr::Const {span: UNKNOWN_SPAN,  dst: 3, lit: Lit::Int(20) },
                    ],
                    term: Terminator::Br(1),
                },
                Block {
                    instrs: vec![
                        Instr::Const {span: UNKNOWN_SPAN,  dst: 4, lit: Lit::Int(0) },
                        Instr::Cmp {span: UNKNOWN_SPAN, 
                            op: CmpOp::Lt,
                            kind: NumKind::Int,
                            dst: 5,
                            lhs: 0,
                            rhs: 4,
                        },
                    ],
                    term: Terminator::BrIf {span: UNKNOWN_SPAN,  cond: 5, then_bb: 2, else_bb: 3 },
                },
                Block {
                    instrs: vec![Instr::Arith {span: UNKNOWN_SPAN, 
                        op: ArithOp::Add,
                        kind: NumKind::Int,
                        dst: 6,
                        lhs: 2,
                        rhs: 3,
                    }],
                    term: Terminator::Br(1),
                },
                Block { instrs: Vec::new(), term: Terminator::Ret(vec![6]) },
            ],
        }
    }

    #[test]
    fn finds_back_edge_loop() {
        let f = loop_func();
        let dom = compute_dominators(&f);
        assert!(dom.dominates(1, 2));
        assert!(!dom.dominates(2, 1));
        let loops = find_natural_loops(&f, &dom);
        assert_eq!(loops.len(), 1);
        assert_eq!(loops[0].header, 1);
        assert!(loops[0].blocks.contains(&2));
    }

    #[test]
    fn hoists_invariant_add() {
        let mut f = loop_func();
        assert!(loop_invariant_code_motion(&mut f));
        let hoisted = f.blocks[0].instrs.iter().any(|i| {
            matches!(i, Instr::Arith {  op: ArithOp::Add, lhs: 2, rhs: 3 , ..})
        });
        assert!(hoisted);
        assert!(!f.blocks[2].instrs.iter().any(|i| {
            matches!(i, Instr::Arith { ..})
        }));
    }

    #[test]
    fn keeps_guarded_div_in_loop() {
        let mut f = loop_func();
        f.blocks[2].instrs.push(Instr::Arith {span: UNKNOWN_SPAN, 
            op: ArithOp::Div,
            kind: NumKind::Int,
            dst: 7,
            lhs: 6,
            rhs: 0,
        });
        assert!(loop_invariant_code_motion(&mut f));
        assert!(f.blocks[2].instrs.iter().any(|i| {
            matches!(i, Instr::Arith {  op: ArithOp::Div , ..})
        }));
    }
}
