use super::*;

pub(super) fn lower_control(cx: &mut FnCx, lir: &Module, ins: &Instr, fname: &str) -> Result<(), Diagnostic> {
    let _ = fname;
    let _ = lir;
    match ins {
        Instr::Defer { body, .. } => {
            let id = cx
                .defers
                .iter()
                .position(|x| x == body)
                .ok_or_else(|| Diagnostic::new(Code::E108, "llvm subset: unknown defer"))?;
            let arg = cx.context.i64_type().const_int(id as u64, false);
            cx.builder.build_call(cx.defer_push, &[arg.into()], "").map_err(err)?;
        }
        Instr::RunDefers { keep, .. } => {
            emit_run_defers(cx, lir, *keep)?;
        }
        _ => return Err(Diagnostic::new(Code::E108, "llvm subset: unsupported instr")),
    }
    Ok(())
}
