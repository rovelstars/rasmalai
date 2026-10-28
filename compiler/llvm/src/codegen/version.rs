use super::*;

pub const VERSIONS: [(&str, Option<&str>, u64); 4] = [
    (".v0", None, 0),
    (".v2", Some("x86-64-v2"), 1),
    (".v3", Some("x86-64-v3"), 2),
    (".v4", Some("x86-64-v4"), 3),
];

pub fn triple_versions(triple: &str) -> bool {
    triple.starts_with("x86_64")
}

pub fn candidate(f: &Function) -> bool {
    let dom = lir::licm::compute_dominators(f);
    !lir::licm::find_natural_loops(f, &dom).is_empty()
}

pub fn version_name(base: &str, suffix: &str) -> String {
    let mut out = String::with_capacity(base.len() + suffix.len());
    out.push_str(base);
    out.push_str(suffix);
    out
}

fn err<E: std::fmt::Display>(msg: E) -> Diagnostic {
    Diagnostic::new(Code::E108, format!("tier stub: {msg}"))
}

pub(super) fn synthesize_stub(
    context: &Context,
    stub: FunctionValue,
    targets: &[FunctionValue],
    tier_fn: FunctionValue,
) -> Result<(), Diagnostic> {
    if targets.len() != VERSIONS.len() {
        return Err(err(format!("want {} versions, got {}", VERSIONS.len(), targets.len())));
    }
    let builder = context.create_builder();
    let entry = context.append_basic_block(stub, "tier_entry");
    let below_v4 = context.append_basic_block(stub, "tier_below_v4");
    let below_v3 = context.append_basic_block(stub, "tier_below_v3");
    let v4_bb = context.append_basic_block(stub, "tier_v4");
    let v3_bb = context.append_basic_block(stub, "tier_v3");
    let v2_bb = context.append_basic_block(stub, "tier_v2");
    let v0_bb = context.append_basic_block(stub, "tier_v0");
    builder.position_at_end(entry);
    let site = builder.build_call(tier_fn, &[], "tier").map_err(err)?;
    let tier = match site.try_as_basic_value() {
        ValueKind::Basic(v) => v.into_int_value(),
        ValueKind::Instruction(_) => return Err(err("tier probe returned void".to_string())),
    };
    let i64t = context.i64_type();
    let at_least = |n: u64| {
        builder
            .build_int_compare(
                inkwell::IntPredicate::UGE,
                tier,
                i64t.const_int(n, false),
                "tier_ge",
            )
            .map_err(err)
    };
    let go_v4 = at_least(VERSIONS[3].2)?;
    builder.build_conditional_branch(go_v4, v4_bb, below_v4).map_err(err)?;
    builder.position_at_end(below_v4);
    let go_v3 = at_least(VERSIONS[2].2)?;
    builder.build_conditional_branch(go_v3, v3_bb, below_v3).map_err(err)?;
    builder.position_at_end(below_v3);
    let go_v2 = at_least(VERSIONS[1].2)?;
    builder.build_conditional_branch(go_v2, v2_bb, v0_bb).map_err(err)?;
    let args: Vec<BasicMetadataValueEnum> =
        stub.get_params().iter().map(|p| (*p).into()).collect();
    let returns_value = stub.get_type().get_return_type().is_some();
    let order = [(v4_bb, targets[3]), (v3_bb, targets[2]), (v2_bb, targets[1]), (v0_bb, targets[0])];
    for (bb, ver) in order {
        builder.position_at_end(bb);
        let site = builder.build_call(ver, &args, "r").map_err(err)?;
        if returns_value {
            match site.try_as_basic_value() {
                ValueKind::Basic(v) => {
                    builder.build_return(Some(&v)).map_err(err)?;
                }
                ValueKind::Instruction(_) => {
                    return Err(err("version returned void".to_string()));
                }
            }
        } else {
            builder.build_return(None).map_err(err)?;
        }
    }
    Ok(())
}
