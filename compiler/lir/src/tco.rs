use crate::instr::*;
use crate::opt::rewrite_target;

fn body_allowed(ins: &Instr) -> bool {
    match ins {
        Instr::Const { ..}
        | Instr::Copy { ..}
        | Instr::Cast { ..}
        | Instr::Convert { ..}
        | Instr::Arith { ..}
        | Instr::Cmp { ..}
        | Instr::Not { ..}
        | Instr::Neg { ..}
        | Instr::Concat { ..}
        | Instr::ToStr { ..}
        | Instr::Range { ..}
        | Instr::Stride { ..}
        | Instr::ArrayNew { ..}
        | Instr::ArrayLen { ..}
        | Instr::ArrayGet { ..}
        | Instr::EnumPayload { ..}
        | Instr::EnumTag { ..}
        | Instr::Extract { ..}
        | Instr::RangeLo { ..}
        | Instr::RangeHi { ..}
        | Instr::RangeStep { ..}
        | Instr::GetField { ..}
        | Instr::GetFieldByName { ..}
        | Instr::ObjNew { ..}
        | Instr::StackAlloc { ..}
        | Instr::Call { ..} => true,
        _ => false,
    }
}

fn function_eligible(func: &Function) -> bool {
    for block in &func.blocks {
        for ins in &block.instrs {
            if !body_allowed(ins) {
                return false;
            }
        }
    }
    true
}

struct TailSite {
    bi: usize,
    args: Vec<Local>,
}

fn tail_sites(func: &Function, self_id: usize) -> Vec<TailSite> {
    let mut out: Vec<TailSite> = Vec::new();
    for (bi, block) in func.blocks.iter().enumerate() {
        let ret_val = match &block.term {
            Terminator::Ret(v) => v.clone(),
            _ => continue,
        };
        let Some(last) = block.instrs.last() else {
            continue;
        };
        if let Instr::Call { span: _, dsts, err: None, target, args, .. } = last {
            let callee = match target {
                CallTarget::Fn(id) => Some(*id),
                CallTarget::Method { method, .. } => Some(*method),
                _ => None,
            };
            if callee != Some(self_id) {
                continue;
            }
            if *dsts != ret_val {
                continue;
            }
            out.push(TailSite { bi, args: args.clone() });
        }
    }
    out
}

fn synthesize_header(func: &mut Function) -> BlockId {
    let header = func.blocks.len();
    let moved_instrs = std::mem::take(&mut func.blocks[0].instrs);
    let moved_term =
        std::mem::replace(&mut func.blocks[0].term, Terminator::Br(header));
    func.blocks.push(Block { instrs: moved_instrs, term: moved_term });
    for block in func.blocks.iter_mut() {
        rewrite_target(&mut block.term, 0, header);
    }
    header
}

fn transform_site(func: &mut Function, site: &TailSite, header: BlockId) {
    let tys: Vec<LirType> = site
        .args
        .iter()
        .map(|a| func.locals.get(*a as usize).cloned().unwrap_or(LirType::Any))
        .collect();
    let mut temps: Vec<Local> = Vec::new();
    for ty in &tys {
        let t = func.locals.len() as Local;
        func.locals.push(ty.clone());
        temps.push(t);
    }
    let param_count = func.params.len();
    let args = site.args.clone();
    let block = &mut func.blocks[site.bi];
    let call_span = block.instrs.pop().map(|ins| ins.span()).unwrap_or(UNKNOWN_SPAN);
    for (t, a) in temps.iter().zip(args.iter()) {
        block.instrs.push(Instr::Copy { span: call_span, dst: *t, src: *a });
    }
    for (p, t) in temps.iter().enumerate() {
        if p < param_count {
            block.instrs.push(Instr::Copy { span: call_span, dst: p as Local, src: *t });
        }
    }
    block.term = Terminator::Br(header);
}

pub fn optimize_tail_calls(func: &mut Function, self_id: usize) -> bool {
    if !function_eligible(func) {
        return false;
    }
    let sites = tail_sites(func, self_id);
    if sites.is_empty() {
        return false;
    }
    for site in &sites {
        if site.args.len() != func.params.len() {
            return false;
        }
    }
    let header = synthesize_header(func);
    for site in &sites {
        transform_site(func, site, header);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recursive_sum() -> (Module, usize) {
        let mut module = Module::default();
        module.functions.push(Function {
            name: "sumDown".to_string(),
            params: vec![LirType::I64, LirType::I64],
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
                        Instr::Const {span: UNKNOWN_SPAN,  dst: 2, lit: Lit::Int(0) },
                        Instr::Cmp {span: UNKNOWN_SPAN,  
                            op: CmpOp::LtEq,
                            kind: NumKind::Int,
                            dst: 3,
                            lhs: 0,
                            rhs: 2},
                    ],
                    term: Terminator::BrIf {span: UNKNOWN_SPAN,  cond: 3, then_bb: 1, else_bb: 2 },
                },
                Block {
                    instrs: Vec::new(),
                    term: Terminator::Ret(vec![1]),
                },
                Block {
                    instrs: vec![
                        Instr::Const {span: UNKNOWN_SPAN,  dst: 4, lit: Lit::Int(1) },
                        Instr::Arith {span: UNKNOWN_SPAN,  
                            op: ArithOp::Sub,
                            kind: NumKind::Int,
                            dst: 5,
                            lhs: 0,
                            rhs: 4},
                        Instr::Arith {span: UNKNOWN_SPAN,  
                            op: ArithOp::Add,
                            kind: NumKind::Int,
                            dst: 6,
                            lhs: 1,
                            rhs: 0},
                        Instr::Call {span: UNKNOWN_SPAN,  
                            dsts: vec![7],
                            err: None,
                            target: CallTarget::Fn(0),
                            args: vec![5, 6]},
                    ],
                    term: Terminator::Ret(vec![7]),
                },
            ],
        });
        module.fn_index.insert("sumDown".to_string(), 0);
        (module, 0)
    }

    #[test]
    fn rewrites_self_tail_call_to_loop() {
        let (mut module, id) = recursive_sum();
        assert!(optimize_tail_calls(&mut module.functions[id], id));
        let f = &module.functions[id];
        assert!(!f.blocks.iter().flat_map(|b| b.instrs.iter()).any(|i| {
            matches!(i, Instr::Call { ..})
        }));
        let backs: usize = f
            .blocks
            .iter()
            .filter(|b| matches!(&b.term, Terminator::Br(t) if *t < f.blocks.len()))
            .count();
        assert!(backs >= 2);
    }

    #[test]
    fn preserves_non_tail_call() {
        let (mut module, id) = recursive_sum();
        module.functions[id].blocks[2].instrs.push(Instr::Arith {span: UNKNOWN_SPAN,
            op: ArithOp::Add,
            kind: NumKind::Int,
            dst: 7,
            lhs: 7,
            rhs: 7,
        });
        module.functions[id].blocks[2].term = Terminator::Ret(vec![7]);
        assert!(!optimize_tail_calls(&mut module.functions[id], id));
    }
}
