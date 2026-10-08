use super::*;

impl<M: Module> FnLower<'_, M> {
    pub(super) fn lower_thread_spawn(&mut self, b: &mut FunctionBuilder<'_>, ins: &Instr) -> Result<(), String> {
        let Instr::ThreadSpawn { dst, func, closure, ret_tag, ..} = ins else {
            return Err("unreachable".to_string());
        };
                if let Some(l) = closure {
                    let bx = self.val(b, *l);
                    let tag = b.ins().iconst(types::I64, *ret_tag as i64);
                    let callee = self.module.declare_func_in_func(self.spawn_closure, &mut b.func);
                    let inst = b.ins().call(callee, &[bx, tag]);
                    let v = b.inst_results(inst)[0];
                    self.set(b, *dst, v);
                    return Ok(());
                }
                let fname = self.lir.functions[*func].name.clone();
                let id = *self.ids.get(&fname).ok_or("unknown thread fn".to_string())?;
                let fr = self.module.declare_func_in_func(id, &mut b.func);
                let entry = b.ins().func_addr(types::I64, fr);
                let zero = b.ins().iconst(types::I64, 0);
                let callee = self.module.declare_func_in_func(self.thread_spawn, &mut b.func);
                let inst = b.ins().call(callee, &[entry, zero]);
                let v = b.inst_results(inst)[0];
                self.set(b, *dst, v);
        Ok(())
    }

    pub(super) fn lower_thread_join(&mut self, b: &mut FunctionBuilder<'_>, ins: &Instr) -> Result<(), String> {
        let Instr::ThreadJoin { dst, handle , ..} = ins else {
            return Err("unreachable".to_string());
        };
                let callee = self.module.declare_func_in_func(self.thread_join, &mut b.func);
                let h = self.val(b, *handle);
                let inst = b.ins().call(callee, &[h]);
                let v = b.inst_results(inst)[0];
                self.set(b, *dst, v);
        Ok(())
    }

    pub(super) fn lower_pool_init(&mut self, b: &mut FunctionBuilder<'_>, ins: &Instr) -> Result<(), String> {
        let Instr::PoolInit { dst, id, workers , ..} = ins else {
            return Err("unreachable".to_string());
        };
                let callee = self.module.declare_func_in_func(self.pool_init, &mut b.func);
                let i = self.val(b, *id);
                let w = self.val(b, *workers);
                b.ins().call(callee, &[i, w]);
                let v = self.val(b, *id);
                self.set(b, *dst, v);
        Ok(())
    }

    pub(super) fn lower_pool_submit(&mut self, b: &mut FunctionBuilder<'_>, ins: &Instr) -> Result<(), String> {
        let Instr::PoolSubmit { dst, pool, func, arg, closure, ret_tag, ..} = ins else {
            return Err("unreachable".to_string());
        };
                let p = self.val(b, *pool);
                let tag = b.ins().iconst(types::I64, *ret_tag as i64);
                if let Some(l) = closure {
                    let bx = self.val(b, *l);
                    let (a, has) = match arg {
                        Some(x) => (self.val(b, *x), b.ins().iconst(types::I64, 1)),
                        None => (b.ins().iconst(types::I64, 0), b.ins().iconst(types::I64, 0)),
                    };
                    let callee = self.module.declare_func_in_func(self.submit_closure, &mut b.func);
                    let inst = b.ins().call(callee, &[p, bx, a, has, tag]);
                    let v = b.inst_results(inst)[0];
                    self.set(b, *dst, v);
                    return Ok(());
                }
                let fname = self.lir.functions[*func].name.clone();
                let id = *self.ids.get(&fname).ok_or("unknown pool fn".to_string())?;
                let fr = self.module.declare_func_in_func(id, &mut b.func);
                let entry = b.ins().func_addr(types::I64, fr);
                let (a, has) = match arg {
                    Some(l) => (self.val(b, *l), b.ins().iconst(types::I64, 1)),
                    None => (b.ins().iconst(types::I64, 0), b.ins().iconst(types::I64, 0)),
                };
                let callee = self.module.declare_func_in_func(self.submit_handle, &mut b.func);
                let inst = b.ins().call(callee, &[p, entry, a, has, tag]);
                let v = b.inst_results(inst)[0];
                self.set(b, *dst, v);
        Ok(())
    }

    pub(super) fn lower_pool_parallel_for(&mut self, b: &mut FunctionBuilder<'_>, ins: &Instr) -> Result<(), String> {
        let Instr::PoolParallelFor { pool, start, end, chunk, func, closure, ..} = ins else {
            return Err("unreachable".to_string());
        };
                let p = self.val(b, *pool);
                let s = self.val(b, *start);
                let e = self.val(b, *end);
                let c = self.val(b, *chunk);
                if let Some(l) = closure {
                    let bx = self.val(b, *l);
                    let callee = self.module.declare_func_in_func(self.parallel_closure, &mut b.func);
                    b.ins().call(callee, &[p, bx, s, e, c]);
                    return Ok(());
                }
                let fname = self.lir.functions[*func].name.clone();
                let id = *self.ids.get(&fname).ok_or("unknown pool fn".to_string())?;
                let fr = self.module.declare_func_in_func(id, &mut b.func);
                let entry = b.ins().func_addr(types::I64, fr);
                let p = self.val(b, *pool);
                let s = self.val(b, *start);
                let e = self.val(b, *end);
                let c = self.val(b, *chunk);
                let callee = self.module.declare_func_in_func(self.pool_parallel_for, &mut b.func);
                b.ins().call(callee, &[p, entry, s, e, c]);
        Ok(())
    }

    pub(super) fn lower_pool_join(&mut self, b: &mut FunctionBuilder<'_>, ins: &Instr) -> Result<(), String> {
        let Instr::PoolJoin { pool , ..} = ins else {
            return Err("unreachable".to_string());
        };
                let callee = self.module.declare_func_in_func(self.pool_join, &mut b.func);
                let p = self.val(b, *pool);
                b.ins().call(callee, &[p]);
        Ok(())
    }

    pub(super) fn lower_pool_shutdown(&mut self, b: &mut FunctionBuilder<'_>, ins: &Instr) -> Result<(), String> {
        let Instr::PoolShutdown { pool , ..} = ins else {
            return Err("unreachable".to_string());
        };
                let callee = self.module.declare_func_in_func(self.pool_shutdown, &mut b.func);
                let p = self.val(b, *pool);
                b.ins().call(callee, &[p]);
        Ok(())
    }

    pub(super) fn lower_defer(&mut self, b: &mut FunctionBuilder<'_>, ins: &Instr) -> Result<(), String> {
        let Instr::Defer { body, .. } = ins else {
            return Err("unreachable".to_string());
        };
                let id = self
                    .defers
                    .iter()
                    .position(|x| x == body)
                    .ok_or("unknown defer body".to_string())?;
                let callee = self.module.declare_func_in_func(self.defer_push, &mut b.func);
                let arg = b.ins().iconst(types::I64, id as i64);
                b.ins().call(callee, &[arg]);
        Ok(())
    }

    pub(super) fn lower_run_defers(&mut self, b: &mut FunctionBuilder<'_>, ins: &Instr) -> Result<(), String> {
        let Instr::RunDefers { keep, .. } = ins else {
            return Err("unreachable".to_string());
        };
                self.emit_run_defers(b, *keep)?;
        Ok(())
    }
}
