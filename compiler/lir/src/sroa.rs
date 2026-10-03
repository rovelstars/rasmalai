use crate::instr::*;
use crate::opt::{instr_dst, instr_reads, term_reads};
use std::collections::{BTreeMap, BTreeSet};

fn is_scalar_field(ty: &LirType) -> bool {
    matches!(
        ty,
        LirType::I64 | LirType::F64(_) | LirType::Bool | LirType::Null
    )
}

enum ObjField {
    Index(usize),
    Name(String),
}

fn field_of(classes: &[ClassDesc], class: usize, field: &ObjField) -> Option<usize> {
    let desc = classes.get(class)?;
    match field {
        ObjField::Index(i) => {
            if *i < desc.fields.len() {
                Some(*i)
            } else {
                None
            }
        }
        ObjField::Name(name) => desc.field_index.get(name.as_str()).copied(),
    }
}

fn access_field(ins: &Instr) -> Option<(Local, ObjField)> {
    match ins {
        Instr::GetField {  obj, field , ..} => Some((*obj, ObjField::Index(*field))),
        Instr::GetFieldByName {  obj, field , ..} => {
            Some((*obj, ObjField::Name(field.clone())))
        }
        Instr::SetField {  obj, field , ..} => Some((*obj, ObjField::Index(*field))),
        Instr::SetFieldByName {  obj, field , ..} => {
            Some((*obj, ObjField::Name(field.clone())))
        }
        _ => None,
    }
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

fn mentioned(ins: &Instr, out: &mut Vec<Local>) {
    let mut tmp: Vec<Local> = Vec::new();
    crate::opt::instr_dsts(ins, &mut tmp);
    out.extend(tmp.iter().copied());
    tmp.clear();
    instr_reads(ins, &mut tmp);
    out.extend(tmp.iter().copied());
    if let Instr::Call { err, .. } = ins {
        if let Some(e) = err {
            out.push(*e);
        }
    }
    if let Instr::Defer {  body , ..} = ins {
        for nested in body {
            if let Some(dst) = instr_dst(nested) {
                out.push(dst);
            }
            let mut tmp: Vec<Local> = Vec::new();
            instr_reads(nested, &mut tmp);
            out.extend(tmp.iter().copied());
        }
    }
}

fn group_of(
    func: &Function,
    alias: &BTreeMap<Local, Local>,
    local: Local,
) -> BTreeSet<Local> {
    let root = find_root(alias, local);
    let mut out: BTreeSet<Local> = BTreeSet::new();
    let mut tmp: Vec<Local> = Vec::new();
    for block in &func.blocks {
        for ins in &block.instrs {
            tmp.clear();
            mentioned(ins, &mut tmp);
            for l in tmp.drain(..) {
                if find_root(alias, l) == root {
                    out.insert(l);
                }
            }
        }
        tmp.clear();
        term_reads(&block.term, &mut tmp);
        for l in tmp.drain(..) {
            if find_root(alias, l) == root {
                out.insert(l);
            }
        }
        if let Terminator::BrErr { catch_bind, .. } = &block.term {
            if find_root(alias, *catch_bind) == root {
                out.insert(*catch_bind);
            }
        }
    }
    out.insert(local);
    out
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DefKind {
    Alloc,
    AliasCopy,
    Other,
}

fn member_defs(func: &Function, group: &BTreeSet<Local>) -> BTreeMap<Local, Vec<DefKind>> {
    let mut defs: BTreeMap<Local, Vec<DefKind>> = BTreeMap::new();
    for block in &func.blocks {
        for ins in &block.instrs {
            record_member_def(ins, group, &mut defs, true);
            if let Instr::Defer {  body , ..} = ins {
                for nested in body {
                    record_member_def(nested, group, &mut defs, false);
                }
            }
        }
        if let Terminator::BrErr { catch_bind, .. } = &block.term {
            if group.contains(catch_bind) {
                defs.entry(*catch_bind).or_default().push(DefKind::Other);
            }
        }
    }
    defs
}

fn record_member_def(
    ins: &Instr,
    group: &BTreeSet<Local>,
    defs: &mut BTreeMap<Local, Vec<DefKind>>,
    top: bool,
) {
    match ins {
        Instr::StackAlloc {  dst , ..} => {
            if group.contains(dst) {
                defs.entry(*dst).or_default().push(DefKind::Alloc);
            }
        }
        Instr::Copy {  dst, src , ..} => {
            if group.contains(dst) {
                let kind = if top && group.contains(src) {
                    DefKind::AliasCopy
                } else {
                    DefKind::Other
                };
                defs.entry(*dst).or_default().push(kind);
            }
        }
        Instr::Call {  dsts, err , ..} => {
            for d in dsts {
                if group.contains(d) {
                    defs.entry(*d).or_default().push(DefKind::Other);
                }
            }
            if let Some(e) = err {
                if group.contains(e) {
                    defs.entry(*e).or_default().push(DefKind::Other);
                }
            }
        }
        _ => {
            if let Some(dst) = instr_dst(ins) {
                if group.contains(&dst) {
                    defs.entry(dst).or_default().push(DefKind::Other);
                }
            }
        }
    }
}

fn instr_ok(
    ins: &Instr,
    group: &BTreeSet<Local>,
    classes: &[ClassDesc],
    class: usize,
) -> bool {
    match ins {
        Instr::StackAlloc { ..} | Instr::Copy { ..} => true,
        Instr::Retain {  obj , ..} | Instr::Release { obj , ..} => !group.contains(obj),
        _ => {
            if let Some((obj, field)) = access_field(ins) {
                if group.contains(&obj) {
                    return field_of(classes, class, &field).is_some();
                }
                if let Instr::SetField {  value , ..} | Instr::SetFieldByName { value , ..} = ins {
                    if group.contains(value) {
                        return false;
                    }
                }
                return true;
            }
            let mut tmp: Vec<Local> = Vec::new();
            mentioned(ins, &mut tmp);
            !tmp.iter().any(|l| group.contains(l))
        }
    }
}

fn uses_ok(
    func: &Function,
    group: &BTreeSet<Local>,
    classes: &[ClassDesc],
    class: usize,
) -> bool {
    for block in &func.blocks {
        for ins in &block.instrs {
            if !instr_ok(ins, group, classes, class) {
                return false;
            }
            if let Instr::Defer {  body , ..} = ins {
                for nested in body {
                    if !instr_ok(nested, group, classes, class) {
                        return false;
                    }
                }
            }
        }
        match &block.term {
            Terminator::Ret(v) => {
                if v.iter().any(|l| group.contains(l)) {
                    return false;
                }
            }
            Terminator::Throw { src, .. } => {
                if group.contains(src) {
                    return false;
                }
            }
            Terminator::Rethrow { err, .. } => {
                if group.contains(err) {
                    return false;
                }
            }
            Terminator::BrErr { err, catch_bind, .. } => {
                if group.contains(err) || group.contains(catch_bind) {
                    return false;
                }
            }
            _ => {}
        }
    }
    true
}

pub fn scalar_replace_aggregates(module: &mut Module, fi: usize) -> bool {
    let mut candidates: Vec<(Local, usize)> = Vec::new();
    {
        let func = &module.functions[fi];
        for block in &func.blocks {
            for ins in &block.instrs {
                if let Instr::StackAlloc {  dst, class , ..} = ins {
                    candidates.push((*dst, *class));
                }
            }
        }
    }
    let mut changed = false;
    for (alloc, class) in candidates {
        if scalar_replace_one(module, fi, alloc, class) {
            changed = true;
        }
    }
    changed
}

fn scalar_replace_one(module: &mut Module, fi: usize, alloc: Local, class: usize) -> bool {
    let field_types: Vec<LirType> = {
        let desc = match module.classes.get(class) {
            Some(d) => d,
            None => return false,
        };
        if desc.deinit.is_some()
            || desc.dtor.is_some_and(|id| {
                !crate::escape::is_trivial_dtor(
                    &module.functions,
                    id,
                )
            })
        {
            return false;
        }
        if !desc.fields.iter().all(|f| is_scalar_field(&f.ty)) {
            return false;
        }
        desc.fields.iter().map(|f| f.ty.clone()).collect()
    };
    let (group, defs_ok) = {
        let func = &module.functions[fi];
        let alias = collect_aliases(func);
        let group = group_of(func, &alias, alloc);
        let defs = member_defs(func, &group);
        let mut ok = true;
        for member in &group {
            match defs.get(member).map(|v| v.as_slice()) {
                Some([DefKind::Alloc]) if *member == alloc => {}
                Some([DefKind::Alloc, rest @ ..])
                    if *member == alloc && rest.iter().all(|k| *k == DefKind::AliasCopy) => {}
                Some([DefKind::AliasCopy]) => {}
                _ => {
                    ok = false;
                    break;
                }
            }
        }
        (group, ok)
    };
    if !defs_ok {
        return false;
    }
    {
        let func = &module.functions[fi];
        if !uses_ok(func, &group, &module.classes, class) {
            return false;
        }
    }
    let mut field_locals: Vec<Local> = Vec::new();
    {
        let func = &mut module.functions[fi];
        for ty in &field_types {
            let id = func.locals.len() as Local;
            func.locals.push(ty.clone());
            field_locals.push(id);
        }
        for block in &mut func.blocks {
            let mut rewritten: Vec<Instr> = Vec::with_capacity(block.instrs.len());
            for ins in block.instrs.drain(..) {
                match ins {
                    Instr::StackAlloc {  dst , ..} if dst == alloc => {}
                    Instr::GetField { span, dst, obj, field } if group.contains(&obj) => {
                        rewritten.push(Instr::Copy { span, dst, src: field_locals[field] });
                    }
                    Instr::GetFieldByName { span, dst, obj, field } if group.contains(&obj) => {
                        match field_of(&module.classes, class, &ObjField::Name(field.clone())) {
                            Some(i) => rewritten.push(Instr::Copy { span, dst, src: field_locals[i] }),
                            None => rewritten.push(Instr::GetFieldByName { span, dst, obj, field }),
                        }
                    }
                    Instr::SetField { span, obj, field, value } if group.contains(&obj) => {
                        rewritten.push(Instr::Copy { span, dst: field_locals[field], src: value });
                    }
                    Instr::SetFieldByName { span, obj, field, value } if group.contains(&obj) => {
                        match field_of(&module.classes, class, &ObjField::Name(field.clone())) {
                            Some(i) => {
                                rewritten.push(Instr::Copy { span, dst: field_locals[i], src: value })
                            }
                            None => rewritten.push(Instr::SetFieldByName { span, obj, field, value }),
                        }
                    }
                    _ => rewritten.push(ins),
                }
            }
            block.instrs = rewritten;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_module() -> Module {
        let mut module = Module::default();
        module.classes.push(ClassDesc {
            name: "C".to_string(),
            type_params: Vec::new(),
            is_struct: false,
            fields: vec![
                FieldDesc { name: "x".to_string(), ty: LirType::I64, private: false, owner: "C".to_string() },
                FieldDesc { name: "y".to_string(), ty: LirType::I64, private: false, owner: "C".to_string() },
            ],
            field_index: BTreeMap::from([
                ("x".to_string(), 0),
                ("y".to_string(), 1),
            ]),
            methods: BTreeMap::new(),
            deinit: None,
            dtor: None,
            ifaces: Vec::new(),
            parent: None,
        });
        module.functions.push(Function {
            name: "Main".to_string(),
            params: Vec::new(),
            sig_params: Vec::new(),
            ret: LirType::I64,
            throws: false,
            is_unsafe: false,
            method_self: false,
            is_pub: false,
            is_closure: false,
            locals: vec![LirType::Obj("C".to_string()), LirType::I64, LirType::I64],
            blocks: vec![Block {
                instrs: vec![
                    Instr::StackAlloc {span: UNKNOWN_SPAN,  dst: 0, class: 0, instance_size: 32 },
                    Instr::Const {span: UNKNOWN_SPAN,  dst: 1, lit: Lit::Int(10) },
                    Instr::SetFieldByName {span: UNKNOWN_SPAN, 
                        obj: 0,
                        field: "x".to_string(),
                        value: 1,
                    },
                    Instr::GetFieldByName {span: UNKNOWN_SPAN,  dst: 2, obj: 0, field: "x".to_string() },
                ],
                term: Terminator::Ret(vec![2]),
            }],
        });
        module.fn_index.insert("Main".to_string(), 0);
        module
    }

    #[test]
    fn decomposes_field_accesses() {
        let mut module = test_module();
        assert!(scalar_replace_aggregates(&mut module, 0));
        let f = &module.functions[0];
        assert!(!f.blocks.iter().flat_map(|b| b.instrs.iter()).any(|i| {
            matches!(i, Instr::StackAlloc { ..} | Instr::ObjNew { ..})
        }));
        assert!(!f.blocks.iter().flat_map(|b| b.instrs.iter()).any(|i| {
            matches!(
                i,
                Instr::GetField { ..}
                    | Instr::GetFieldByName { ..}
                    | Instr::SetField { ..}
                    | Instr::SetFieldByName { ..}
            )
        }));
        assert!(f.blocks.iter().flat_map(|b| b.instrs.iter()).any(|i| {
            matches!(i, Instr::Copy { ..})
        }));
    }

    #[test]
    fn disqualifies_on_call_arg() {
        let mut module = test_module();
        module.functions[0].blocks[0].instrs.push(Instr::Call {span: UNKNOWN_SPAN,
            dsts: vec![],
            err: None,
            target: CallTarget::Builtin("print".to_string()),
            args: vec![0],
        });
        assert!(!scalar_replace_aggregates(&mut module, 0));
        assert!(module.functions[0]
            .blocks
            .iter()
            .flat_map(|b| b.instrs.iter())
            .any(|i| matches!(i, Instr::StackAlloc { ..})));
    }

    #[test]
    fn disqualifies_ref_fields() {
        let mut module = test_module();
        module.classes[0].fields[0].ty = LirType::Obj("D".to_string());
        assert!(!scalar_replace_aggregates(&mut module, 0));
    }
}
