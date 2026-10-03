use crate::ast::*;
use diagnostics::{Code, Diagnostic, Span};
use std::collections::BTreeMap;
use super::{ScopeCheck, VarInfo, SCOPE_BUILTINS, array_elem, builtin_method_sig, canon_ty, is_higher_order, is_removed_poll_base, promise_inner, removed_replacement, starts_uppercase, ty_name};
impl<'a> ScopeCheck<'a> {
    fn lookup(&self, name: &str) -> Option<Option<String>> {
        for s in self.scopes.iter().rev() {
            if let Some(info) = s.get(name) {
                return Some(info.ty.clone());
            }
        }
        if self.globals.contains(name) || SCOPE_BUILTINS.contains(&name) {
            return Some(None);
        }
        if crate::prelude::provides(name) {
            return Some(None);
        }
        if name.starts_with("__rnx_") {
            return Some(None);
        }
        None
    }

    fn lookup_elem(&self, name: &str) -> Option<String> {
        for s in self.scopes.iter().rev() {
            if let Some(info) = s.get(name) {
                return info.elem.clone();
            }
        }
        None
    }

    fn use_name(&mut self, name: &str) -> bool {
        for s in self.scopes.iter().rev() {
            if s.contains_key(name) {
                return true;
            }
        }
        if self.globals.contains(name) || SCOPE_BUILTINS.contains(&name) {
            return true;
        }
        if crate::prelude::provides(name) {
            self.used_prelude.insert(crate::prelude::short_name(name).to_string());
            return true;
        }
        if name.starts_with("__rnx_") {
            return true;
        }
        false
    }

    fn shadowed(&self, name: &str) -> bool {
        self.scopes.iter().rev().any(|s| s.contains_key(name))
            || self.globals.contains(name)
            || SCOPE_BUILTINS.contains(&name)
    }

    fn shadowed_local(&self, name: &str) -> bool {
        self.scopes.iter().rev().any(|s| s.contains_key(name))
    }

    fn deny_removed_value(&mut self, name: &str, span: Span) -> bool {
        let short = crate::prelude::short_name(name);
        let (what, hint) = match short {
            "Option" => (
                "`Option<T>` has been removed",
                "use `T?` and `null` instead",
            ),
            "Some" => (
                "`Option.Some` has been removed",
                "use the value directly with a nullable type (`T?`)",
            ),
            "None" => ("`Option.None` has been removed", "use `null` instead"),
            _ => return false,
        };
        if self.shadowed(short) || (short != name && self.shadowed(name)) {
            return false;
        }
        self.diags.push(
            Diagnostic::new(Code::E303, what)
                .with_span(self.span(span))
                .with_hint(hint),
        );
        true
    }

    pub(super) fn deny_removed_ty(&mut self, t: &Type, span: Span) {
        if let Some(head) = t.path.first() {
            if head == "Option" && !self.shadowed(head) {
                self.diags.push(
                    Diagnostic::new(Code::E303, "`Option<T>` has been removed")
                        .with_span(self.span(span))
                        .with_hint("use `T?` and `null` instead"),
                );
            } else if let Some(replacement) = removed_replacement(head) {
                if !self.shadowed(head) {
                    self.diags.push(
                        Diagnostic::new(Code::E303, format!("unresolved identifier `{head}`"))
                            .with_span(self.span(span))
                            .with_hint(format!("`{head}` was removed; use `{replacement}`")),
                    );
                }
            }
        }
        for a in &t.args {
            self.deny_removed_ty(a, span);
        }
        for x in &t.tuple {
            self.deny_removed_ty(x, span);
        }
        if let Some(sig) = t.fn_sig.as_ref() {
            for p in &sig.params {
                self.deny_removed_ty(p, span);
            }
            if let Some(r) = sig.ret.as_ref() {
                self.deny_removed_ty(r, span);
            }
        }
    }

    fn note_ty(&mut self, t: &Type) {
        if let Some(head) = t.path.first() {
            self.use_name(head);
        }
        for a in &t.args {
            self.note_ty(a);
        }
        for x in &t.tuple {
            self.note_ty(x);
        }
        if let Some(sig) = t.fn_sig.as_ref() {
            for p in &sig.params {
                self.note_ty(p);
            }
            if let Some(r) = sig.ret.as_ref() {
                self.note_ty(r);
            }
        }
    }

    fn declare(&mut self, name: String, ty: Option<String>, mutable: bool) {
        if let Some(s) = self.scopes.last_mut() {
            s.insert(name, VarInfo { ty, elem: None, mutable });
        }
    }

    fn declare_elem(&mut self, name: &str, elem: Option<String>) {
        if elem.is_none() {
            return;
        }
        for s in self.scopes.iter_mut().rev() {
            if let Some(info) = s.get_mut(name) {
                info.elem = elem;
                return;
            }
        }
    }

    fn binding_mutable(&self, name: &str) -> Option<bool> {
        for s in self.scopes.iter().rev() {
            if let Some(info) = s.get(name) {
                return Some(info.mutable);
            }
        }
        None
    }

    fn undefine(&mut self, name: &str) {
        for s in self.scopes.iter_mut().rev() {
            if let Some(info) = s.get_mut(name) {
                info.ty = None;
                info.elem = None;
                return;
            }
        }
    }

    pub(super) fn span(&self, s: Span) -> Span {
        if s.start == 0 && s.end == 0 {
            self.stmt_span
        } else {
            s
        }
    }

    fn err_undefined(&mut self, name: &str, span: Span) {
        let span = self.span(span);
        let hint = if self.let_names.contains(name) {
            "a `let` in another function (or at the top level) stays local to it; share the value with `const` or pass it as a parameter"
        } else {
            "declare the variable with `let` or check the spelling"
        };
        self.diags.push(
            Diagnostic::new(Code::E303, format!("undefined variable `{name}`"))
                .with_span(span)
                .with_hint(hint),
        );
    }

    fn check_builtin_method(&mut self, base: &str, method: &str, args: &[CallArg], span: Span) {
        if is_higher_order(base, method) {
            if args.len() != 1 {
                self.diags.push(
                    Diagnostic::new(Code::E108, format!("`{base}.{method}` takes 1 arg"))
                        .with_span(self.span(span)),
                );
                return;
            }
            self.check_closure_arg(base, method, args, span, base == "Array" && method == "filter");
            return;
        }
        let Some((arity, params, _)) = builtin_method_sig(base, method) else {
            if let Some((mandatory, total)) = self.extensions.get(&(base.to_string(), method.to_string())) {
                if args.len() < *mandatory || args.len() > *total {
                    self.diags.push(
                        Diagnostic::new(Code::E108, format!("`{base}.{method}` takes {mandatory}..{total} args"))
                            .with_span(self.span(span)),
                    );
                }
                return;
            }
            self.diags.push(
                Diagnostic::new(Code::E108, format!("unknown method `{method}` on `{base}`"))
                    .with_span(self.span(span)),
            );
            return;
        };
        if base == "String" && method == "indexOf" {
            if args.len() != 1 && args.len() != 2 {
                self.diags.push(
                    Diagnostic::new(Code::E108, "`String.indexOf` takes 1..2 args")
                        .with_span(self.span(span)),
                );
                return;
            }
            let wants = if args.len() == 2 { vec!["String", "Int"] } else { vec!["String"] };
            for (arg, want) in args.iter().zip(wants.iter()) {
                if let Some(got) = self.infer(&arg.value.node) {
                    if canon_ty(&got) != canon_ty(want) {
                        self.diags.push(
                            Diagnostic::new(
                                Code::E108,
                                format!("`String.indexOf` needs a `{want}` argument, got `{got}`"),
                            )
                            .with_span(self.span(arg.value.span)),
                        );
                    }
                }
            }
            return;
        }
        if args.len() != arity {
            self.diags.push(
                Diagnostic::new(Code::E108, format!("`{base}.{method}` takes {arity} args"))
                    .with_span(self.span(span)),
            );
            return;
        }
        for (arg, want) in args.iter().zip(params.iter()) {
            if let Some(got) = self.infer(&arg.value.node) {
                if canon_ty(&got) != canon_ty(want) {
                    self.diags.push(
                        Diagnostic::new(
                            Code::E108,
                            format!("`{base}.{method}` needs a `{want}` argument, got `{got}`"),
                        )
                        .with_span(self.span(arg.value.span)),
                    );
                }
            }
        }
    }

    fn check_class_method(&mut self, base_ty: &str, method: &str, args: &[CallArg], span: Span) {
        let base = crate::prelude::short_name(base_ty).to_string();
        if base == "Any" {
            return;
        }
        if let Some((mandatory, total)) = self.methods.get(&(base.clone(), method.to_string())).cloned()
        {
            if args.len() < mandatory || args.len() > total {
                let what = if mandatory == total {
                    format!("`{base}.{method}` takes {mandatory} args")
                } else {
                    format!("`{base}.{method}` takes {mandatory}..{total} args")
                };
                self.diags.push(
                    Diagnostic::new(Code::E108, what).with_span(self.span(span)),
                );
            }
            return;
        }
        if let Some((mandatory, total)) =
            self.extensions.get(&(base.clone(), method.to_string())).cloned()
        {
            if args.len() < mandatory || args.len() > total {
                self.diags.push(
                    Diagnostic::new(Code::E108, format!("`{base}.{method}` takes {mandatory}..{total} args"))
                        .with_span(self.span(span)),
                );
            }
            return;
        }
        if self.classes.contains(&base)
            || self.methods.keys().any(|(c, _)| c == &base)
            || self.fields.iter().any(|(c, _)| c == &base)
        {
            if self.fields.contains(&(base.clone(), method.to_string())) {
                return;
            }
            self.diags.push(
                Diagnostic::new(Code::E108, format!("unknown method `{method}` on `{base}`"))
                    .with_span(self.span(span)),
            );
        }
    }

    fn check_fn_arity(&mut self, name: &str, args: &[CallArg], span: Span) {
        let Some((mandatory, total)) = self.fn_arity.get(name).cloned() else {
            return;
        };
        if args.len() >= mandatory && args.len() <= total {
            return;
        }
        let what = if mandatory == total {
            format!("`{name}` takes {mandatory} args")
        } else {
            format!("`{name}` takes {mandatory}..{total} args")
        };
        self.diags.push(
            Diagnostic::new(Code::E108, what).with_span(self.span(span)),
        );
    }

    fn check_ctor_arity(&mut self, target: &str, args: &[CallArg], span: Span) {
        let short = crate::prelude::short_name(target);
        let Some((mandatory, total)) = self.ctors.get(short).cloned() else {
            return;
        };
        if args.len() >= mandatory && args.len() <= total {
            return;
        }
        let what = if mandatory == total {
            format!("`{short}.init` takes {mandatory} args")
        } else {
            format!("`{short}.init` takes {mandatory}..{total} args")
        };
        self.diags.push(
            Diagnostic::new(Code::E108, what).with_span(self.span(span)),
        );
    }

    fn arg_hints(params: &[(String, Option<String>)], args: &[CallArg]) -> Vec<Option<String>> {
        let mut filled = vec![false; params.len()];
        let mut out = Vec::with_capacity(args.len());
        for a in args {
            if let Some(nm) = &a.name {
                match params.iter().position(|(pn, _)| pn == nm) {
                    Some(i) => {
                        filled[i] = true;
                        out.push(params[i].1.clone());
                    }
                    None => out.push(None),
                }
            } else {
                match filled.iter().position(|f| !f) {
                    Some(i) => {
                        filled[i] = true;
                        out.push(params[i].1.clone());
                    }
                    None => out.push(None),
                }
            }
        }
        out
    }

    fn walk_args_with(&mut self, params: Option<Vec<(String, Option<String>)>>, args: &[CallArg]) {
        match params {
            Some(p) => {
                let hints = Self::arg_hints(&p, args);
                for (a, h) in args.iter().zip(hints) {
                    let outer = self.enum_hint.clone();
                    self.enum_hint = h;
                    self.walk_expr(&a.value);
                    self.enum_hint = outer;
                }
            }
            None => {
                for a in args {
                    self.walk_expr(&a.value);
                }
            }
        }
    }

    fn check_implicit_member(&mut self, path: &[String], span: Span) {
        let Some(vname) = path.last() else {
            return;
        };
        let Some(exp) = self.enum_hint.clone() else {
            return;
        };
        let exp_short = crate::prelude::short_name(&exp).to_string();
        let mut members: Option<Vec<String>> = None;
        for decl in &self.module.decls {
            if let Decl::Enum { name, members: ms, .. } = &decl.node {
                if crate::prelude::short_name(name) == exp_short {
                    members = Some(ms.iter().map(|m| m.name.clone()).collect());
                    break;
                }
            }
        }
        let Some(members) = members else {
            return;
        };
        let vshort = crate::prelude::short_name(vname).to_string();
        if members
            .iter()
            .any(|m| crate::prelude::short_name(m) == vshort)
        {
            return;
        }
        self.diags.push(
            Diagnostic::new(Code::E108, format!("unknown variant `.{vname}` for enum `{exp_short}`"))
                .with_span(self.span(span))
                .with_hint(format!("`{exp_short}` has no variant `.{vname}`")),
        );
    }

    fn check_float_static(&mut self, method: &str, args: &[CallArg], span: Span) {
        let arity = match method {
            "isNaN" => 1,
            "nan" => 0,
            "fma" => 3,
            "fromBits" => 1,
            _ => {
                self.diags.push(
                    Diagnostic::new(Code::E108, format!("unknown static `Float.{method}`"))
                        .with_span(self.span(span)),
                );
                return;
            }
        };
        if args.len() != arity {
            self.diags.push(
                Diagnostic::new(Code::E108, format!("`Float.{method}` takes {arity} args"))
                    .with_span(self.span(span)),
            );
        }
    }

    fn check_closure_arg(&mut self, base: &str, method: &str, args: &[CallArg], span: Span, expect_bool: bool) {
        let _ = span;
        match &args[0].value.node {
            Expr::Closure { params, ret, body, .. } => {
                if params.len() != 1 {
                    self.diags.push(
                        Diagnostic::new(
                            Code::E108,
                            format!("`{base}.{method}` needs a one-parameter closure"),
                        )
                        .with_span(self.span(args[0].value.span)),
                    );
                    return;
                }
                if expect_bool {
                    let ret_ty = ret.as_ref().and_then(ty_name).or_else(|| match body {
                        FnBody::Block(_) => None,
                        FnBody::Expr(e) => self.infer(&e.node),
                    });
                    if let Some(t) = ret_ty {
                        if canon_ty(&t) != "Bool" {
                            self.diags.push(
                                Diagnostic::new(
                                    Code::E108,
                                    format!("`{base}.{method}` needs a `Bool` callback, got `{t}`"),
                                )
                                .with_span(self.span(args[0].value.span)),
                            );
                        }
                    }
                }
            }
            Expr::Ident(_) => {}
            _ => {
                self.diags.push(
                    Diagnostic::new(Code::E108, format!("`{base}.{method}` needs a closure"))
                        .with_span(self.span(args[0].value.span)),
                );
            }
        }
    }

    fn check_ident_use(&mut self, name: &str, span: Span) {
        if name == "this" || name == "super" {
            if !self.allow_this {
                let span = self.span(span);
                self.diags.push(
                    Diagnostic::new(Code::E108, format!("`{name}` used outside of a class method"))
                        .with_span(span)
                        .with_hint("move it into a method or `init` body"),
                );
            }
            return;
        }
        // UpperCamel names are types from the prelude or sibling modules;
        // single-file checks cannot resolve sibling modules, so only
        // lowercase value names are flagged here.
        if starts_uppercase(name) {
            self.use_name(name);
            return;
        }
        if !self.use_name(name) {
            self.err_undefined(name, span);
        }
    }

    fn promise_call_ret(declared: Option<String>, is_async: bool) -> Option<String> {
        if !is_async {
            return declared;
        }
        match declared {
            None => Some("Promise<Int>".to_string()),
            Some(t) if crate::prelude::short_name(&t) == "Promise" => Some(t),
            Some(t) => Some(format!("Promise<{t}>")),
        }
    }

    fn infer(&self, e: &Expr) -> Option<String> {
        match e {
            Expr::Int(_) => Some("Int".to_string()),
            Expr::Float(_) => Some("Float".to_string()),
            Expr::Bool(_) => Some("Bool".to_string()),
            Expr::Interp(_) => Some("String".to_string()),
            Expr::Null => None,
            Expr::Array(_) => Some("Array".to_string()),
            Expr::MapLiteral(_) => Some("Map".to_string()),
            Expr::Tuple(_) | Expr::TupleGet { .. } => None,
            Expr::Ident(n) => self.lookup(n).flatten(),
            Expr::Index { base, .. } => match &base.node {
                Expr::Ident(n) => self.lookup_elem(n),
                _ => None,
            },
            Expr::Binary { op, lhs, rhs } => match op {
                BinOp::Eq | BinOp::NotEq | BinOp::Lt | BinOp::LtEq | BinOp::Gt | BinOp::GtEq
                | BinOp::And | BinOp::Or => Some("Bool".to_string()),
                _ => {
                    let l = self.infer(&lhs.node);
                    let r = self.infer(&rhs.node);
                    match (l.as_deref(), r.as_deref()) {
                        (Some("Vec4f"), Some("Vec4f")) => Some("Vec4f".to_string()),
                        (Some("Vec4i"), Some("Vec4i")) => Some("Vec4i".to_string()),
                        (Some("Float"), _) | (_, Some("Float")) => Some("Float".to_string()),
                        (Some("Int"), Some("Int")) => Some("Int".to_string()),
                        _ => None,
                    }
                }
            },
            Expr::Unary { op, rhs } => match op {
                UnOp::Not => Some("Bool".to_string()),
                UnOp::BitNot => self.infer(&rhs.node),
                UnOp::Neg => self.infer(&rhs.node),
                UnOp::PreInc | UnOp::PreDec => self.infer(&rhs.node),
                UnOp::AddrOf => None,
                UnOp::Deref => None,
            },
            Expr::Postfix { expr, .. } => self.infer(&expr.node),
            Expr::Ternary { then, .. } => self.infer(&then.node),
            Expr::Coalesce { rhs, .. } => self.infer(&rhs.node),
            Expr::OptChain { .. } | Expr::OptCall { .. } => None,
            Expr::Call { callee, trailing, .. } => {
                if trailing.is_some() {
                    return None;
                }
                match &callee.node {
                    Expr::Ident(n) => {
                        if n == "typeOf" {
                            return Some("String".to_string());
                        }
                        if let Some(ret) = self.fn_rets.get(n).cloned() {
                            return Self::promise_call_ret(ret, self.async_fns.contains(n));
                        }
                        if self.globals.contains(n) {
                            return Some(n.clone());
                        }
                        match crate::prelude::short_name(n.as_str()) {
                            "Int" => Some("Int".to_string()),
                            "Float" => Some("Float".to_string()),
                            "Bool" => Some("Bool".to_string()),
                            "String" => Some("String".to_string()),
                            _ => None,
                        }
                    }
                    Expr::Member { base, field } => {
                        if let Some(base_ty) = self.infer(&base.node) {
                            if let Some((_, _, ret)) = builtin_method_sig(
                                crate::prelude::short_name(&base_ty),
                                field,
                            ) {
                                return ret.map(|r| r.to_string());
                            }
                            if field == "wait" {
                                if let Some(inner) = promise_inner(&base_ty) {
                                    return Some(format!("Result<{inner}, String>"));
                                }
                            }
                        }
                        if let Expr::Ident(root) = &base.node {
                            let short = crate::prelude::short_name(root).to_string();
                            let qualified = format!("{short}.{field}");
                            if let Some(ret) = self.fn_rets.get(&qualified).cloned() {
                                return Self::promise_call_ret(ret, self.async_fns.contains(&qualified));
                            }
                            if let Some(ret) = self.fn_rets.get(field).cloned() {
                                return Self::promise_call_ret(ret, self.async_fns.contains(field));
                            }
                            let key = (short, field.clone());
                            if self.async_methods.contains(&key) {
                                return Self::promise_call_ret(None, true);
                            }
                        }
                        None
                    }
                    _ => None,
                }
            }
            Expr::Cast { ty, .. } => ty_name(ty),
            Expr::Is { .. } => Some("Bool".to_string()),
            Expr::New { target, .. } => {
                Some(crate::prelude::short_name(target).to_string())
            }
            Expr::Await(inner) => self.infer(&inner.node).and_then(|t| promise_inner(&t)),
            _ => None,
        }
    }

    fn assignable(&self, target_ty: &str, val_ty: &str) -> bool {
        if canon_ty(target_ty) == "Any" {
            return true;
        }
        let target = canon_ty(target_ty);
        let val = canon_ty(val_ty);
        if target == val {
            return true;
        }
        if target == "Float" && val == "Int" {
            return true;
        }
        let target_short = crate::prelude::short_name(&target).to_string();
        let val_short = crate::prelude::short_name(&val).to_string();
        if target_short == val_short {
            return true;
        }
        self.conforms
            .get(&val_short)
            .is_some_and(|v| v.iter().any(|i| i == &target_short))
            || self
                .variants
                .get(&val_short)
                .is_some_and(|e| e == &target_short)
    }

    fn err_assign_mismatch(&mut self, name: &str, target_ty: &str, val_ty: &str, span: Span) {
        let target_show = canon_ty(target_ty);
        let val_show = canon_ty(val_ty);
        self.diags.push(
            Diagnostic::new(
                Code::E205,
                format!(
                    "type mismatch: cannot assign value of type `{val_show}` to variable `{name}` of type `{target_show}`"
                ),
            )
            .with_span(self.span(span))
            .with_hint(format!("expected `{target_show}`, found `{val_show}`")),
        );
    }

    fn check_return(&mut self, expected: &Option<String>, value: &Spanned<Expr>) {        let exp = match expected {
            Some(e) if e != "Void" => canon_ty(e),
            _ => return,
        };
        let got = match self.infer(&value.node) {
            Some(g) => canon_ty(&g),
            None => return,
        };
        if got == exp {
            return;
        }
        if exp == "Float" && got == "Int" {
            return;
        }
        let got_short = crate::prelude::short_name(&got).to_string();
        let exp_short = crate::prelude::short_name(&exp).to_string();
        if got_short != exp_short
            && self
                .conforms
                .get(&got_short)
                .is_some_and(|v| v.iter().any(|i| i == &exp_short))
        {
            return;
        }
        if self
            .variants
            .get(&got_short)
            .is_some_and(|e| e == &exp_short)
        {
            return;
        }
        self.diags.push(
            Diagnostic::new(
                Code::E304,
                format!("type mismatch in return statement: expected `{exp}`, got `{got}`"),
            )
            .with_span(self.span(value.span))
            .with_hint("return a value matching the declared return type"),
        );
    }

    fn check_spread(&mut self, it: &ArrayElem) {
        let is_array = self
            .infer(&it.expr.node)
            .as_deref()
            .map(|t| t.rsplit('.').next().unwrap_or(t).split('<').next().unwrap_or("") == "Array")
            .unwrap_or(false);
        if !is_array {
            self.diags.push(
                Diagnostic::new(Code::E108, "spread `...` needs an Array value")
                    .with_span(self.span(it.expr.span)),
            );
        }
    }

    fn check_incdec_target(&mut self, target: &Expr, span: Span, what: &str) {
        match target {
            Expr::Ident(n) => {
                if self.binding_mutable(n) == Some(false) || self.consts.contains(n) {
                    self.diags.push(
                        Diagnostic::new(Code::E108, format!("cannot mutate `const` variable `{n}` with `{what}`"))
                            .with_span(self.span(span))
                            .with_hint("declare it with `let` to allow reassignment"),
                    );
                }
            }
            Expr::Member { .. } | Expr::Index { .. } => {}
            Expr::Unary { op, .. } if matches!(op, UnOp::Deref) => {}
            _ => {
                self.diags.push(
                    Diagnostic::new(Code::E108, format!("invalid target for {what} operator"))
                        .with_span(self.span(span)),
                );
            }
        }
    }

    pub(super) fn walk_expr(&mut self, e: &Spanned<Expr>) {
        match &e.node {
            Expr::Ident(n) => {
                if n == "Some" || n == "None" {
                    if !self.deny_removed_value(n, e.span) {
                        self.check_ident_use(n, e.span);
                    }
                    return;
                }
                self.check_ident_use(n, e.span)
            }            Expr::This => self.check_ident_use("this", e.span),
            Expr::Super => self.check_ident_use("super", e.span),
            Expr::Bool(_) | Expr::Null | Expr::Int(_) | Expr::Float(_) => {}
            Expr::Interp(parts) => {
                for p in parts {
                    if let InterpPart::Expr(inner) = p {
                        self.walk_expr(inner);
                    }
                }
            }
            Expr::Array(items) => {
                for it in items {
                    if it.spread {
                        self.check_spread(it);
                    }
                    self.walk_expr(&it.expr);
                }
            }
            Expr::Tuple(items) => {
                for it in items {
                    self.walk_expr(it);
                }
            }
            Expr::TupleGet { base, .. } => self.walk_expr(base),
            Expr::Record(fields) => {
                for e in fields {
                    self.walk_expr(e.value());
                }
            }
            Expr::MapLiteral(entries) => {
                for e in entries {
                    self.walk_expr(e.value());
                }
            }
            Expr::Binary { lhs, rhs, .. } => {
                self.walk_expr(lhs);
                self.walk_expr(rhs);
            }
            Expr::Unary { op, rhs } => {
                self.walk_expr(rhs);
                if matches!(op, UnOp::PreInc | UnOp::PreDec) {
                    self.check_incdec_target(&rhs.node, rhs.span, "increment");
                }
            }
            Expr::Postfix { expr, .. } => {
                self.walk_expr(expr);
                self.check_incdec_target(&expr.node, expr.span, "increment");
            }
            Expr::Ternary { cond, then, otherwise } => {
                self.walk_expr(cond);
                self.walk_expr(then);
                self.walk_expr(otherwise);
            }
            Expr::Range { lo, hi, .. } => {
                self.walk_expr(lo);
                self.walk_expr(hi);
            }
            Expr::Call { callee, args, trailing, .. } => {
                if let Expr::ImplicitMember(path) = &callee.node {
                    let cspan = callee.span;
                    let payload = args.clone();
                    self.check_implicit_member(path, cspan);
                    let outer = self.enum_hint.clone();
                    self.enum_hint = None;
                    for a in &payload {
                        self.walk_expr(&a.value);
                    }
                    self.enum_hint = outer;
                    if let Some(b) = trailing {
                        self.walk_block(b);
                    }
                    return;
                }
                let mut params: Option<Vec<(String, Option<String>)>> = None;
                match &callee.node {
                    Expr::Ident(n) => {
                        if n == "Some" || n == "None" {
                            self.deny_removed_value(n, callee.span);
                        }
                        if starts_uppercase(n) {
                            self.use_name(n);
                        } else if !self.use_name(n) {
                            self.err_undefined(n, callee.span);
                        }
                        if self.classes.contains(n) && !self.shadowed_local(n) {
                            let short = crate::prelude::short_name(n);
                            let primitive_stub = n.starts_with("std.prelude.")
                                && matches!(
                                    short,
                                    "Int" | "Float" | "FastFloat" | "Bool" | "String" | "Char"
                                        | "Array" | "GenRef"
                                );
                            if !primitive_stub {
                                self.diags.push(
                                    Diagnostic::new(Code::E204, format!("class `{n}` cannot be invoked without `new`"))
                                        .with_span(self.span(callee.span))
                                        .with_hint(format!("use `new {n}(...)` to instantiate it")),
                                );
                            }
                        }
                        if trailing.is_none() && !self.shadowed_local(n) && !self.classes.contains(n) {
                            self.check_fn_arity(n, args, e.span);
                        }
                        if !self.shadowed_local(n) {
                            params = self.fn_params.get(n).cloned();
                        }
                    }
                    Expr::Member { base, field } => {
                        if field == "poll" {
                            self.diags.push(
                                Diagnostic::new(Code::E111, "'poll()' is an internal runtime intrinsic and cannot be invoked directly. Use 'await' or 'promise.wait()' instead.")
                                    .with_span(self.span(callee.span)),
                            );
                            return;
                        }
                        if let Expr::Ident(root) = &base.node {
                            if self.deny_removed_value(root, base.span) {
                                for a in args {
                                    self.walk_expr(&a.value);
                                }
                                if let Some(b) = trailing {
                                    self.walk_block(b);
                                }
                                return;
                            }
                        }
                        if is_removed_poll_base(&base.node) && !self.shadowed("Poll") {
                            self.diags.push(
                                Diagnostic::new(Code::E303, "unresolved identifier `Poll`")
                                    .with_span(self.span(base.span))
                                    .with_hint("`Poll` was removed; use `Promise`"),
                            );
                            return;
                        }
                        self.walk_expr(base);
                        if let Expr::Ident(root) = &base.node {
                            if root == "Float" || root.ends_with(".Float") {
                                self.check_float_static(field, args, callee.span);
                            }
                        }
                        if let Some(base_ty) = self.infer(&base.node) {
                            let short = crate::prelude::short_name(&base_ty);
                            if short == "String"
                                || short == "Array"
                                || short == "Option"
                                || short == "Result"
                                || short == "Float"
                            {
                                self.used_prelude.insert(short.to_string());
                                self.check_builtin_method(short, field, args, callee.span);
                            } else {
                                self.check_class_method(&base_ty, field, args, callee.span);
                                params = self
                                    .method_params
                                    .get(&(short.to_string(), field.clone()))
                                    .cloned()
                                    .or_else(|| {
                                        self.ext_params
                                            .get(&(short.to_string(), field.clone()))
                                            .cloned()
                                    });
                            }
                        } else if let Expr::Ident(root) = &base.node {
                            let short = crate::prelude::short_name(root).to_string();
                            if self.classes.contains(root)
                                || self.classes.contains(&short)
                                || self.globals.contains(root)
                            {
                                let key = (short.clone(), field.clone());
                                let q = format!("{short}.{field}");
                                if self.statics.contains(&key) || self.fn_arity.contains_key(&q) {
                                    if !self.shadowed_local(root) {
                                        self.check_fn_arity(&q, args, e.span);
                                    }
                                    params = self
                                        .method_params
                                        .get(&key)
                                        .cloned()
                                        .or_else(|| self.fn_params.get(&q).cloned());
                                } else if self.methods.contains_key(&key)
                                    || self.extensions.contains_key(&key)
                                {
                                    if !self.shadowed_local(root) {
                                        self.diags.push(
                                            Diagnostic::new(
                                                Code::E108,
                                                format!("`{q}` is an instance method and cannot be called on the type"),
                                            )
                                            .with_span(self.span(e.span))
                                            .with_hint(format!("call it on an instance, e.g. `new {short}(..).{field}(..)`")),
                                        );
                                    }
                                    params = self
                                        .method_params
                                        .get(&key)
                                        .cloned()
                                        .or_else(|| {
                                            self.ext_params
                                                .get(&key)
                                                .cloned()
                                        });
                                }
                            }
                        }
                    }
                    other => self.walk_expr(&Spanned { node: other.clone(), span: callee.span }),
                }
                self.walk_args_with(params, args);
                if let Some(b) = trailing {
                    self.walk_block(b);
                }
            }
            Expr::New { target, args, .. } => {
                if crate::prelude::short_name(target) == "Option" {
                    self.deny_removed_value(target, e.span);
                }
                if starts_uppercase(target) {
                    self.use_name(target);
                }
                self.check_ctor_arity(target, args, e.span);
                let short = crate::prelude::short_name(target).to_string();
                self.walk_args_with(self.ctor_params.get(&short).cloned(), args);
            }
            Expr::Index { base, index } => {
                self.walk_expr(base);
                self.walk_expr(index);
                if matches!(&index.node, Expr::Range { .. }) {
                    let is_sliceable = self
                        .infer(&base.node)
                        .as_deref()
                        .map(|t| {
                            let short = crate::prelude::short_name(t);
                            short == "Array" || short == "String"
                        })
                        .unwrap_or(true);
                    if !is_sliceable {
                        self.diags.push(
                            Diagnostic::new(Code::E108, "range index needs an Array or String target")
                                .with_span(self.span(index.span)),
                        );
                    }
                }
            }
            Expr::Member { base, field } => {
                if let Expr::Ident(root) = &base.node {
                    if self.deny_removed_value(root, base.span) {
                        return;
                    }
                }
                if is_removed_poll_base(&base.node) && !self.shadowed("Poll") {
                    self.diags.push(
                        Diagnostic::new(Code::E303, "unresolved identifier `Poll`")
                            .with_span(self.span(base.span))
                            .with_hint("`Poll` was removed; use `Promise`"),
                    );
                    return;
                }
                self.walk_expr(base);
                if field == "length" || field == "len" {
                    if let Some(base_ty) = self.infer(&base.node) {
                        let short = crate::prelude::short_name(&base_ty);
                        if short == "String" || short == "Array" {
                            self.used_prelude.insert(short.to_string());
                        }
                    }
                }
            }
            Expr::Coalesce { lhs, rhs } => {
                self.walk_expr(lhs);
                self.walk_expr(rhs);
            }
            Expr::OptChain { base, .. } => {
                self.walk_expr(base);
            }
            Expr::OptCall { base, field, args } => {
                if field == "poll" {
                    self.diags.push(
                        Diagnostic::new(Code::E111, "'poll()' is an internal runtime intrinsic and cannot be invoked directly. Use 'await' or 'promise.wait()' instead.")
                            .with_span(self.span(e.span)),
                    );
                    return;
                }
                self.walk_expr(base);
                let mut oparams: Option<Vec<(String, Option<String>)>> = None;
                if let Some(base_ty) = self.infer(&base.node) {
                    let short = crate::prelude::short_name(&base_ty);
                    if short == "String" || short == "Array" {
                        self.check_builtin_method(short, field, args, e.span);
                    } else {
                        self.check_class_method(&base_ty, field, args, e.span);
                        oparams = self
                            .method_params
                            .get(&(short.to_string(), field.clone()))
                            .cloned()
                            .or_else(|| {
                                self.ext_params
                                    .get(&(short.to_string(), field.clone()))
                                    .cloned()
                            });
                    }
                }
                self.walk_args_with(oparams, args);
            }
            Expr::Cast { ty, expr } => {
                self.note_ty(ty);
                self.deny_removed_ty(ty, e.span);
                self.walk_expr(expr);
            }
            Expr::Is { base, target } => {
                self.note_ty(target);
                self.deny_removed_ty(target, e.span);
                self.walk_expr(base);
            }
            Expr::ImplicitMember(path) => self.check_implicit_member(path, e.span),
            Expr::Macro { name, args, .. } => {
                if name == "Assert" || name == "PanicIf" {
                    for a in args {
                        self.walk_expr(a);
                    }
                }
            }
            Expr::Closure { decay, params, ret, body, .. } => {
                let outer_this = self.allow_this;
                let outer_exp = self.expected.clone();
                let outer_hint = self.enum_hint.clone();
                self.enum_hint = None;
                if *decay {
                    self.allow_this = true;
                }
                for p in params.iter() {
                    if let Some(t) = p.ty.as_ref() {
                        self.deny_removed_ty(t, e.span);
                    }
                }
                if let Some(t) = ret.as_ref() {
                    self.deny_removed_ty(t, e.span);
                }
                self.expected = ret.as_ref().and_then(ty_name);
                self.scopes.push(BTreeMap::new());
                for p in params {
                    self.declare(p.name.clone(), p.ty.as_ref().and_then(ty_name), true);
                    self.declare_elem(&p.name, p.ty.as_ref().and_then(array_elem));
                }
                match body {
                    FnBody::Block(b) => self.walk_block(b),
                    FnBody::Expr(inner) => {
                        self.walk_expr(inner);
                        let exp = self.expected.clone();
                        self.check_return(&exp, inner);
                    }
                }
                self.scopes.pop();
                self.allow_this = outer_this;
                self.expected = outer_exp;
                self.enum_hint = outer_hint;
            }
            Expr::UnsafeBlock(b) => self.walk_block(b),
            Expr::Await(inner) | Expr::Propagate(inner) => self.walk_expr(inner),
            Expr::Switch { scrutinee, cases, default } => {
                self.walk_expr(scrutinee);
                for c in cases {
                    let mut binds = Vec::new();
                    self.pattern_binds(&c.pattern, &mut binds);
                    if let Pattern::Is(t) = &c.pattern {
                        if let Expr::Ident(n) = &scrutinee.node {
                            binds.push((n.clone(), ty_name(t).map(|t| crate::prelude::short_name(&t).to_string())));
                        }
                    }
                    self.scopes.push(BTreeMap::new());
                    for (n, ty) in binds {
                        self.declare(n, ty, true);
                    }
                    if let Some(g) = &c.guard {
                        self.walk_expr(g);
                    }
                    match &c.body {
                        crate::ast::SwitchExprBody::Expr(e) => self.walk_expr(e),
                        crate::ast::SwitchExprBody::Block(b) => self.walk_block(b),
                    }
                    self.scopes.pop();
                }
                if let Some(d) = default {
                    match d {
                        crate::ast::SwitchExprBody::Expr(e) => self.walk_expr(e),
                        crate::ast::SwitchExprBody::Block(b) => self.walk_block(b),
                    }
                }
            }
        }
    }

    fn pattern_binds(&mut self, pat: &Pattern, out: &mut Vec<(String, Option<String>)>) {
        match pat {
            Pattern::Enum { path, args } => {
                let mut payloads: Vec<Option<String>> = Vec::new();
                if let Some(vname) = path.last() {
                    let qual = if path.len() > 1 { Some(&path[path.len() - 2]) } else { None };
                    for decl in &self.module.decls {
                        if let Decl::Enum { name, members, type_params, .. } = &decl.node {
                            if let Some(q) = qual {
                                if crate::prelude::short_name(name) != q {
                                    continue;
                                }
                            }
                            for m in members {
                                if &m.name == vname {
                                    payloads = m.payload.iter().map(|t| {
                                        let n = ty_name(t);
                                        match &n {
                                            Some(s) if type_params.iter().any(|p| p == s) => None,
                                            other => other.clone(),
                                        }
                                    }).collect();
                                }
                            }
                        }
                    }
                }
                for (i, a) in args.iter().enumerate() {
                    match a {
                        Pattern::Literal(e) => {
                            if let Expr::Ident(n) = &e.node {
                                out.push((n.clone(), payloads.get(i).cloned().flatten()));
                            }
                        }
                        nested => self.pattern_binds(nested, out),
                    }
                }
            }
            Pattern::Literal(e) => {
                if let Expr::Ident(_) = &e.node {
                } else {
                    self.pattern_uses(pat, &[]);
                }
            }
            Pattern::Range { lo, hi, .. } => {
                self.walk_expr(lo);
                self.walk_expr(hi);
            }
            Pattern::Is(_) | Pattern::Wildcard => {}
        }
    }

    fn pattern_uses(&mut self, pat: &Pattern, bound: &[String]) {
        match pat {
            Pattern::Literal(e) => {
                if let Expr::Ident(n) = &e.node {
                    if !bound.contains(n) {
                        self.walk_expr(e);
                    }
                } else {
                    self.walk_expr(e);
                }
            }
            Pattern::Range { lo, hi, .. } => {
                self.walk_expr(lo);
                self.walk_expr(hi);
            }
            Pattern::Enum { args, .. } => {
                for a in args {
                    self.pattern_uses(a, bound);
                }
            }
            Pattern::Is(_) | Pattern::Wildcard => {}
        }
    }

    pub(super) fn walk_block(&mut self, b: &Block) {
        self.scopes.push(BTreeMap::new());
        for st in &b.stmts {
            self.walk_stmt(&st.node, st.span);
        }
        self.scopes.pop();
    }

    pub(super) fn walk_fn_body(&mut self, params: &[Param], ret: &Option<Type>, body: &FnBody, allow_this: bool, sig_span: Span) {
        let outer = self.allow_this;
        let outer_exp = self.expected.clone();
        self.allow_this = allow_this;
        for p in params {
            if let Some(t) = p.ty.as_ref() {
                self.note_ty(t);
                self.deny_removed_ty(t, p.span);
            }
        }
        if let Some(t) = ret.as_ref() {
            self.note_ty(t);
            self.deny_removed_ty(t, sig_span);
        }
        self.expected = ret.as_ref().and_then(ty_name);
        self.scopes.push(BTreeMap::new());
        for p in params {
            self.declare(p.name.clone(), p.ty.as_ref().and_then(ty_name), true);
            self.declare_elem(&p.name, p.ty.as_ref().and_then(array_elem));
        }
        match body {
            FnBody::Block(b) => self.walk_block(b),
            FnBody::Expr(e) => {
                self.walk_expr(e);
                let exp = self.expected.clone();
                self.check_return(&exp, e);
            }
        }
        self.scopes.pop();
        self.allow_this = outer;
        self.expected = outer_exp;
    }

    fn walk_stmt(&mut self, s: &Stmt, span: Span) {
        let outer_span = self.stmt_span;
        if span.start != 0 || span.end != 0 {
            self.stmt_span = span;
        }
        self.walk_stmt_inner(s);
        self.stmt_span = outer_span;
    }

    fn walk_stmt_inner(&mut self, s: &Stmt) {
        match s {
            Stmt::Var { name, ty, value, mutable, .. } => {
                if let Some(t) = ty {
                    self.note_ty(t);
                    let span = self.stmt_span;
                    self.deny_removed_ty(t, span);
                }
                let outer_hint = self.enum_hint.clone();
                self.enum_hint = ty.as_ref().and_then(ty_name);
                self.walk_expr(value);
                self.enum_hint = outer_hint;
                let inferred = ty.as_ref().and_then(ty_name).or_else(|| self.infer(&value.node));
                if let (Some(t), Some(v)) = (
                    ty.as_ref().and_then(ty_name),
                    self.infer(&value.node),
                ) {
                    if !self.assignable(&t, &v) {
                        self.err_assign_mismatch(name, &t, &v, value.span);
                    }
                }
                self.declare(name.clone(), inferred, *mutable);
                self.declare_elem(name, ty.as_ref().and_then(array_elem));
            }
            Stmt::DestructureTuple { names, ty, value, .. } => {
                if let Some(t) = ty {
                    self.note_ty(t);
                    let span = self.stmt_span;
                    self.deny_removed_ty(t, span);
                }
                self.walk_expr(value);
                for n in names {
                    self.declare(n.clone(), None, true);
                }
            }
            Stmt::DestructureRecord { fields, rest, value, .. } => {
                self.walk_expr(value);
                for (_, local) in fields {
                    self.declare(local.clone(), None, true);
                }
                if let Some(r) = rest {
                    self.declare(r.clone(), None, true);
                }
            }
            Stmt::DestructureArray { names, rest, value, .. } => {
                self.walk_expr(value);
                for n in names.iter().chain(rest.iter()) {
                    self.declare(n.clone(), None, true);
                }
            }
            Stmt::Assign { target, value, op, .. } => {
                self.walk_expr(value);
                self.walk_expr(target);
                if let Expr::Ident(n) = &target.node {
                    if self.binding_mutable(n) == Some(false) || self.consts.contains(n) {
                        self.diags.push(
                            Diagnostic::new(Code::E108, format!("cannot assign to `const` variable `{n}`"))
                                .with_span(self.span(target.span))
                                .with_hint("declare it with `let` to allow reassignment"),
                        );
                        return;
                    }
                    if matches!(op, AssignOp::Eq) {
                        let target_ty = self.lookup(n).flatten();
                        let val_ty = self.infer(&value.node);
                        match (target_ty, val_ty) {
                            (Some(t), Some(v)) => {
                                if !self.assignable(&t, &v) {
                                    self.err_assign_mismatch(n, &t, &v, value.span);
                                }
                            }
                            _ => self.undefine(n),
                        }
                    } else {
                        self.undefine(n);
                    }
                }
            }
            Stmt::Expr(e) => self.walk_expr(e),
            Stmt::If { cond, then, otherwise } => {
                match cond {
                    IfCond::Expr(e) => {
                        self.walk_expr(e);
                        if let Expr::Is { base, target } = &e.node {
                            if let Expr::Ident(n) = &base.node {
                                self.scopes.push(BTreeMap::new());
                                self.declare(n.clone(), ty_name(target).map(|t| crate::prelude::short_name(&t).to_string()), true);
                                self.walk_block(then);
                                self.scopes.pop();
                                if let Some(el) = otherwise {
                                    self.walk_else(el);
                                }
                                return;
                            }
                        }
                    }
                    IfCond::Let { name, value } => {
                        self.walk_expr(value);
                        self.scopes.push(BTreeMap::new());
                        let ty = self.infer(&value.node);
                        self.declare(name.clone(), ty, true);
                        self.walk_block(then);
                        self.scopes.pop();
                        if let Some(el) = otherwise {
                            self.walk_else(el);
                        }
                        return;
                    }
                }
                self.walk_block(then);
                if let Some(el) = otherwise {
                    self.walk_else(el);
                }
            }
            Stmt::For { binding, iter, body } => {
                self.walk_expr(iter);
                self.scopes.push(BTreeMap::new());
                let range_elem = matches!(iter.node, Expr::Range { .. });
                match binding {
                    ForBinding::One(n) => {
                        let ty = range_elem.then(|| "Int".to_string());
                        self.declare(n.clone(), ty, true);
                    }
                    ForBinding::Many(ns) => {
                        for n in ns {
                            self.declare(n.clone(), None, true);
                        }
                    }
                }
                self.walk_block(body);
                self.scopes.pop();
            }
            Stmt::While { cond, body } => {
                self.walk_expr(cond);
                self.walk_block(body);
            }
            Stmt::DoWhile { body, cond } => {
                self.walk_block(body);
                self.walk_expr(cond);
            }
            Stmt::Switch { scrutinee, cases, default } => {
                self.walk_expr(scrutinee);
                for c in cases {
                    let mut binds = Vec::new();
                    self.pattern_binds(&c.pattern, &mut binds);
                    if let Pattern::Is(t) = &c.pattern {
                        if let Expr::Ident(n) = &scrutinee.node {
                            binds.push((n.clone(), ty_name(t).map(|t| crate::prelude::short_name(&t).to_string())));
                        }
                    }
                    let names: Vec<String> = binds.iter().map(|(n, _)| n.clone()).collect();
                    self.scopes.push(BTreeMap::new());
                    for (n, ty) in binds {
                        self.declare(n, ty, true);
                    }
                    self.pattern_uses(&c.pattern, &names);
                    if let Some(g) = &c.guard {
                        self.walk_expr(g);
                    }
                    for st in &c.body {
                        self.walk_stmt(&st.node, st.span);
                    }
                    self.scopes.pop();
                }
                if let Some(stmts) = default {
                    self.scopes.push(BTreeMap::new());
                    for st in stmts {
                        self.walk_stmt(&st.node, st.span);
                    }
                    self.scopes.pop();
                }
            }
            Stmt::Return(opt) => {
                if let Some(e) = opt {
                    self.walk_expr(e);
                    let exp = self.expected.clone();
                    self.check_return(&exp, e);
                }
            }
            Stmt::Assert(e) => self.walk_expr(e),
            Stmt::Defer(b) => self.walk_block(b),
            Stmt::Guard { name, value, otherwise } => {
                self.walk_expr(value);
                let ty = self.infer(&value.node);
                self.declare(name.clone(), ty, true);
                self.walk_block(otherwise);
            }
            Stmt::Try { body, catch, finally } => {
                self.walk_block(body);
                if let Some((name, block)) = catch {
                    self.scopes.push(BTreeMap::new());
                    self.declare(name.clone(), None, true);
                    self.walk_block(block);
                    self.scopes.pop();
                }
                if let Some(f) = finally {
                    self.walk_block(f);
                }
            }
            Stmt::Throw(opt) => {
                if let Some(e) = opt {
                    self.walk_expr(e);
                }
            }
            Stmt::UnsafeBlock(b) => self.walk_block(b),
            Stmt::Break | Stmt::Continue | Stmt::Fallthrough | Stmt::Pass | Stmt::Empty => {}
        }
    }

    fn walk_else(&mut self, el: &Else) {
        match el {
            Else::Block(b) => self.walk_block(b),
            Else::If(st) => self.walk_stmt(&st.node, st.span),
        }
    }
}
