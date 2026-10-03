use crate::instr::*;
use crate::opt::{instr_dst, instr_reads, term_reads};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EscapeState {
    NoEscape,
    Escapes,
}

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
    let mut copies: Vec<(Local, Local)> = Vec::new();
    for block in &func.blocks {
        for ins in &block.instrs {
            if let Instr::Copy {  dst, src , ..} | Instr::Cast { dst, src , ..} = ins {
                copies.push((*dst, *src));
            }
            if let Instr::Defer {  body , ..} = ins {
                for nested in body {
                    if let Instr::Copy {  dst, src , ..} | Instr::Cast { dst, src , ..} = nested {
                        copies.push((*dst, *src));
                    }
                }
            }
        }
    }
    for (dst, src) in copies {
        union_alias(&mut alias, dst, src);
    }
    alias
}

struct EscapeCtx<'a> {
    module: &'a Module,
    memo: BTreeMap<(usize, usize), bool>,
    visiting: Vec<(usize, usize)>,
}

const MAX_DEPTH: usize = 8;

impl<'a> EscapeCtx<'a> {
    fn param_safe(&mut self, id: usize, param: usize) -> bool {
        if let Some(safe) = self.memo.get(&(id, param)) {
            return *safe;
        }
        if self.visiting.len() >= MAX_DEPTH || self.visiting.contains(&(id, param)) {
            return false;
        }
        let callee = match self.module.functions.get(id) {
            Some(f) => f,
            None => return false,
        };
        if param >= callee.params.len() {
            return false;
        }
        self.visiting.push((id, param));
        let safe = self.param_body_safe(id, param);
        self.visiting.pop();
        self.memo.insert((id, param), safe);
        safe
    }

    fn param_body_safe(&mut self, id: usize, param: usize) -> bool {
        let callee = &self.module.functions[id];
        let alias = collect_aliases(callee);
        let proot = find_root(&alias, param as Local);
        let member = |l: Local| find_root(&alias, l) == proot;
        for block in &callee.blocks {
            for ins in &block.instrs {
                if self.instr_leaks(ins, member) {
                    return false;
                }
            }
            match &block.term {
                Terminator::Ret(v) => {
                    if v.iter().any(|l| member(*l)) {
                        return false;
                    }
                }
                Terminator::Throw { src, .. } => {
                    if member(*src) {
                        return false;
                    }
                }
                Terminator::Rethrow { err, .. } => {
                    if member(*err) {
                        return false;
                    }
                }
                _ => {}
            }
        }
        true
    }

    fn instr_leaks(&mut self, ins: &Instr, member: impl Fn(Local) -> bool + Copy) -> bool {
        match ins {
            Instr::Call {  target, args , ..} => match target {
                CallTarget::Fn(id) => {
                    for (i, a) in args.iter().enumerate() {
                        if member(*a) && !self.param_safe(*id, i) {
                            return true;
                        }
                    }
                    false
                }
                _ => args.iter().any(|a| member(*a)),
            },
            Instr::SetField {  value , ..} | Instr::SetFieldByName { value , ..} => member(*value),
            Instr::ArraySet {  value , ..} => member(*value),
            Instr::ArrayPush {  value , ..} => member(*value),
            Instr::EnumNew {  payload , ..} => payload.iter().any(|p| member(*p)),
            Instr::ClosureNew {  captures , ..} => captures.iter().any(|c| member(*c)),
            Instr::GenRefOf {  obj , ..} => member(*obj),
            Instr::AddrOf {  src , ..} => member(*src),
            Instr::PtrLoad {  ptr: src , ..} => member(*src),
            Instr::PtrStore {  val , ..} => member(*val),
            Instr::Defer {  body , ..} => body.iter().any(|n| self.instr_leaks(n, member)),
            Instr::Const { ..}
            | Instr::Copy { ..}
            | Instr::Cast { ..}
            | Instr::Convert { ..}
            | Instr::Arith { ..}
            | Instr::Fma { ..}
            | Instr::Cmp { ..}
            | Instr::Not { ..}
            | Instr::Neg { ..}
            | Instr::Concat { ..}
            | Instr::ToStr { ..}
            | Instr::Range { ..}
            | Instr::Stride { ..}
            | Instr::ArrayNew { ..}
            | Instr::ArrayPop { ..}
            | Instr::ArrayLen { ..}
            | Instr::ArrayGet { ..}
            | Instr::PtrLoad { ..}
            | Instr::ObjNew { ..}
            | Instr::StackAlloc { ..}
            | Instr::EnumPayload { ..}
            | Instr::EnumTag { ..}
            | Instr::Extract { ..}
            | Instr::RangeLo { ..}
            | Instr::RangeHi { ..}
            | Instr::RangeStep { ..}
            | Instr::GetField { ..}
            | Instr::GetFieldByName { ..}
            | Instr::GenRefEmpty { ..}
            | Instr::GenRefGet { ..}
            | Instr::GenRefInvalidate { ..}
            | Instr::ThreadSpawn { ..}
            | Instr::ThreadJoin { ..}
            | Instr::PoolInit { ..}
            | Instr::PoolSubmit { ..}
            | Instr::PoolParallelFor { ..}
            | Instr::PoolJoin { ..}
            | Instr::PoolShutdown { ..}
            | Instr::VecNew { ..}
            | Instr::VecSplat { ..}
            | Instr::VecExtract { ..}
            | Instr::VecInsert { ..}
            | Instr::VecArith { ..}
            | Instr::VecUnary { ..}
            | Instr::VecDot { ..}
            | Instr::ReleaseField { ..}
            | Instr::Retain { ..}
            | Instr::Release { ..}
            | Instr::ReleaseAs { ..}
            | Instr::RunDefers { ..}
            | Instr::Assert { ..}
            | Instr::Panic { ..} => false,
        }
    }

    fn roots(&mut self, func: &Function) -> BTreeSet<Local> {
        let alias = collect_aliases(func);
        let mut out: BTreeSet<Local> = BTreeSet::new();
        let mut escape = |l: Local| {
            out.insert(find_root(&alias, l));
        };
        for block in &func.blocks {
            for ins in &block.instrs {
                self.scan_top(ins, &mut escape);
            }
            match &block.term {
                Terminator::Ret(v) => {
                    for l in v {
                        escape(*l);
                    }
                }
                Terminator::Throw { src, .. } => escape(*src),
                Terminator::Rethrow { err, .. } => escape(*err),
                Terminator::Br(_)
                | Terminator::BrIf { .. }
                | Terminator::BrErr { .. }
                | Terminator::Switch { .. }
                | Terminator::Unreachable { .. } => {}
            }
        }
        out
    }

    fn scan_top(&mut self, ins: &Instr, escape: &mut impl FnMut(Local)) {
        match ins {
            Instr::Call {  target, args , ..} => match target {
                CallTarget::Fn(id) => {
                    for (i, a) in args.iter().enumerate() {
                        if !self.param_safe(*id, i) {
                            escape(*a);
                        }
                    }
                }
                _ => {
                    for a in args {
                        escape(*a);
                    }
                }
            },
            Instr::SetField {  value , ..} | Instr::SetFieldByName { value , ..} => {
                escape(*value)
            }
            Instr::ArraySet {  value , ..} => escape(*value),
            Instr::ArrayPush {  value , ..} => escape(*value),
            Instr::EnumNew {  payload , ..} => {
                for p in payload {
                    escape(*p);
                }
            }
            Instr::ClosureNew {  captures , ..} => {
                for c in captures {
                    escape(*c);
                }
            }
            Instr::GenRefOf {  obj , ..} => escape(*obj),
            Instr::AddrOf {  src , ..} => escape(*src),
            Instr::PtrStore {  val , ..} => escape(*val),
            Instr::Defer {  body , ..} => {
                for nested in body {
                    self.scan_top(nested, escape);
                }
            }
            Instr::Const { ..}
            | Instr::Copy { ..}
            | Instr::Cast { ..}
            | Instr::Convert { ..}
            | Instr::Arith { ..}
            | Instr::Fma { ..}
            | Instr::Cmp { ..}
            | Instr::Not { ..}
            | Instr::Neg { ..}
            | Instr::Concat { ..}
            | Instr::ToStr { ..}
            | Instr::Range { ..}
            | Instr::Stride { ..}
            | Instr::ArrayNew { ..}
            | Instr::ArrayPop { ..}
            | Instr::ArrayLen { ..}
            | Instr::ArrayGet { ..}
             | Instr::PtrLoad { ..}
            | Instr::ObjNew { ..}
            | Instr::StackAlloc { ..}
            | Instr::EnumPayload { ..}
            | Instr::EnumTag { ..}
            | Instr::Extract { ..}
            | Instr::RangeLo { ..}
            | Instr::RangeHi { ..}
            | Instr::RangeStep { ..}
            | Instr::GetField { ..}
            | Instr::GetFieldByName { ..}
            | Instr::GenRefEmpty { ..}
            | Instr::GenRefGet { ..}
            | Instr::GenRefInvalidate { ..}
            | Instr::ThreadSpawn { ..}
            | Instr::ThreadJoin { ..}
            | Instr::PoolInit { ..}
            | Instr::PoolSubmit { ..}
            | Instr::PoolParallelFor { ..}
            | Instr::PoolJoin { ..}
            | Instr::PoolShutdown { ..}
            | Instr::VecNew { ..}
            | Instr::VecSplat { ..}
            | Instr::VecExtract { ..}
            | Instr::VecInsert { ..}
            | Instr::VecArith { ..}
            | Instr::VecUnary { ..}
            | Instr::VecDot { ..}
            | Instr::ReleaseField { ..}
            | Instr::Retain { ..}
            | Instr::Release { ..}
            | Instr::ReleaseAs { ..}
            | Instr::RunDefers { ..}
            | Instr::Assert { ..}
            | Instr::Panic { ..} => {}
        }
    }
}

pub fn compute_escape_states(
    module: &Module,
    func: &Function,
) -> BTreeMap<Local, EscapeState> {
    let mut ctx = EscapeCtx {
        module,
        memo: BTreeMap::new(),
        visiting: Vec::new(),
    };
    let roots = ctx.roots(func);
    let alias = collect_aliases(func);
    let mut states: BTreeMap<Local, EscapeState> = BTreeMap::new();
    for block in &func.blocks {
        for ins in &block.instrs {
            if let Instr::ObjNew {  dst , ..} = ins {
                let state = if roots.contains(&find_root(&alias, *dst)) {
                    EscapeState::Escapes
                } else {
                    EscapeState::NoEscape
                };
                states.insert(*dst, state);
            }
        }
    }
    states
}

fn all_locals(func: &Function) -> BTreeSet<Local> {
    let mut out: BTreeSet<Local> = BTreeSet::new();
    let mut tmp: Vec<Local> = Vec::new();
    for block in &func.blocks {
        for ins in &block.instrs {
            if let Some(dst) = instr_dst(ins) {
                out.insert(dst);
            }
            tmp.clear();
            instr_reads(ins, &mut tmp);
            out.extend(tmp.iter().copied());
            if let Instr::Call {  dsts, err , ..} = ins {
                for d in dsts {
                    out.insert(*d);
                }
                if let Some(e) = err {
                    out.insert(*e);
                }
            }
            if let Instr::Defer {  body , ..} = ins {
                for nested in body {
                    if let Some(dst) = instr_dst(nested) {
                        out.insert(dst);
                    }
                    tmp.clear();
                    instr_reads(nested, &mut tmp);
                    out.extend(tmp.iter().copied());
                }
            }
        }
        tmp.clear();
        term_reads(&block.term, &mut tmp);
        out.extend(tmp.iter().copied());
        if let Terminator::BrErr { catch_bind, .. } = &block.term {
            out.insert(*catch_bind);
        }
    }
    out
}

fn is_transparent_def(ins: &Instr, local: Local) -> bool {
    match ins {
        Instr::Copy {  dst , ..} | Instr::Cast { dst , ..} => *dst == local,
        _ => false,
    }
}

fn real_def_count(func: &Function, local: Local) -> usize {
    let mut count = 0;
    for block in &func.blocks {
        for ins in &block.instrs {
            if instr_dst(ins) == Some(local) && !is_transparent_def(ins, local) {
                count += 1;
            }
            if let Instr::Call {  dsts, err , ..} = ins {
                if dsts.contains(&local) || *err == Some(local) {
                    count += 1;
                }
            }
            if let Instr::Defer {  body , ..} = ins {
                for nested in body {
                    if instr_dst(nested) == Some(local) && !is_transparent_def(nested, local) {
                        count += 1;
                    }
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

pub(crate) fn is_trivial_dtor(functions: &[Function], id: usize) -> bool {
    let func = match functions.get(id) {
        Some(f) => f,
        None => return false,
    };
    func.blocks.iter().all(|b| {
        b.instrs.iter().all(|i| matches!(i, Instr::GenRefInvalidate { ..}))
            && matches!(b.term, Terminator::Ret(_) | Terminator::Br(_))
    })
}

pub fn optimize_stack_allocations(module: &mut Module, fi: usize) -> bool {
    let states = {
        let func = &module.functions[fi];
        compute_escape_states(module, func)
    };
    if states.is_empty() {
        return false;
    }
    let (alias, universe, promotable) = {
        let func = &module.functions[fi];
        let alias = collect_aliases(func);
        let universe = all_locals(func);
        let mut promotable: BTreeSet<Local> = BTreeSet::new();
        for (local, state) in &states {
            if *state != EscapeState::NoEscape {
                continue;
            }
            if real_def_count(func, *local) != 1 {
                continue;
            }
            promotable.insert(*local);
        }
        (alias, universe, promotable)
    };
    if promotable.is_empty() {
        return false;
    }
    let mut blocked_classes: BTreeSet<usize> = BTreeSet::new();
    for (ci, class) in module.classes.iter().enumerate() {
        if class.deinit.is_some()
            || class.dtor.is_some_and(|id| !is_trivial_dtor(&module.functions, id))
        {
            blocked_classes.insert(ci);
        }
    }
    let mut promoted: BTreeSet<Local> = BTreeSet::new();
    {
        let functions = &mut module.functions;
        let func = &mut functions[fi];
        for block in &mut func.blocks {
            for ins in &mut block.instrs {
                if let Instr::ObjNew {  dst, class, instance_size , ..} = ins {
                    if !promotable.contains(dst) || blocked_classes.contains(class) {
                        continue;
                    }
                    let (dst, class, instance_size) = (*dst, *class, *instance_size);
                    *ins = Instr::StackAlloc { span: ins.span(), dst, class, instance_size };
                    promoted.insert(dst);
                }
            }
        }
    }
    if promoted.is_empty() {
        return false;
    }
    let mut group: BTreeSet<Local> = BTreeSet::new();
    for local in &promoted {
        let root = find_root(&alias, *local);
        for l in &universe {
            if find_root(&alias, *l) == root {
                group.insert(*l);
            }
        }
    }
    for block in &mut module.functions[fi].blocks {
        block.instrs.retain(|ins| match ins {
            Instr::Retain {  obj , ..} | Instr::Release { obj , ..} | Instr::ReleaseAs { obj , ..} => !group.contains(obj),
            _ => true,
        });
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj_func() -> Function {
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
            locals: vec![LirType::Obj("C".to_string()); 4],
            blocks: vec![Block {
                instrs: vec![
                    Instr::ObjNew {span: UNKNOWN_SPAN,  dst: 0, class: 0, instance_size: 24 },
                    Instr::Copy {span: UNKNOWN_SPAN,  dst: 1, src: 0 },
                    Instr::GetField {span: UNKNOWN_SPAN,  dst: 2, obj: 1, field: 0 },
                    Instr::Retain {span: UNKNOWN_SPAN,  obj: 0 },
                    Instr::Release {span: UNKNOWN_SPAN,  obj: 1 },
                ],
                term: Terminator::Ret(vec![2]),
            }],
        }
    }

    fn test_module(func: Function) -> Module {
        let mut module = Module { functions: vec![func], ..Default::default() };
        module.classes.push(ClassDesc {
            name: "C".to_string(),
            type_params: Vec::new(),
            is_struct: false,
            fields: Vec::new(),
            field_index: BTreeMap::new(),
            methods: BTreeMap::new(),
            deinit: None,
            dtor: None,
            ifaces: Vec::new(),
            parent: None,
        });
        module
    }

    #[test]
    fn noescape_through_alias() {
        let module = test_module(obj_func());
        let states = compute_escape_states(&module, &module.functions[0]);
        assert_eq!(states.get(&0), Some(&EscapeState::NoEscape));
    }

    #[test]
    fn escapes_on_return() {
        let mut module = test_module(obj_func());
        module.functions[0].blocks[0].term = Terminator::Ret(vec![0]);
        let states = compute_escape_states(&module, &module.functions[0]);
        assert_eq!(states.get(&0), Some(&EscapeState::Escapes));
    }

    #[test]
    fn escapes_on_call_arg() {
        let mut module = test_module(obj_func());
        module.functions[0].blocks[0].instrs.push(Instr::Call {span: UNKNOWN_SPAN,
            dsts: vec![],
            err: None,
            target: CallTarget::Builtin("print".to_string()),
            args: vec![1],
        });
        let states = compute_escape_states(&module, &module.functions[0]);
        assert_eq!(states.get(&0), Some(&EscapeState::Escapes));
    }

    #[test]
    fn promotes_and_strips_group_retains() {
        let mut module = test_module(obj_func());
        assert!(optimize_stack_allocations(&mut module, 0));
        assert!(matches!(
            module.functions[0].blocks[0].instrs[0],
            Instr::StackAlloc {  dst: 0 , ..}
        ));
        assert!(!module.functions[0].blocks[0].instrs.iter().any(|i| matches!(
            i,
            Instr::Retain { ..} | Instr::Release { ..} | Instr::ReleaseAs { ..}
        )));
    }

    #[test]
    fn skips_dtor_classes() {
        let mut module = test_module(obj_func());
        module.classes[0].dtor = Some(7);
        assert!(!optimize_stack_allocations(&mut module, 0));
        assert!(matches!(module.functions[0].blocks[0].instrs[0], Instr::ObjNew { ..}));
    }
}
