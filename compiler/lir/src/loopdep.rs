use crate::bce::{
    analyze_counted, array_aliases, clean_reachable, external_preds, idx_affine, iv_range,
    loop_latches, nest_has_exits, AffineForm, CountedLoop,
};
use crate::instr::*;
use crate::licm::{compute_dominators, find_natural_loops, DominatorTree};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_NEST_DEPTH: usize = 3;
pub const MAX_ACCESS_PAIRS: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    Lt,
    Eq,
    Gt,
    Star,
}

#[derive(Clone, Debug)]
pub struct AccessFact {
    pub block: BlockId,
    pub instr: usize,
    pub arr: Local,
    pub root: Local,
    pub is_write: bool,
    pub form: AffineForm,
}

#[derive(Clone, Debug)]
pub struct Dependence {
    pub src: usize,
    pub dst: usize,
    pub dirs: Vec<Dir>,
}

#[derive(Clone, Debug)]
pub struct NestFacts {
    pub innermost_header: BlockId,
    pub ivs: Vec<Local>,
    pub headers: Vec<BlockId>,
    pub blocks: Vec<BTreeSet<BlockId>>,
    pub bounds: Vec<(i128, i128)>,
    pub accesses: Vec<AccessFact>,
    pub dependences: Vec<Dependence>,
}

fn gcd_i128(mut a: i128, mut b: i128) -> i128 {
    a = a.abs();
    b = b.abs();
    while b != 0 {
        let t = a % b;
        a = b;
        b = t;
    }
    a
}

fn floor_div(a: i128, b: i128) -> Option<i128> {
    if b == 0 {
        return None;
    }
    if b > 0 {
        Some(a.div_euclid(b))
    } else {
        Some(a.checked_neg()?.div_euclid(b.checked_neg()?))
    }
}

fn ceil_div(a: i128, b: i128) -> Option<i128> {
    if b == 0 {
        return None;
    }
    Some(floor_div(a.checked_neg()?, b)?.checked_neg()?)
}

#[derive(Clone, Copy)]
struct Range {
    lo: i128,
    hi: i128,
}

fn range_add(a: Range, b: Range) -> Option<Range> {
    Some(Range { lo: a.lo.checked_add(b.lo)?, hi: a.hi.checked_add(b.hi)? })
}

fn range_scale(r: Range, k: i128) -> Option<Range> {
    if k == 0 {
        return Some(Range { lo: 0, hi: 0 });
    }
    if k > 0 {
        Some(Range { lo: r.lo.checked_mul(k)?, hi: r.hi.checked_mul(k)? })
    } else {
        Some(Range { lo: r.hi.checked_mul(k)?, hi: r.lo.checked_mul(k)? })
    }
}

fn range_of_iv(bounds: &[Option<Range>], ivs: &[Local], iv: Local) -> Option<Range> {
    let pos = ivs.iter().position(|v| *v == iv)?;
    bounds.get(pos).copied().flatten()
}

enum PairVerdict {
    Independent,
    Dependent(Vec<Dir>),
}

enum LevelOutcome {
    Contradiction,
    Unknown,
    Diff(i128, i128),
}

fn solve_pair(
    src: &AffineForm,
    dst: &AffineForm,
    ivs: &[Local],
    bounds: &[Option<Range>],
) -> PairVerdict {
    if src.coeffs.is_empty() && dst.coeffs.is_empty() {
        if src.c0 == dst.c0 {
            return PairVerdict::Dependent(vec![Dir::Star; ivs.len()]);
        }
        return PairVerdict::Independent;
    }
    let rhs = match dst.c0.checked_sub(src.c0) {
        Some(v) => v,
        None => return PairVerdict::Dependent(vec![Dir::Star; ivs.len()]),
    };
    let mut g: i128 = 0;
    for v in ivs {
        g = gcd_i128(g, src.coeffs.get(v).copied().unwrap_or(0));
        g = gcd_i128(g, dst.coeffs.get(v).copied().unwrap_or(0));
    }
    if g != 0 && rhs % g != 0 {
        return PairVerdict::Independent;
    }
    let mut eq: BTreeSet<usize> = BTreeSet::new();
    for (k, v) in ivs.iter().enumerate() {
        let a = src.coeffs.get(v).copied().unwrap_or(0);
        let b = dst.coeffs.get(v).copied().unwrap_or(0);
        if a == 0 && b == 0 {
            eq.insert(k);
        }
    }
    loop {
        let mut grown = false;
        for (k, v) in ivs.iter().enumerate() {
            if eq.contains(&k) {
                continue;
            }
            let a = src.coeffs.get(v).copied().unwrap_or(0);
            let b = dst.coeffs.get(v).copied().unwrap_or(0);
            if a != b || a == 0 {
                continue;
            }
            if matches!(level_diff(src, dst, ivs, bounds, k, &eq, rhs), LevelOutcome::Diff(0, 0))
            {
                eq.insert(k);
                grown = true;
            }
        }
        if !grown {
            break;
        }
    }
    let mut dirs = Vec::with_capacity(ivs.len());
    for (k, v) in ivs.iter().enumerate() {
        if eq.contains(&k) {
            dirs.push(Dir::Eq);
            continue;
        }
        let a = src.coeffs.get(v).copied().unwrap_or(0);
        let b = dst.coeffs.get(v).copied().unwrap_or(0);
        if a != b {
            dirs.push(Dir::Star);
            continue;
        }
        match level_diff(src, dst, ivs, bounds, k, &eq, rhs) {
            LevelOutcome::Contradiction => return PairVerdict::Independent,
            LevelOutcome::Unknown => dirs.push(Dir::Star),
            LevelOutcome::Diff(0, 0) => dirs.push(Dir::Eq),
            LevelOutcome::Diff(_, hi) if hi < 0 => dirs.push(Dir::Lt),
            LevelOutcome::Diff(lo, _) if lo > 0 => dirs.push(Dir::Gt),
            LevelOutcome::Diff(_, _) => dirs.push(Dir::Star),
        }
    }
    PairVerdict::Dependent(dirs)
}

fn level_diff(
    src: &AffineForm,
    dst: &AffineForm,
    ivs: &[Local],
    bounds: &[Option<Range>],
    k: usize,
    eq: &BTreeSet<usize>,
    rhs: i128,
) -> LevelOutcome {
    let c = src.coeffs.get(&ivs[k]).copied().unwrap_or(0);
    if c == 0 {
        return LevelOutcome::Unknown;
    }
    let mut acc = Range { lo: rhs, hi: rhs };
    for (j, v) in ivs.iter().enumerate() {
        if j == k {
            continue;
        }
        let a = src.coeffs.get(v).copied().unwrap_or(0);
        let b = dst.coeffs.get(v).copied().unwrap_or(0);
        if a == 0 && b == 0 {
            continue;
        }
        let r = match range_of_iv(bounds, ivs, *v) {
            Some(r) => r,
            None => return LevelOutcome::Unknown,
        };
        let part = if eq.contains(&j) {
            match range_scale(r, a.checked_sub(b).unwrap_or(0)) {
                Some(p) => p,
                None => return LevelOutcome::Unknown,
            }
        } else {
            let pos = match range_scale(r, a) {
                Some(p) => p,
                None => return LevelOutcome::Unknown,
            };
            let neg = match b.checked_neg().and_then(|nb| range_scale(r, nb)) {
                Some(p) => p,
                None => return LevelOutcome::Unknown,
            };
            match range_add(pos, neg) {
                Some(p) => p,
                None => return LevelOutcome::Unknown,
            }
        };
        acc = match acc.lo.checked_sub(part.hi).and_then(|lo| {
            acc.hi.checked_sub(part.lo).map(|hi| Range { lo, hi })
        }) {
            Some(r) => r,
            None => return LevelOutcome::Unknown,
        };
    }
    let (mut lo, mut hi) = if c > 0 {
        match (ceil_div(acc.lo, c), floor_div(acc.hi, c)) {
            (Some(lo), Some(hi)) => (lo, hi),
            _ => return LevelOutcome::Unknown,
        }
    } else {
        match (ceil_div(acc.hi, c), floor_div(acc.lo, c)) {
            (Some(lo), Some(hi)) => (lo, hi),
            _ => return LevelOutcome::Unknown,
        }
    };
    match range_of_iv(bounds, ivs, ivs[k]) {
        Some(r) => {
            let feasible = match (r.lo.checked_sub(r.hi), r.hi.checked_sub(r.lo)) {
                (Some(lo), Some(hi)) => Range { lo, hi },
                _ => return LevelOutcome::Unknown,
            };
            lo = lo.max(feasible.lo);
            hi = hi.min(feasible.hi);
        }
        None => return LevelOutcome::Unknown,
    }
    if lo > hi {
        return LevelOutcome::Contradiction;
    }
    LevelOutcome::Diff(lo, hi)
}

fn may_alias(
    fam_a: &BTreeSet<Local>,
    fam_b: &BTreeSet<Local>,
    fresh: &BTreeMap<Local, Local>,
    a: Local,
    b: Local,
) -> bool {
    if !fam_a.is_disjoint(fam_b) {
        return true;
    }
    match (fresh.get(&a), fresh.get(&b)) {
        (Some(ra), Some(rb)) => ra == rb,
        _ => true,
    }
}

fn union_has_banned(func: &Function, union_blocks: &BTreeSet<BlockId>) -> bool {
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

fn single_br_pred(func: &Function, header: BlockId, blocks: &BTreeSet<BlockId>) -> bool {
    let externals = external_preds(func, header, blocks);
    if externals.len() != 1 {
        return false;
    }
    matches!(func.blocks.get(externals[0]).map(|b| &b.term), Some(Terminator::Br(t)) if *t == header)
}

fn analyze_nest(
    func: &Function,
    dom: &DominatorTree,
    nest: &[&CountedLoop],
    fresh: &BTreeMap<Local, Local>,
) -> Option<NestFacts> {
    if nest.is_empty() || nest.len() > MAX_NEST_DEPTH {
        return None;
    }
    let mut sizes: Vec<usize> = nest.iter().map(|l| l.blocks.len()).collect();
    sizes.sort_by(|a, b| b.cmp(a));
    for w in sizes.windows(2) {
        if w[0] <= w[1] {
            return None;
        }
    }
    let union_blocks: BTreeSet<BlockId> =
        nest.iter().flat_map(|l| l.blocks.iter().copied()).collect();
    let headers: BTreeSet<BlockId> = nest.iter().map(|l| l.header).collect();
    if nest_has_exits(func, &union_blocks, &headers) {
        return None;
    }
    if union_has_banned(func, &union_blocks) {
        return None;
    }
    for l in nest {
        if loop_latches(func, &l.blocks, l.header).len() != 1 {
            return None;
        }
    }
    let inner = nest[nest.len() - 1];
    if !single_br_pred(func, inner.header, &inner.blocks) {
        return None;
    }
    let ivs: Vec<Local> = nest.iter().map(|l| l.iv).collect();
    let exact_bounds: Vec<(i128, i128)> =
        ivs.iter().map(|v| iv_range(nest, *v)).collect::<Option<Vec<_>>>()?;
    let opt_bounds: Vec<Option<Range>> =
        exact_bounds.iter().map(|(lo, hi)| Some(Range { lo: *lo, hi: *hi })).collect();
    let mut ordered_blocks: Vec<BlockId> = union_blocks.iter().copied().collect();
    ordered_blocks.sort();
    let mut accesses: Vec<AccessFact> = Vec::new();
    for b in ordered_blocks {
        if headers.contains(&b) {
            let block = match func.blocks.get(b) {
                Some(x) => x,
                None => return None,
            };
            for ins in &block.instrs {
                if matches!(ins, Instr::ArrayGet { .. } | Instr::ArraySet { .. }) {
                    return None;
                }
            }
            continue;
        }
        let count = func.blocks.get(b).map(|x| x.instrs.len()).unwrap_or(0);
        for ii in 0..count {
            let (arr, idx, is_write) = match func.blocks.get(b).and_then(|x| x.instrs.get(ii)) {
                Some(Instr::ArrayGet { arr, index, .. }) => (*arr, *index, false),
                Some(Instr::ArraySet { arr, index, .. }) => (*arr, *index, true),
                _ => continue,
            };
            if !matches!(func.locals.get(arr as usize), Some(LirType::Array(_))) {
                return None;
            }
            if crate::bce::all_defs(func, arr).iter().any(|(db, _)| union_blocks.contains(db)) {
                return None;
            }
            let form = idx_affine(func, nest, dom, b, ii, idx, 0)?;
            for v in form.coeffs.keys() {
                if !ivs.contains(v) {
                    return None;
                }
            }
            for l in nest.iter() {
                if form.coeffs.contains_key(&l.iv)
                    && !clean_reachable(func, l.header, l.then_bb, l.iv, b, ii)
                {
                    return None;
                }
            }
            let fam = array_aliases(func, arr);
            let root = fam.iter().next().copied().unwrap_or(arr);
            accesses.push(AccessFact { block: b, instr: ii, arr, root, is_write, form });
        }
    }
    let mut pairs: Vec<(usize, usize)> = Vec::new();
    for i in 0..accesses.len() {
        for j in i..accesses.len() {
            if !accesses[i].is_write && !accesses[j].is_write {
                continue;
            }
            let fa = array_aliases(func, accesses[i].arr);
            let fb = array_aliases(func, accesses[j].arr);
            if !may_alias(&fa, &fb, fresh, accesses[i].arr, accesses[j].arr) {
                continue;
            }
            pairs.push((i, j));
        }
    }
    if pairs.len() > MAX_ACCESS_PAIRS {
        return None;
    }
    let mut dependences: Vec<Dependence> = Vec::new();
    for (i, j) in pairs {
        match solve_pair(&accesses[i].form, &accesses[j].form, &ivs, &opt_bounds) {
            PairVerdict::Independent => {}
            PairVerdict::Dependent(dirs) => dependences.push(Dependence { src: i, dst: j, dirs }),
        }
    }
    Some(NestFacts {
        innermost_header: inner.header,
        ivs,
        headers: nest.iter().map(|l| l.header).collect(),
        blocks: nest.iter().map(|l| l.blocks.clone()).collect(),
        bounds: exact_bounds,
        accesses,
        dependences,
    })
}

pub fn analyze_function(func: &Function) -> Vec<NestFacts> {
    let dom = compute_dominators(func);
    let loops = find_natural_loops(func, &dom);
    let mut counted: Vec<CountedLoop> =
        loops.iter().filter_map(|lp| analyze_counted(func, lp)).collect();
    counted.sort_by_key(|l| l.header);
    let fresh = crate::escape::fresh_array_roots(func);
    let mut out: Vec<NestFacts> = Vec::new();
    for inner in &counted {
        let mut nest: Vec<&CountedLoop> =
            counted.iter().filter(|o| o.blocks.is_superset(&inner.blocks)).collect();
        nest.sort_by(|a, b| b.blocks.len().cmp(&a.blocks.len()).then(a.header.cmp(&b.header)));
        if nest.last().map(|l| l.header) != Some(inner.header) {
            continue;
        }
        if let Some(facts) = analyze_nest(func, &dom, &nest, &fresh) {
            out.push(facts);
        }
    }
    out.sort_by_key(|f| f.innermost_header);
    out
}

pub fn permutation_valid(facts: &NestFacts, perm: &[usize]) -> bool {
    // Listed-Eq at a level holds for every solution of the subscript
    // equation, hence for every execution-ordered dependence too. A
    // permutation only reorders moved levels, so requiring Eq on all
    // moved levels keeps the first non-Eq component untouched.
    if perm.len() != facts.ivs.len() {
        return false;
    }
    let mut seen: Vec<usize> = perm.to_vec();
    seen.sort();
    if seen != (0..facts.ivs.len()).collect::<Vec<_>>() {
        return false;
    }
    for dep in &facts.dependences {
        if dep.dirs.len() != facts.ivs.len() {
            return false;
        }
        for (k, p) in perm.iter().enumerate() {
            if *p != k && dep.dirs[k] != Dir::Eq {
                return false;
            }
        }
    }
    true
}

pub fn interchange_valid(facts: &NestFacts, a: usize, b: usize) -> bool {
    if a >= facts.ivs.len() || b >= facts.ivs.len() {
        return false;
    }
    let mut perm: Vec<usize> = (0..facts.ivs.len()).collect();
    perm.swap(a, b);
    permutation_valid(facts, &perm)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lower_entry(src: &str, entry: &str) -> crate::instr::Function {
        let mut m = frontend::parser::Parser::parse_module(src).expect("parse");
        assert!(frontend::desugar::desugar(&mut m).is_empty());
        let module = crate::lower::lower(&m).expect("lower");
        module.functions.iter().find(|f| f.name == entry).expect("entry").clone()
    }

    fn facts_of(src: &str, entry: &str) -> Vec<NestFacts> {
        let f = lower_entry(src, entry);
        analyze_function(&f)
    }

    const MATMUL_IJK: &str = "fn Main(): Int {\n    let n = 4;\n    let a: Array<Int> = [];\n    let b: Array<Int> = [];\n    let c: Array<Int> = [];\n    let x = 0;\n    while x < n {\n        let y = 0;\n        while y < n {\n            let s = 0;\n            let z = 0;\n            while z < n {\n                s = s + a[x * n + z] * b[z * n + y];\n                z = z + 1;\n            }\n            c[x * n + y] = s;\n            y = y + 1;\n        }\n        x = x + 1;\n    }\n    return 0;\n}\n";

    #[test]
    fn matmul_ijk_yields_loop_independent_c_store() {
        let facts = facts_of(MATMUL_IJK, "Main");
        let kernel = facts.iter().find(|f| f.ivs.len() == 3).expect("3-deep kernel facts");
        assert_eq!(kernel.accesses.len(), 3);
        assert_eq!(kernel.dependences.len(), 1);
        let dep = &kernel.dependences[0];
        assert_eq!(dep.dirs, vec![Dir::Eq, Dir::Eq, Dir::Eq]);
        assert!(kernel.accesses[dep.src].is_write);
        assert_eq!(dep.src, dep.dst);
        assert!(interchange_valid(kernel, 1, 2));
        assert!(permutation_valid(kernel, &[0, 1, 2]));
    }

    #[test]
    fn non_affine_index_yields_no_facts() {
        let src = "fn Main(): Int {\n    let n = 4;\n    let a: Array<Int> = [];\n    let b: Array<Int> = [];\n    let s = 0;\n    let i = 0;\n    while i < n {\n        let j = 0;\n        while j < n {\n            s = s + a[b[j]];\n            j = j + 1;\n        }\n        i = i + 1;\n    }\n    return s;\n}\n";
        assert!(facts_of(src, "Main").is_empty());
    }

    #[test]
    fn call_in_body_yields_no_facts() {
        let src = "fn Main(): Int {\n    let n = 4;\n    let a: Array<Int> = [];\n    let s = 0;\n    let i = 0;\n    while i < n {\n        let j = 0;\n        while j < n {\n            s = s + a[j];\n            print(\"x\", s);\n            j = j + 1;\n        }\n        i = i + 1;\n    }\n    return s;\n}\n";
        assert!(facts_of(src, "Main").is_empty());
    }

    #[test]
    fn carried_c_subscript_blocks_interchange() {
        let src = "fn Main(): Int {\n    let n = 8;\n    let c: Array<Int> = [];\n    let x = 0;\n    while x < n {\n        let y = 0;\n        while y < n {\n            let z = 0;\n            while z < n {\n                c[z + 1] = c[z] + 1;\n                z = z + 1;\n            }\n            y = y + 1;\n        }\n        x = x + 1;\n    }\n    return 0;\n}\n";
        let facts = facts_of(src, "Main");
        let kernel = facts.iter().find(|f| f.ivs.len() == 3).expect("3-deep facts");
        assert!(!kernel.dependences.is_empty());
        assert!(
            kernel
                .dependences
                .iter()
                .any(|d| d.dirs.contains(&Dir::Gt)
                    || d.dirs.contains(&Dir::Lt)
                    || d.dirs.contains(&Dir::Star))
        );
        assert!(!interchange_valid(kernel, 1, 2));
    }

    #[test]
    fn float_reduction_yields_no_facts() {
        let src = "fn Main(): Int {\n    let n = 4;\n    let s = 0.5;\n    let i = 0;\n    while i < n {\n        let j = 0;\n        while j < n {\n            s = s + 0.5;\n            j = j + 1;\n        }\n        i = i + 1;\n    }\n    return 0;\n}\n";
        assert!(facts_of(src, "Main").is_empty());
    }

    #[test]
    fn nest_deeper_than_three_yields_no_facts() {
        let src = "fn Main(): Int {\n    let n = 4;\n    let a: Array<Int> = [];\n    let s = 0;\n    let i = 0;\n    while i < n {\n        let j = 0;\n        while j < n {\n            let k = 0;\n            while k < n {\n                let m = 0;\n                while m < n {\n                    s = s + a[m];\n                    m = m + 1;\n                }\n                k = k + 1;\n            }\n            j = j + 1;\n        }\n        i = i + 1;\n    }\n    return s;\n}\n";
        assert!(facts_of(src, "Main").is_empty());
    }

    #[test]
    fn pair_cap_yields_no_facts() {
        let mut body = String::new();
        for k in 0..6 {
            body.push_str(&format!("            c[i + {k}] = {k};\n"));
        }
        let src = format!("fn Main(): Int {{\n    let n = 8;\n    let c: Array<Int> = [];\n    let i = 0;\n    while i < n {{\n{body}        i = i + 1;\n    }}\n    return 0;\n}}\n");
        assert!(facts_of(&src, "Main").is_empty());
    }

    #[test]
    fn gcd_proves_disjoint_parity_accesses() {
        let src = "fn Main(): Int {\n    let n = 8;\n    let a: Array<Int> = [];\n    let s = 0;\n    let i = 0;\n    while i < n {\n        s = s + a[2 * i] + a[2 * i + 1];\n        i = i + 1;\n    }\n    return s;\n}\n";
        let facts = facts_of(src, "Main");
        assert_eq!(facts.len(), 1);
        assert!(facts[0].dependences.is_empty());
    }

    #[test]
    fn banerjee_range_proves_far_apart_accesses() {
        let src = "fn Main(): Int {\n    let n = 8;\n    let a: Array<Int> = [];\n    let s = 0;\n    let i = 0;\n    while i < n {\n        a[i] = a[i + 300];\n        i = i + 1;\n    }\n    return s;\n}\n";
        let facts = facts_of(src, "Main");
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].dependences.len(), 1);
        assert_eq!(facts[0].dependences[0].dirs, vec![Dir::Eq]);
    }

    #[test]
    fn analysis_leaves_function_byte_identical() {
        let f = lower_entry(MATMUL_IJK, "Main");
        let before = format!("{:?}", f);
        let facts = analyze_function(&f);
        assert!(!facts.is_empty());
        assert_eq!(format!("{:?}", f), before);
    }
}
