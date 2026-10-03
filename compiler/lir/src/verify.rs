use crate::instr::*;
use crate::opt::{instr_reads, term_reads};
use diagnostics::{Code, Diagnostic};

pub fn verify(module: &Module) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    for (fi, f) in module.functions.iter().enumerate() {
        if f.blocks.is_empty() {
            diags.push(Diagnostic::new(
                Code::E108,
                format!("function `{}` has no blocks", f.name),
            ));
            continue;
        }
        for b in f.blocks.iter() {
            for ins in &b.instrs {
                check_instr(module, fi, ins, f.locals.len(), &mut diags);
            }
            check_term(module, f, &b.term, &mut diags);
        }
        check_no_tuple_escape(f, &mut diags);
    }
    diags
}

fn local_ok(n: usize, l: Local, what: &str, diags: &mut Vec<Diagnostic>) {
    if (l as usize) >= n {
        diags.push(Diagnostic::new(
            Code::E108,
            format!("local %{l} out of range in {what}"),
        ));
    }
}

fn block_ok(f: &Function, bb: BlockId, what: &str, diags: &mut Vec<Diagnostic>) {
    if bb >= f.blocks.len() {
        diags.push(Diagnostic::new(
            Code::E108,
            format!("block {bb} out of range in {what} of `{}`", f.name),
        ));
    }
}

fn check_instr(
    module: &Module,
    fi: usize,
    ins: &Instr,
    nlocals: usize,
    diags: &mut Vec<Diagnostic>,
) {
    let fname = &module.functions[fi].name;
    let what = format!("`{fname}`");

    match ins {
        Instr::Const { dst, .. } => local_ok(nlocals, *dst, &what, diags),
        Instr::Copy {  dst, src, ..} => {
            local_ok(nlocals, *dst, &what, diags);
            local_ok(nlocals, *src, &what, diags);
        }
        Instr::Cast {  dst, src, ..} => {
            local_ok(nlocals, *dst, &what, diags);
            local_ok(nlocals, *src, &what, diags);
        }
        Instr::Arith { dst, lhs, rhs, .. } | Instr::Cmp { dst, lhs, rhs, .. } => {
            local_ok(nlocals, *dst, &what, diags);
            local_ok(nlocals, *lhs, &what, diags);
            local_ok(nlocals, *rhs, &what, diags);
        }
        Instr::Fma { dst, a, b, c, .. } => {
            local_ok(nlocals, *dst, &what, diags);
            local_ok(nlocals, *a, &what, diags);
            local_ok(nlocals, *b, &what, diags);
            local_ok(nlocals, *c, &what, diags);
        }
        Instr::Not {  dst, src, ..} | Instr::Neg { dst, src, .. } | Instr::ToStr {  dst, src, ..} => {
            local_ok(nlocals, *dst, &what, diags);
            local_ok(nlocals, *src, &what, diags);
        }
        Instr::Convert { dst, src, .. } => {
            local_ok(nlocals, *dst, &what, diags);
            local_ok(nlocals, *src, &what, diags);
        }
        Instr::Concat {  dst, lhs, rhs, ..} => {
            local_ok(nlocals, *dst, &what, diags);
            local_ok(nlocals, *lhs, &what, diags);
            local_ok(nlocals, *rhs, &what, diags);
        }
        Instr::Range { dst, lo, hi, .. } => {
            local_ok(nlocals, *dst, &what, diags);
            local_ok(nlocals, *lo, &what, diags);
            local_ok(nlocals, *hi, &what, diags);
        }
        Instr::Stride {  dst, range, step, ..} => {
            local_ok(nlocals, *dst, &what, diags);
            local_ok(nlocals, *range, &what, diags);
            local_ok(nlocals, *step, &what, diags);
        }
        Instr::ArrayNew { dst, .. } => local_ok(nlocals, *dst, &what, diags),
        Instr::ArrayPush { arr, value, .. } => {
            local_ok(nlocals, *arr, &what, diags);
            local_ok(nlocals, *value, &what, diags);
        }
        Instr::ArrayPop {  dst, arr, ..} => {
            local_ok(nlocals, *dst, &what, diags);
            local_ok(nlocals, *arr, &what, diags);
        }
        Instr::ArrayLen {  dst, arr, ..} => {
            local_ok(nlocals, *dst, &what, diags);
            local_ok(nlocals, *arr, &what, diags);
        }
        Instr::ArrayGet { dst, arr, index, .. } => {
            local_ok(nlocals, *dst, &what, diags);
            local_ok(nlocals, *arr, &what, diags);
            local_ok(nlocals, *index, &what, diags);
        }
        Instr::ArraySet { arr, index, value, .. } => {
            local_ok(nlocals, *arr, &what, diags);
            local_ok(nlocals, *index, &what, diags);
            local_ok(nlocals, *value, &what, diags);
        }
        Instr::EnumNew {  dst, enu, variant, payload, ..} => {
            local_ok(nlocals, *dst, &what, diags);
            match module.enums.get(*enu) {
                Some(e) if *variant < e.variants.len() => {
                    if payload.len() != e.variants[*variant].payload.len() {
                        diags.push(Diagnostic::new(Code::E108, "enum payload arity mismatch"));
                    }
                }
                _ => diags.push(Diagnostic::new(Code::E108, "enum id out of range")),
            }
            for c in payload {
                local_ok(nlocals, *c, &what, diags);
            }
        }
        Instr::EnumPayload { dst, scrut, .. } => {
            local_ok(nlocals, *dst, &what, diags);
            local_ok(nlocals, *scrut, &what, diags);
        }
        Instr::EnumTag {  dst, scrut, ..} => {
            local_ok(nlocals, *dst, &what, diags);
            local_ok(nlocals, *scrut, &what, diags);
        }
        Instr::Extract { dst, base, .. } => {
            local_ok(nlocals, *dst, &what, diags);
            local_ok(nlocals, *base, &what, diags);
        }
        Instr::AddrOf {  dst, src, ..} => {
            local_ok(nlocals, *dst, &what, diags);
            local_ok(nlocals, *src, &what, diags);
        }
        Instr::PtrLoad {  dst, ptr, ..} => {
            local_ok(nlocals, *dst, &what, diags);
            local_ok(nlocals, *ptr, &what, diags);
        }
        Instr::PtrStore {  ptr, val, ..} => {
            local_ok(nlocals, *ptr, &what, diags);
            local_ok(nlocals, *val, &what, diags);
        }
        Instr::ObjNew {  dst, class, instance_size: size, ..} => {
            local_ok(nlocals, *dst, &what, diags);
            match module.classes.get(*class) {
                None => diags.push(Diagnostic::new(Code::E108, "class id out of range")),
                Some(c) => {
                    if *size != instance_size(c.fields.len()) {
                        diags.push(Diagnostic::new(Code::E108, "instance size mismatch"));
                    }
                }
            }
        }
        Instr::StackAlloc {  dst, class, instance_size: size, ..} => {
            local_ok(nlocals, *dst, &what, diags);
            match module.classes.get(*class) {
                None => diags.push(Diagnostic::new(Code::E108, "class id out of range")),
                Some(c) => {
                    if *size != instance_size(c.fields.len()) {
                        diags.push(Diagnostic::new(Code::E108, "instance size mismatch"));
                    }
                }
            }
        }
        Instr::RangeLo {  dst, range, ..}
        | Instr::RangeHi {  dst, range, ..}
        | Instr::RangeStep {  dst, range, ..} => {
            local_ok(nlocals, *dst, &what, diags);
            local_ok(nlocals, *range, &what, diags);
        }
        Instr::GetField { dst, obj, .. } => {
            local_ok(nlocals, *dst, &what, diags);
            local_ok(nlocals, *obj, &what, diags);
        }
        Instr::GetFieldByName { dst, obj, .. } => {
            local_ok(nlocals, *dst, &what, diags);
            local_ok(nlocals, *obj, &what, diags);
        }
        Instr::SetField { obj, value, .. } => {
            local_ok(nlocals, *obj, &what, diags);
            local_ok(nlocals, *value, &what, diags);
        }
        Instr::SetFieldByName { obj, value, .. } => {
            local_ok(nlocals, *obj, &what, diags);
            local_ok(nlocals, *value, &what, diags);
        }
        Instr::ClosureNew {
            dst,
            func,
            captures,
            ..
        } => {
            local_ok(nlocals, *dst, &what, diags);
            if *func >= module.functions.len() {
                diags.push(Diagnostic::new(Code::E108, "closure func out of range"));
            }
            for c in captures {
                local_ok(nlocals, *c, &what, diags);
            }
        }
        Instr::Call {  dsts, err, target, args, ..} => {
            for d in dsts {
                local_ok(nlocals, *d, &what, diags);
            }
            if let Some(e) = err {
                local_ok(nlocals, *e, &what, diags);
            }
            match target {
                CallTarget::Fn(id) => {
                    if *id >= module.functions.len() {
                        diags.push(Diagnostic::new(Code::E108, "call target out of range"));
                    }
                }
                CallTarget::Method { class, .. } => {
                    if *class >= module.classes.len() {
                        diags.push(Diagnostic::new(Code::E108, "method class out of range"));
                    }
                }
                CallTarget::Value(v) => local_ok(nlocals, *v, &what, diags),
                CallTarget::Dyn { obj, .. } => local_ok(nlocals, *obj, &what, diags),
                CallTarget::Builtin(_) => {}
                CallTarget::Foreign { .. } => {}
            }
            for a in args {
                local_ok(nlocals, *a, &what, diags);
            }
        }
        Instr::GenRefOf {  dst, obj, ..} => {
            local_ok(nlocals, *dst, &what, diags);
            local_ok(nlocals, *obj, &what, diags);
        }
        Instr::GenRefEmpty {  dst, ..} => local_ok(nlocals, *dst, &what, diags),
        Instr::GenRefGet {  dst, gref, ..} => {
            local_ok(nlocals, *dst, &what, diags);
            local_ok(nlocals, *gref, &what, diags);
        }
        Instr::Retain {  obj, ..} | Instr::Release {  obj, ..} | Instr::ReleaseAs {  obj, ..} => local_ok(nlocals, *obj, &what, diags),
        Instr::GenRefInvalidate {  obj, ..} => local_ok(nlocals, *obj, &what, diags),
        Instr::ThreadSpawn { dst, .. } => local_ok(nlocals, *dst, &what, diags),
        Instr::ThreadJoin {  dst, handle, ..} => {
            local_ok(nlocals, *dst, &what, diags);
            local_ok(nlocals, *handle, &what, diags);
        }
        Instr::PoolInit {  dst, id, workers, ..} => {
            local_ok(nlocals, *dst, &what, diags);
            local_ok(nlocals, *id, &what, diags);
            local_ok(nlocals, *workers, &what, diags);
        }
        Instr::PoolSubmit { pool, arg, .. } => {
            local_ok(nlocals, *pool, &what, diags);
            if let Some(a) = arg {
                local_ok(nlocals, *a, &what, diags);
            }
        }
        Instr::PoolParallelFor { pool, start, end, chunk, .. } => {
            local_ok(nlocals, *pool, &what, diags);
            local_ok(nlocals, *start, &what, diags);
            local_ok(nlocals, *end, &what, diags);
            local_ok(nlocals, *chunk, &what, diags);
        }
        Instr::PoolJoin {  pool, ..} | Instr::PoolShutdown {  pool, ..} => {
            local_ok(nlocals, *pool, &what, diags);
        }
        Instr::VecNew { dst, x, y, z, w, .. } => {
            local_ok(nlocals, *dst, &what, diags);
            local_ok(nlocals, *x, &what, diags);
            local_ok(nlocals, *y, &what, diags);
            local_ok(nlocals, *z, &what, diags);
            local_ok(nlocals, *w, &what, diags);
        }
        Instr::VecSplat { dst, val, .. } => {
            local_ok(nlocals, *dst, &what, diags);
            local_ok(nlocals, *val, &what, diags);
        }
        Instr::VecExtract {  dst, vec, lane, ..} => {
            local_ok(nlocals, *dst, &what, diags);
            local_ok(nlocals, *vec, &what, diags);
            local_ok(nlocals, *lane, &what, diags);
        }
        Instr::VecInsert { dst, vec, val, .. } => {
            local_ok(nlocals, *dst, &what, diags);
            local_ok(nlocals, *vec, &what, diags);
            local_ok(nlocals, *val, &what, diags);
        }
        Instr::VecArith { dst, lhs, rhs, .. } => {
            local_ok(nlocals, *dst, &what, diags);
            local_ok(nlocals, *lhs, &what, diags);
            local_ok(nlocals, *rhs, &what, diags);
        }
        Instr::VecUnary { dst, src, .. } => {
            local_ok(nlocals, *dst, &what, diags);
            local_ok(nlocals, *src, &what, diags);
        }
        Instr::VecDot {  dst, lhs, rhs, ..} => {
            local_ok(nlocals, *dst, &what, diags);
            local_ok(nlocals, *lhs, &what, diags);
            local_ok(nlocals, *rhs, &what, diags);
        }
        Instr::ReleaseField { obj, .. } => local_ok(nlocals, *obj, &what, diags),
        Instr::Defer {  body, ..} => {
            for ins in body {
                check_instr(module, fi, ins, nlocals, diags);
            }
        }
        Instr::RunDefers { .. } => {}
        Instr::Assert {  cond, message, ..} => {
            local_ok(nlocals, *cond, &what, diags);
            local_ok(nlocals, *message, &what, diags);
        }
        Instr::Panic {  message, ..} => local_ok(nlocals, *message, &what, diags),
    }
}

fn check_term(
    _module: &Module,
    f: &Function,
    term: &Terminator,
    diags: &mut Vec<Diagnostic>,
) {
    let what = format!("terminator in `{}`", f.name);
    let n = f.locals.len();

    match term {
        Terminator::Ret(v) => {
            for l in v {
                local_ok(n, *l, &what, diags);
            }
        }
        Terminator::Br(bb) => block_ok(f, *bb, &what, diags),
        Terminator::BrIf {
            span: _,
            cond,
            then_bb,
            else_bb,
        } => {
            local_ok(n, *cond, &what, diags);
            block_ok(f, *then_bb, &what, diags);
            block_ok(f, *else_bb, &what, diags);
        }
        Terminator::BrErr {
            err,
            catch_bb,
            catch_bind,
            next_bb,
            ..
        } => {
            local_ok(n, *err, &what, diags);
            local_ok(n, *catch_bind, &what, diags);
            block_ok(f, *catch_bb, &what, diags);
            block_ok(f, *next_bb, &what, diags);
        }
        Terminator::Switch {
            span: _,
            scrut,
            cases,
            default,
        } => {
            local_ok(n, *scrut, &what, diags);
            for (pat, bb) in cases {
                if let SwitchPat::Is { source, .. } = pat {
                    local_ok(n, *source, &what, diags);
                }
                block_ok(f, *bb, &what, diags);
            }
            block_ok(f, *default, &what, diags);
        }
        Terminator::Throw { span: _, src, catch } => {
            local_ok(n, *src, &what, diags);
            if let Some((bb, e, _)) = catch {
                block_ok(f, *bb, &what, diags);
                local_ok(n, *e, &what, diags);
            }
        }
        Terminator::Rethrow { catch_bb, err, .. } => {
            block_ok(f, *catch_bb, &what, diags);
            local_ok(n, *err, &what, diags);
        }
        Terminator::Unreachable { span } => {
            if *span == UNKNOWN_SPAN {
                diags.push(Diagnostic::new(
                    Code::E108,
                    format!("unreachable terminator without a source span in `{}`", f.name),
                ));
            }
        }
    }
}

fn check_no_tuple_escape(f: &Function, diags: &mut Vec<Diagnostic>) {
    let mut tmp = Vec::new();
    let check = |l: Local, diags: &mut Vec<Diagnostic>| {
        if matches!(f.locals.get(l as usize), Some(LirType::Tuple(_))) {
            diags.push(Diagnostic::new(
                Code::E108,
                "tuple value cannot be used here; destructure it or index an element first",
            ));
        }
    };
    for b in f.blocks.iter() {
        for ins in &b.instrs {
            tmp.clear();
            instr_reads(ins, &mut tmp);
            for l in tmp.drain(..) {
                check(l, diags);
            }
        }
        tmp.clear();
        term_reads(&b.term, &mut tmp);
        for l in tmp.drain(..) {
            check(l, diags);
        }
    }
}
