use super::*;
use cranelift_codegen::settings::Configurable;
use cranelift_object::{ObjectBuilder, ObjectModule};

fn object_isa() -> Result<cranelift_codegen::isa::OwnedTargetIsa, Diagnostic> {
    let mut fb = settings::builder();
    for (name, value) in SHARED_FLAGS {
        fb.set(name, value)
            .map_err(|e| Diagnostic::new(Code::E108, format!("object flags: {e}")))?;
    }
    fb.set("is_pic", "true")
        .map_err(|e| Diagnostic::new(Code::E108, format!("object flags: {e}")))?;
    let flags = settings::Flags::new(fb);
    let isa_builder = cranelift_native::builder()
        .map_err(|e| Diagnostic::new(Code::E108, format!("object isa: {e}")))?;
    isa_builder
        .finish(flags)
        .map_err(|e| Diagnostic::new(Code::E108, format!("object isa: {e}")))
}

fn check_object_build(lir: &LirModule, entry: &str) -> Result<(), Diagnostic> {
    for f in &lir.functions {
        if f.name == "main" && entry != "main" || f.name == "rnx_entry_main" || f.name == "rnx_init" {
            return Err(Diagnostic::new(
                Code::E108,
                format!("build reserves `{}`", f.name),
            ));
        }
    }
    let target = lir
        .functions
        .iter()
        .find(|f| f.name == entry)
        .ok_or_else(|| Diagnostic::new(Code::E108, format!("unknown entry `{entry}`")))?;
    if !target.params.is_empty() {
        return Err(Diagnostic::new(
            Code::E108,
            format!("build entry `{entry}` takes no arguments"),
        ));
    }
    if matches!(target.ret, LirType::Vec4f | LirType::Vec4i) {
        return Err(Diagnostic::new(
            Code::E108,
            format!("build entry `{entry}` must return Int, not a vector"),
        ));
    }
    if matches!(target.ret, LirType::Void) {
        return Err(Diagnostic::new(Code::E108, "build: entry returned void"));
    }
    Ok(())
}

fn decl<M: Module>(
    module: &mut M,
    name: &str,
    linkage: Linkage,
    sig: &Signature,
) -> Result<FuncId, Diagnostic> {
    module
        .declare_function(name, linkage, sig)
        .map_err(|e| Diagnostic::new(Code::E108, format!("declare {name}: {e}")))
}

fn void_sig() -> Signature {
    Signature::new(CallConv::SystemV)
}

fn i64_sig(nparams: usize) -> Signature {
    let mut sig = Signature::new(CallConv::SystemV);
    for _ in 0..nparams {
        sig.params.push(AbiParam::new(types::I64));
    }
    sig.returns.push(AbiParam::new(types::I64));
    sig
}

fn seal_all(b: &mut FunctionBuilder<'_>, blocks: &[cranelift_codegen::ir::Block]) {
    for bb in blocks {
        b.seal_block(*bb);
    }
}

fn define_entry_wrapper<M: Module>(
    module: &mut M,
    entry_id: FuncId,
    entry_sig: &Signature,
    ctx: &mut FunctionBuilderContext,
) -> Result<FuncId, Diagnostic> {
    let mut sig = Signature::new(CallConv::SystemV);
    sig.returns.clone_from(&entry_sig.returns);
    let id = decl(module, "rnx_entry_main", Linkage::Export, &sig)?;
    let mut codegen_ctx = module.make_context();
    codegen_ctx.func =
        ClFunction::with_name_signature(UserFuncName::user(0, id.as_u32()), sig);
    let mut b = FunctionBuilder::new(&mut codegen_ctx.func, ctx);
    let blk = b.create_block();
    b.switch_to_block(blk);
    let callee = module.declare_func_in_func(entry_id, &mut b.func);
    let inst = b.ins().call(callee, &[]);
    let out = b.inst_results(inst).to_vec();
    b.ins().return_(&out);
    b.seal_block(blk);
    let config = module.isa().frontend_config();
    b.finalize(config);
    module
        .define_function(id, &mut codegen_ctx)
        .map_err(|e| Diagnostic::new(Code::E108, format!("define rnx_entry_main: {e}")))?;
    Ok(id)
}

fn define_closure_init<M: Module>(
    module: &mut M,
    lir: &LirModule,
    ids: &BTreeMap<String, FuncId>,
    reg_id: FuncId,
    ctx: &mut FunctionBuilderContext,
) -> Result<FuncId, Diagnostic> {
    let id = decl(module, "__rnx_closure_init", Linkage::Export, &void_sig())?;
    let mut codegen_ctx = module.make_context();
    codegen_ctx.func = ClFunction::with_name_signature(
        UserFuncName::user(0, id.as_u32()),
        void_sig(),
    );
    let mut b = FunctionBuilder::new(&mut codegen_ctx.func, ctx);
    let blk = b.create_block();
    b.switch_to_block(blk);
    for (i, f) in lir.functions.iter().enumerate() {
        if !f.is_closure {
            continue;
        }
        let fid = ids.get(&f.name).copied().ok_or_else(|| {
            Diagnostic::new(Code::E108, format!("build: missing closure `{}`", f.name))
        })?;
        let target = module.declare_func_in_func(fid, &mut b.func);
        let addr = b.ins().func_addr(types::I64, target);
        let tag = b.ins().iconst(
            types::I64,
            runtime::native::rnx_closure_tag(i as u64) as i64,
        );
        let reg = module.declare_func_in_func(reg_id, &mut b.func);
        b.ins().call(reg, &[tag, addr]);
    }
    b.ins().return_(&[]);
    b.seal_block(blk);
    let config = module.isa().frontend_config();
    b.finalize(config);
    module
        .define_function(id, &mut codegen_ctx)
        .map_err(|e| Diagnostic::new(Code::E108, format!("define __rnx_closure_init: {e}")))?;
    Ok(id)
}

fn define_main<M: Module>(
    module: &mut M,
    init_id: FuncId,
    clo_init_id: FuncId,
    strict_id: FuncId,
    wrap_id: FuncId,
    fatal_id: FuncId,
    take_id: FuncId,
    report_id: FuncId,
    ctx: &mut FunctionBuilderContext,
) -> Result<FuncId, Diagnostic> {
    let mut sig = Signature::new(CallConv::SystemV);
    sig.params.push(AbiParam::new(types::I32));
    sig.params.push(AbiParam::new(types::I64));
    sig.returns.push(AbiParam::new(types::I32));
    let id = decl(module, "main", Linkage::Export, &sig)?;
    let mut codegen_ctx = module.make_context();
    codegen_ctx.func =
        ClFunction::with_name_signature(UserFuncName::user(0, id.as_u32()), sig);
    let mut b = FunctionBuilder::new(&mut codegen_ctx.func, ctx);
    let entry = b.create_block();
    let fatal_bb = b.create_block();
    let nofatal_bb = b.create_block();
    let err_bb = b.create_block();
    let ok_bb = b.create_block();
    b.switch_to_block(entry);
    b.append_block_params_for_function_params(entry);
    let params = b.block_params(entry).to_vec();
    let (argc, argv) = (params[0], params[1]);
    let init = module.declare_func_in_func(init_id, &mut b.func);
    b.ins().call(init, &[argc, argv]);
    let clo_init = module.declare_func_in_func(clo_init_id, &mut b.func);
    b.ins().call(clo_init, &[]);
    let strict = module.declare_func_in_func(strict_id, &mut b.func);
    let one = b.ins().iconst(types::I8, 1);
    b.ins().call(strict, &[one]);
    let wrap = module.declare_func_in_func(wrap_id, &mut b.func);
    let inst = b.ins().call(wrap, &[]);
    let r = b.inst_results(inst)[0];
    let fatal = module.declare_func_in_func(fatal_id, &mut b.func);
    let inst = b.ins().call(fatal, &[]);
    let fatal_code = b.inst_results(inst)[0];
    let zero32 = b.ins().iconst(types::I32, 0);
    let has_fatal = b.ins().icmp(
        cranelift_codegen::ir::condcodes::IntCC::NotEqual,
        fatal_code,
        zero32,
    );
    b.ins().brif(has_fatal, fatal_bb, &[], nofatal_bb, &[]);
    b.switch_to_block(fatal_bb);
    let one32 = b.ins().iconst(types::I32, 1);
    b.ins().return_(&[one32]);
    b.switch_to_block(nofatal_bb);
    let take = module.declare_func_in_func(take_id, &mut b.func);
    let inst = b.ins().call(take, &[]);
    let pending = b.inst_results(inst)[0];
    let zero64 = b.ins().iconst(types::I64, 0);
    let has_err = b.ins().icmp(
        cranelift_codegen::ir::condcodes::IntCC::NotEqual,
        pending,
        zero64,
    );
    b.ins().brif(has_err, err_bb, &[], ok_bb, &[]);
    b.switch_to_block(err_bb);
    let report = module.declare_func_in_func(report_id, &mut b.func);
    let inst = b.ins().call(report, &[pending]);
    let code = b.inst_results(inst)[0];
    b.ins().return_(&[code]);
    b.switch_to_block(ok_bb);
    let exit = b.ins().ireduce(types::I32, r);
    b.ins().return_(&[exit]);
    seal_all(&mut b, &[entry, fatal_bb, nofatal_bb, err_bb, ok_bb]);
    let config = module.isa().frontend_config();
    b.finalize(config);
    module
        .define_function(id, &mut codegen_ctx)
        .map_err(|e| Diagnostic::new(Code::E108, format!("define main: {e}")))?;
    Ok(id)
}

fn define_glue<M: Module>(
    module: &mut M,
    lir: &LirModule,
    entry: &str,
    ids: &BTreeMap<String, FuncId>,
    sigs: &BTreeMap<FuncId, Signature>,
    rt: &RtIds,
    ctx: &mut FunctionBuilderContext,
) -> Result<(), Diagnostic> {
    let entry_id = ids.get(entry).copied().ok_or_else(|| {
        Diagnostic::new(Code::E108, format!("unknown entry `{entry}`"))
    })?;
    let entry_sig = sigs.get(&entry_id).cloned().ok_or_else(|| {
        Diagnostic::new(Code::E108, format!("build: no signature for `{entry}`"))
    })?;
    let mut init_sig = Signature::new(CallConv::SystemV);
    init_sig.params.push(AbiParam::new(types::I32));
    init_sig.params.push(AbiParam::new(types::I64));
    let init_id = decl(module, "rnx_init", Linkage::Import, &init_sig)?;
    let reg_id = decl(module, "rnx_closure_register", Linkage::Import, &i64_sig(2))?;
    let mut strict_sig = Signature::new(CallConv::SystemV);
    strict_sig.params.push(AbiParam::new(types::I8));
    let strict_id = decl(module, "rnx_set_assert_strict", Linkage::Import, &strict_sig)?;
    let mut report_sig = Signature::new(CallConv::SystemV);
    report_sig.params.push(AbiParam::new(types::I64));
    report_sig.returns.push(AbiParam::new(types::I32));
    let report_id = decl(module, "rnx_report_uncaught", Linkage::Import, &report_sig)?;
    let mut fatal_sig = Signature::new(CallConv::SystemV);
    fatal_sig.returns.push(AbiParam::new(types::I32));
    let fatal_id = decl(module, "rnx_report_fatal", Linkage::Import, &fatal_sig)?;
    let wrap_id = define_entry_wrapper(module, entry_id, &entry_sig, ctx)?;
    let clo_init_id = define_closure_init(module, lir, ids, reg_id, ctx)?;
    define_main(module, init_id, clo_init_id, strict_id, wrap_id, fatal_id, rt.error_take, report_id, ctx)?;
    Ok(())
}

pub fn emit_object(lir: &LirModule, name: &str, entry: &str) -> Result<Vec<u8>, Diagnostic> {
    runtime::native::rnx_set_closure_epoch(0);
    check_object_build(lir, entry)?;
    let isa = object_isa()?;
    let builder = ObjectBuilder::new(isa, name, default_libcall_names())
        .map_err(|e| Diagnostic::new(Code::E108, format!("object init: {e}")))?;
    let mut module = ObjectModule::new(builder);
    let (ids, sigs, rt, bytes_fns, statics, foreign_ids) =
        Jit::declare_imports(&mut module, lir, Linkage::Export, Some(entry))?;
    let mut ctx = FunctionBuilderContext::new();
    let mut timings = CodegenTimings::default();
    timings.fn_count = lir.functions.len();
    Jit::define_functions(
        &mut module,
        lir,
        &ids,
        &sigs,
        &foreign_ids,
        &rt,
        &bytes_fns,
        &statics,
        &mut ctx,
        false,
        0,
        &mut timings,
    )?;
    define_glue(&mut module, lir, entry, &ids, &sigs, &rt, &mut ctx)?;
    module
        .finish()
        .emit()
        .map_err(|e| Diagnostic::new(Code::E108, format!("object emit: {e}")))
}
