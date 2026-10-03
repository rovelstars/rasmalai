use crate::ast as A;
use diagnostics::{Code, Diagnostic, Span};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
pub enum ConfigValue {
    String(String),
    Int(i64),
    Bool(bool),
    Object(Vec<(String, ConfigValue)>),
    Array(Vec<ConfigValue>),
}

impl ConfigValue {
    pub fn get(&self, key: &str) -> Option<&ConfigValue> {
        match self {
            ConfigValue::Object(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn truthy(&self) -> Option<bool> {
        match self {
            ConfigValue::Bool(b) => Some(*b),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ConfigTarget {
    pub os: String,
    pub arch: String,
    pub env: String,
}

impl ConfigTarget {
    pub fn host() -> ConfigTarget {
        let os = std::env::consts::OS.to_string();
        let arch = std::env::consts::ARCH.to_string();
        let env = std::env::var("RNX_TARGET_ENV").unwrap_or_else(|_| {
            if os == "windows" {
                "msvc".to_string()
            } else if os == "linux" {
                "gnu".to_string()
            } else {
                String::new()
            }
        });
        ConfigTarget { os, arch, env }
    }
}

pub struct ConfigContext {
    pub target: ConfigTarget,
    pub constants: HashMap<String, ConfigValue>,
}

impl ConfigContext {
    pub fn new(target: ConfigTarget) -> ConfigContext {
        ConfigContext { target, constants: HashMap::new() }
    }
}

fn err(span: Span, what: &str) -> Diagnostic {
    Diagnostic::new(Code::E108, format!("{what} is not permitted in configuration files"))
        .with_span(span)
}

pub fn eval_module(module: &A::Module, target: ConfigTarget) -> Result<ConfigValue, Diagnostic> {
    let mut ctx = ConfigContext::new(target);
    let mut default: Option<ConfigValue> = None;
    for decl in &module.decls {
        match &decl.node {
            A::Decl::Const { name, value, .. } => {
                if ctx.constants.contains_key(name) {
                    return Err(Diagnostic::new(
                        Code::E108,
                        format!("duplicate config constant `{name}`"),
                    )
                    .with_span(decl.span));
                }
                let v = eval_expr(&mut ctx, value)?;
                ctx.constants.insert(name.clone(), v);
            }
            A::Decl::ExportDefault(d) => {
                if default.is_some() {
                    return Err(Diagnostic::new(
                        Code::E108,
                        "duplicate `export default` in configuration file",
                    )
                    .with_span(decl.span));
                }
                default = Some(eval_expr(&mut ctx, &d.expr)?);
            }
            A::Decl::Fn(_) => return Err(err(decl.span, "function definitions")),
            A::Decl::Class { .. }
            | A::Decl::Struct { .. }
            | A::Decl::Trait { .. }
            | A::Decl::Interface { .. }
            | A::Decl::Extension { .. }
            | A::Decl::Enum { .. }
            | A::Decl::Record { .. } => {
                return Err(err(decl.span, "type definitions"));
            }
            A::Decl::Import(_) => return Err(err(decl.span, "`import` statements")),
            A::Decl::ExportFrom(_) => return Err(err(decl.span, "re-exports")),
            A::Decl::ExportList(_) => return Err(err(decl.span, "export lists")),
            A::Decl::Stmt(s) => return Err(stmt_err(s)),
        }
    }
    default.ok_or_else(|| {
        Diagnostic::new(Code::E108, "configuration file needs `export default { ... }`")
    })
}

fn stmt_err(s: &A::Spanned<A::Stmt>) -> Diagnostic {
    let what = match &s.node {
        A::Stmt::Var { mutable, .. } if *mutable => "`let` bindings",
        A::Stmt::Var { .. } => "variable declarations (use `const`)",
        A::Stmt::While { .. } | A::Stmt::DoWhile { .. } => "loops",
        A::Stmt::For { .. } => "loops",
        _ => "statements",
    };
    err(s.span, what)
}

pub fn eval_expr(ctx: &mut ConfigContext, e: &A::Spanned<A::Expr>) -> Result<ConfigValue, Diagnostic> {
    let span = e.span;
    match &e.node {
        A::Expr::Bool(b) => Ok(ConfigValue::Bool(*b)),
        A::Expr::Int(n) => Ok(ConfigValue::Int(*n)),
        A::Expr::Interp(parts) => {
            if parts.is_empty() {
                return Ok(ConfigValue::String(String::new()));
            }
            if parts.len() == 1 {
                if let A::InterpPart::Text(t) = &parts[0] {
                    return Ok(ConfigValue::String(t.clone()));
                }
            }
            Err(err(span, "string interpolation"))
        }
        A::Expr::Ident(name) => ctx
            .constants
            .get(name)
            .cloned()
            .ok_or_else(|| Diagnostic::new(Code::E108, format!("unknown config name `{name}`")).with_span(span)),
        A::Expr::Member { base, field } => {
            if let A::Expr::Ident(root) = &base.node
                && root == "target"
            {
                return match field.as_str() {
                    "os" => Ok(ConfigValue::String(ctx.target.os.clone())),
                    "arch" => Ok(ConfigValue::String(ctx.target.arch.clone())),
                    "env" => Ok(ConfigValue::String(ctx.target.env.clone())),
                    _ => Err(Diagnostic::new(
                        Code::E108,
                        format!("unknown `target` property `{field}` (expected `os`, `arch`, or `env`)"),
                    )
                    .with_span(span)),
                };
            }
            let base_v = eval_expr(ctx, base)?;
            match &base_v {
                ConfigValue::Object(fields) => fields
                    .iter()
                    .find(|(k, _)| k == field)
                    .map(|(_, v)| v.clone())
                    .ok_or_else(|| {
                        Diagnostic::new(Code::E108, format!("unknown field `{field}`")).with_span(span)
                    }),
                _ => Err(err(span, "member access on non-objects")),
            }
        }
        A::Expr::Array(elems) => {
            let mut out = Vec::with_capacity(elems.len());
            for el in elems {
                let v = eval_expr(ctx, &el.expr)?;
                if el.spread {
                    match v {
                        ConfigValue::Array(items) => out.extend(items),
                        _ => return Err(err(el.expr.span, "spreading non-arrays")),
                    }
                } else {
                    out.push(v);
                }
            }
            Ok(ConfigValue::Array(out))
        }
        A::Expr::Record(fields) => {
            let mut out = Vec::new();
            for e in fields {
                match e {
                    A::RecordEntry::Field(k, v) => {
                        let val = eval_expr(ctx, v)?;
                        if let Some(slot) = out.iter_mut().find(|(ek, _)| ek == k) {
                            slot.1 = val;
                        } else {
                            out.push((k.clone(), val));
                        }
                    }
                    A::RecordEntry::Spread(v) => {
                        match eval_expr(ctx, v)? {
                            ConfigValue::Object(items) => {
                                for (k, val) in items {
                                    if let Some(slot) = out.iter_mut().find(|(ek, _)| ek == &k) {
                                        slot.1 = val;
                                    } else {
                                        out.push((k, val));
                                    }
                                }
                            }
                            _ => return Err(err(v.span, "spreading non-objects")),
                        }
                    }
                }
            }
            Ok(ConfigValue::Object(out))
        }
        A::Expr::MapLiteral(entries) => {
            let mut out = Vec::new();
            for e in entries {
                match e {
                    A::MapEntry::Field(k, v) => {
                        let val = eval_expr(ctx, v)?;
                        if let Some(slot) = out.iter_mut().find(|(ek, _)| ek == k) {
                            slot.1 = val;
                        } else {
                            out.push((k.clone(), val));
                        }
                    }
                    A::MapEntry::Spread(v) => match eval_expr(ctx, v)? {
                        ConfigValue::Object(items) => {
                            for (k, val) in items {
                                if let Some(slot) = out.iter_mut().find(|(ek, _)| ek == &k) {
                                    slot.1 = val;
                                } else {
                                    out.push((k, val));
                                }
                            }
                        }
                        _ => return Err(err(v.span, "spreading non-objects")),
                    },
                }
            }
            Ok(ConfigValue::Object(out))
        }
        A::Expr::Binary { op, lhs, rhs } => {
            let l = eval_expr(ctx, lhs)?;
            let r = eval_expr(ctx, rhs)?;
            eval_binary(*op, &l, &r, span)
        }
        A::Expr::Unary { op, rhs } => match op {
            A::UnOp::Not => match eval_expr(ctx, rhs)? {
                ConfigValue::Bool(b) => Ok(ConfigValue::Bool(!b)),
                _ => Err(err(span, "non-boolean `!` operands")),
            },
            A::UnOp::Neg => match eval_expr(ctx, rhs)? {
                ConfigValue::Int(n) => Ok(ConfigValue::Int(n.wrapping_neg())),
                _ => Err(err(span, "non-integer negation")),
            },
            _ => Err(err(span, "pointer operators")),
        },
        A::Expr::Ternary { cond, then, otherwise } => {
            match eval_expr(ctx, cond)? {
                ConfigValue::Bool(true) => eval_expr(ctx, then),
                ConfigValue::Bool(false) => eval_expr(ctx, otherwise),
                _ => Err(err(cond.span, "non-boolean conditions")),
            }
        }
        A::Expr::Switch { scrutinee, cases, default } => {
            let s = eval_expr(ctx, scrutinee)?;
            for case in cases {
                if case.guard.is_some() {
                    return Err(err(span, "match guards"));
                }
                let hit = match &case.pattern {
                    A::Pattern::Wildcard => true,
                    A::Pattern::Literal(pat) => eval_expr(ctx, pat)? == s,
                    _ => return Err(err(span, "non-literal match patterns")),
                };
                if hit {
                    match &case.body {
                        A::SwitchExprBody::Expr(e) => return eval_expr(ctx, e),
                        A::SwitchExprBody::Block(_) => return Err(err(span, "match arm blocks")),
                    }
                }
            }
            match default {
                Some(A::SwitchExprBody::Expr(e)) => eval_expr(ctx, e),
                Some(A::SwitchExprBody::Block(_)) => Err(err(span, "match arm blocks")),
                None => Err(Diagnostic::new(Code::E108, "no match arm matched the target").with_span(span)),
            }
        }
        A::Expr::Call { .. } => Err(err(span, "function calls")),
        A::Expr::Closure { .. } => Err(err(span, "function definitions")),
        A::Expr::Index { .. } => Err(err(span, "indexing")),
        A::Expr::Cast { .. } => Err(err(span, "casts")),
        A::Expr::Float(_) => Err(err(span, "float literals")),
        _ => Err(err(span, "this expression")),
    }
}

fn eval_binary(op: A::BinOp, l: &ConfigValue, r: &ConfigValue, span: Span) -> Result<ConfigValue, Diagnostic> {
    match op {
        A::BinOp::Eq => Ok(ConfigValue::Bool(l == r)),
        A::BinOp::NotEq => Ok(ConfigValue::Bool(l != r)),
        A::BinOp::And => match (l.truthy(), r.truthy()) {
            (Some(a), Some(b)) => Ok(ConfigValue::Bool(a && b)),
            _ => Err(err(span, "non-boolean `&&` operands")),
        },
        A::BinOp::Or => match (l.truthy(), r.truthy()) {
            (Some(a), Some(b)) => Ok(ConfigValue::Bool(a || b)),
            _ => Err(err(span, "non-boolean `||` operands")),
        },
        _ => Err(err(span, "arithmetic")),
    }
}
