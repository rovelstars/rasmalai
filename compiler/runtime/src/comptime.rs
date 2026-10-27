use diagnostics::{Code, Diagnostic, Span};
use frontend::macros::{ComptimeRunner, ComptimeValue, CALL_DEPTH_MAX, FUEL_STEPS};
use lir::instr::{CallTarget, Instr, Module};
use std::collections::BTreeSet;

use crate::machine::{ExecError, Machine};
use crate::value::Value;

const DENY_SUBSTRINGS: &[&str] = &[
    "clock",
    "random",
    "file",
    "env",
    "dns",
    "socket",
    "http",
    "net_",
    "thread",
    "mutex",
    "condvar",
    "barrier",
    "process",
    "exit",
    "stdin",
    "stdout",
    "stderr",
    "host_version",
    "sleep",
    "spawn",
    "kill",
    "signal",
    "prompt",
];

const DENY_EXACT: &[&str] = &["print"];

fn denied_name(name: &str) -> bool {
    if DENY_EXACT.contains(&name) {
        return true;
    }
    let lower = name.to_lowercase();
    DENY_SUBSTRINGS.iter().any(|d| lower.contains(d))
}

pub struct InterpComptime {
    lir: Module,
    comptime_lir_names: BTreeSet<String>,
}

fn lir_name_for(module: &Module, name: &str) -> Option<String> {
    if module.fn_index.contains_key(name) {
        return Some(name.to_string());
    }
    let suffix = format!(".{name}");
    let mut hit: Option<String> = None;
    for key in module.fn_index.keys() {
        if key.ends_with(suffix.as_str()) {
            if hit.is_some() {
                return None;
            }
            hit = Some(key.clone());
        }
    }
    hit
}

impl InterpComptime {
    pub fn build(module: &frontend::ast::Module) -> Result<Option<InterpComptime>, Diagnostic> {
        let mut names: Vec<String> = Vec::new();
        for decl in &module.decls {
            if let frontend::ast::Decl::Fn(f) = &decl.node {
                if f.attrs.iter().any(|a| a.name == "CompTime") {
                    names.push(f.name.clone());
                }
            }
        }
        if names.is_empty() {
            return Ok(None);
        }
        let snapshot = frontend::ast::Module {
            inner_attrs: Vec::new(),
            decls: module
                .decls
                .iter()
                .filter(|d| match &d.node {
                    frontend::ast::Decl::Fn(f) => f.attrs.iter().any(|a| a.name == "CompTime"),
                    _ => false,
                })
                .cloned()
                .collect(),
            docs: String::new(),
            warnings: Vec::new(),
        };
        let lir = lir::lower::lower(&snapshot).map_err(|e| {
            Diagnostic::new(
                Code::E122,
                format!("comptime snapshot failed to lower: {e}"),
            )
            .with_hint("comptime fns must be self-contained: builtin types and #[CompTime] callees only")
        })?;
        let mut comptime_lir_names = BTreeSet::new();
        for n in &names {
            if let Some(lir_name) = lir_name_for(&lir, n) {
                comptime_lir_names.insert(lir_name);
            }
        }
        Ok(Some(InterpComptime { lir, comptime_lir_names }))
    }

    fn sandbox_check(&self, target: &str, use_span: Span) -> Result<(), Diagnostic> {
        let Some(fi) = self.lir.fn_index.get(target).cloned() else {
            return Err(Diagnostic::new(
                Code::E122,
                format!("comptime fn `{target}` is missing from lowered code"),
            )
            .with_span(use_span));
        };
        let mut seen = BTreeSet::new();
        let mut stack = vec![fi];
        while let Some(f) = stack.pop() {
            if !seen.insert(f) {
                continue;
            }
            let func = &self.lir.functions[f];
            for block in &func.blocks {
                for ins in &block.instrs {
                    self.check_instr(ins, &mut stack, use_span)?;
                }
            }
        }
        Ok(())
    }

    fn check_instr(
        &self,
        ins: &Instr,
        stack: &mut Vec<usize>,
        use_span: Span,
    ) -> Result<(), Diagnostic> {
        let target = match ins {
            Instr::Call { target, .. } => target,
            _ => return Ok(()),
        };
        match target {
            CallTarget::Foreign { lib, symbol } => Err(Diagnostic::new(
                Code::E122,
                format!("comptime forbids foreign calls (`{lib}.{symbol}`)"),
            )
            .with_span(use_span)
            .with_hint("file bytes enter only through Phase-A EmbedBytes!")),
            CallTarget::Builtin(name) => {
                if denied_name(name) {
                    return Err(Diagnostic::new(
                        Code::E122,
                        format!("comptime forbids I/O builtin `{name}`"),
                    )
                    .with_span(use_span)
                    .with_hint("arguments, TypeInfo, and the pure prelude subset only"));
                }
                Ok(())
            }
            CallTarget::Dyn { method, .. } => {
                if denied_name(method) {
                    return Err(Diagnostic::new(
                        Code::E122,
                        format!("comptime forbids I/O method `.{method}`"),
                    )
                    .with_span(use_span));
                }
                Ok(())
            }
            CallTarget::Fn(i) => {
                let name = self.lir.functions[*i].name.clone();
                if !self.comptime_lir_names.contains(&name) {
                    return Err(Diagnostic::new(
                        Code::E122,
                        format!("comptime fn calls non-comptime `{name}`"),
                    )
                    .with_span(use_span)
                    .with_hint("only #[CompTime] fns run at build time"));
                }
                stack.push(*i);
                Ok(())
            }
            CallTarget::Method { class, method } => {
                let owner = self
                    .lir
                    .classes
                    .get(*class)
                    .map(|c| c.name.clone())
                    .unwrap_or_default();
                let mname = self
                    .lir
                    .classes
                    .get(*class)
                    .and_then(|c| {
                        c.methods.iter().find_map(|(k, v)| {
                            if v.id == *method { Some(k.clone()) } else { None }
                        })
                    })
                    .unwrap_or_default();
                if denied_name(&mname) {
                    return Err(Diagnostic::new(
                        Code::E122,
                        format!("comptime forbids I/O method `{owner}.{mname}`"),
                    )
                    .with_span(use_span));
                }
                stack.push(*method);
                Ok(())
            }
            CallTarget::Value(_) => Ok(()),
        }
    }

    fn encode(&self, machine: &mut Machine, value: &ComptimeValue) -> Value {
        match value {
            ComptimeValue::Int(i) => Value::Int(*i),
            ComptimeValue::Float(f) => Value::Float(*f, lir::instr::FloatKind::Strict),
            ComptimeValue::Bool(b) => Value::Bool(*b),
            ComptimeValue::Str(s) => Value::Str(s.clone()),
            ComptimeValue::Array(items) => {
                let elems: Vec<Value> =
                    items.iter().map(|i| self.encode(machine, i)).collect();
                machine.new_array(elems)
            }
        }
    }

    fn decode(&self, machine: &mut Machine, value: &Value) -> Result<ComptimeValue, String> {
        match value {
            Value::Int(i) => Ok(ComptimeValue::Int(*i)),
            Value::Float(f, _) => Ok(ComptimeValue::Float(*f)),
            Value::Bool(b) => Ok(ComptimeValue::Bool(*b)),
            Value::Str(s) => Ok(ComptimeValue::Str(s.clone())),
            Value::Null => Err("null is not a comptime value".to_string()),
            Value::Array { id } => {
                let elems: Option<Vec<Value>> =
                    machine.arrays.with_live(*id, |live| live.elems.clone());
                match elems {
                    Some(elems) => {
                        let mut out = Vec::with_capacity(elems.len());
                        for e in &elems {
                            out.push(self.decode(machine, e)?);
                        }
                        Ok(ComptimeValue::Array(out))
                    }
                    None => Err("dead array in comptime result".to_string()),
                }
            }
            other => Err(format!("unsupported comptime value `{other:?}`")),
        }
    }
}

impl ComptimeRunner for InterpComptime {
    fn call(
        &self,
        fn_name: &str,
        fn_span: Span,
        args: Vec<ComptimeValue>,
        use_span: Span,
    ) -> Result<ComptimeValue, Diagnostic> {
        let _ = fn_span;
        let Some(lir_name) = lir_name_for(&self.lir, fn_name) else {
            return Err(Diagnostic::new(
                Code::E122,
                format!("comptime fn `{fn_name}` is missing from lowered code"),
            )
            .with_span(use_span));
        };
        self.sandbox_check(&lir_name, use_span)?;
        frontend::macros::note_comptime_run();
        let mut machine = Machine::new(&self.lir);
        machine.fuel = Some(FUEL_STEPS);
        machine.max_depth = Some(CALL_DEPTH_MAX);
        let encoded: Vec<Value> = args.iter().map(|a| self.encode(&mut machine, a)).collect();
        let result = machine.call(&lir_name, encoded);
        match result {
            Ok(v) => self.decode(&mut machine, &v).map_err(|m| {
                Diagnostic::new(
                    Code::E122,
                    format!("comptime fn `{fn_name}` returned {m}"),
                )
                .with_span(use_span)
            }),
            Err(ExecError::Throw(v)) => Err(Diagnostic::new(
                Code::E122,
                format!("comptime fn `{fn_name}` threw `{v:?}`"),
            )
            .with_span(use_span)),
            Err(ExecError::Fatal(msg)) => {
                if msg.contains("fuel") {
                    return Err(Diagnostic::new(
                        Code::E117,
                        format!("comptime fn `{fn_name}` exceeded {FUEL_STEPS} steps"),
                    )
                    .with_span(use_span)
                    .with_hint("simplify the comptime fn"));
                }
                if msg.contains("depth") {
                    return Err(Diagnostic::new(
                        Code::E118,
                        format!("comptime fn `{fn_name}` exceeded call depth {CALL_DEPTH_MAX}"),
                    )
                    .with_span(use_span));
                }
                Err(Diagnostic::new(
                    Code::E122,
                    format!("comptime fn `{fn_name}` failed: {msg}"),
                )
                .with_span(use_span))
            }
        }
    }
}

pub fn base_dir_for(path: &std::path::Path) -> std::path::PathBuf {
    if path.is_file() {
        path.parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| std::path::PathBuf::from("."))
    } else {
        path.to_path_buf()
    }
}

pub fn dump_dir_for(base: &std::path::Path, enabled: bool) -> Option<std::path::PathBuf> {
    enabled.then(|| base.join(".rnx-cache").join("expansions"))
}
