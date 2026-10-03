use crate::ast as A;
use crate::ast::sp;
use diagnostics::{Code, Diagnostic, Span};

const Z: Span = Span { start: 0, end: 0 };

pub fn strip_benches(module: &mut A::Module) {
    module
        .decls
        .retain(|d| !matches!(&d.node, A::Decl::Fn(f) if f.is_bench));
}

pub fn collect_benches(
    module: &A::Module,
    filter: Option<&str>,
) -> Result<Vec<(String, String)>, Diagnostic> {
    let mut out = Vec::new();
    for decl in &module.decls {
        if let A::Decl::Fn(f) = &decl.node {
            if !f.is_bench {
                continue;
            }
            let display = f
                .name
                .rsplit('.')
                .next()
                .unwrap_or(&f.name)
                .strip_prefix("__rnx_bench_")
                .unwrap_or(&f.name)
                .to_string();
            if let Some(sub) = filter
                && !display.contains(sub)
            {
                continue;
            }
            if out.iter().any(|(d, _)| d == &display) {
                return Err(Diagnostic::new(
                    Code::E108,
                    format!("duplicate bench `{display}`"),
                )
                .with_span(decl.span));
            }
            out.push((display, f.name.clone()));
        }
    }
    out.sort();
    Ok(out)
}

pub fn strip_tests(module: &mut A::Module) {
    module
        .decls
        .retain(|d| !matches!(&d.node, A::Decl::Fn(f) if f.is_test));
}

pub fn collect_tests(
    module: &A::Module,
    filter: Option<&str>,
    exact: bool,
) -> Result<Vec<String>, Diagnostic> {
    let mut names: Vec<String> = Vec::new();
    for decl in &module.decls {
        if let A::Decl::Fn(f) = &decl.node {
            if !f.is_test {
                continue;
            }
            if !f.params.is_empty() {
                return Err(Diagnostic::new(
                    Code::E108,
                    format!("test function `{}` must take no parameters", f.name),
                )
                .with_span(decl.span));
            }
            if let Some(sub) = filter
                && !(if exact { f.name == sub } else { f.name.contains(sub) })
            {
                continue;
            }
            names.push(f.name.clone());
        }
    }
    names.sort();
    Ok(names)
}

pub fn display_for(files: &[(String, String)], name: &str) -> String {
    let mut best: Option<&String> = None;
    let mut best_len = 0usize;
    for (prefix, display) in files {
        if (name == prefix || name.starts_with(&format!("{prefix}."))) && prefix.len() > best_len
        {
            best = Some(display);
            best_len = prefix.len();
        }
    }
    match best {
        Some(display) => display.clone(),
        None => match name.rfind('.') {
            Some(i) => name[..i].to_string(),
            None => "root".to_string(),
        },
    }
}

pub fn generate_test_harness(
    module: &mut A::Module,
    filter: Option<&str>,
    exact: bool,
    files: &[(String, String)],
) -> Result<Vec<String>, Diagnostic> {
    let names = collect_tests(module, filter, exact)?;
    module.decls.retain(|d| !matches!(&d.node, A::Decl::Fn(f) if f.name == "Main"));
    module.decls.retain(|d| !matches!(&d.node, A::Decl::Stmt(_)));
    module.decls.push(synthesize_main(&names, files));
    Ok(names)
}

fn ident(name: &str) -> A::Spanned<A::Expr> {
    sp(A::Expr::Ident(name.to_string()), Z)
}

fn strlit(text: &str) -> A::Spanned<A::Expr> {
    sp(A::Expr::Interp(vec![A::InterpPart::Text(text.to_string())]), Z)
}

fn intlit(n: i64) -> A::Spanned<A::Expr> {
    sp(A::Expr::Int(n), Z)
}

fn call(name: &str, args: Vec<A::Spanned<A::Expr>>) -> A::Spanned<A::Expr> {
    sp(
        A::Expr::Call {
            callee: Box::new(ident(name)),
            type_args: Vec::new(),
            args: args
                .into_iter()
                .map(|value| A::CallArg { name: None, value })
                .collect(),
            trailing: None,
        },
        Z,
    )
}

fn stmt(node: A::Stmt) -> A::Spanned<A::Stmt> {
    sp(node, Z)
}

fn print_stmt(args: Vec<A::Spanned<A::Expr>>) -> A::Spanned<A::Stmt> {
    stmt(A::Stmt::Expr(call("print", args)))
}

fn counter(name: &str) -> A::Spanned<A::Stmt> {
    stmt(A::Stmt::Var {
        mutable: true,
        is_static: false,
        name: name.to_string(),
        ty: Some(A::Type::named("Int")),
        value: intlit(0),
    })
}

fn bump(name: &str) -> A::Spanned<A::Stmt> {
    stmt(A::Stmt::Assign {
        target: ident(name),
        op: A::AssignOp::PlusEq,
        value: intlit(1),
    })
}

fn binop(op: A::BinOp, lhs: A::Spanned<A::Expr>, rhs: A::Spanned<A::Expr>) -> A::Spanned<A::Expr> {
    sp(
        A::Expr::Binary {
            op,
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
        },
        Z,
    )
}

fn concat(parts: Vec<A::Spanned<A::Expr>>) -> A::Spanned<A::Expr> {
    let mut iter = parts.into_iter();
    let first = iter.next().expect("concat needs at least one part");
    iter.fold(first, |acc, part| binop(A::BinOp::Add, acc, part))
}

fn var_str(name: &str) -> A::Spanned<A::Stmt> {
    stmt(A::Stmt::Var {
        mutable: true,
        is_static: false,
        name: name.to_string(),
        ty: Some(A::Type::named("String")),
        value: strlit(""),
    })
}

fn if_else(
    cond: A::Spanned<A::Expr>,
    then: Vec<A::Spanned<A::Stmt>>,
    otherwise: Vec<A::Spanned<A::Stmt>>,
) -> A::Spanned<A::Stmt> {
    stmt(A::Stmt::If {
        cond: A::IfCond::Expr(cond),
        then: A::Block { stmts: then },
        otherwise: Some(A::Else::Block(A::Block { stmts: otherwise })),
    })
}

fn ms_format_stmts(dt: &str, whole: &str, fracs: &str, frac: &str, hund: &str) -> Vec<A::Spanned<A::Stmt>> {
    vec![
        stmt(A::Stmt::Var {
            mutable: true,
            is_static: false,
            name: hund.to_string(),
            ty: Some(A::Type::named("Int")),
            value: binop(
                A::BinOp::Div,
                binop(A::BinOp::Mul, ident(dt), intlit(100)),
                intlit(1_000_000),
            ),
        }),
        stmt(A::Stmt::Var {
            mutable: true,
            is_static: false,
            name: whole.to_string(),
            ty: Some(A::Type::named("Int")),
            value: binop(A::BinOp::Div, ident(hund), intlit(100)),
        }),
        stmt(A::Stmt::Var {
            mutable: true,
            is_static: false,
            name: frac.to_string(),
            ty: Some(A::Type::named("Int")),
            value: binop(A::BinOp::Mod, ident(hund), intlit(100)),
        }),
        var_str(fracs),
        if_else(
            binop(A::BinOp::Lt, ident(frac), intlit(10)),
            vec![stmt(A::Stmt::Assign {
                target: ident(fracs),
                op: A::AssignOp::Eq,
                value: concat(vec![strlit("0"), ident(frac)]),
            })],
            vec![stmt(A::Stmt::Assign {
                target: ident(fracs),
                op: A::AssignOp::Eq,
                value: concat(vec![strlit(""), ident(frac)]),
            })],
        ),
    ]
}

fn short_of(name: &str) -> &str {
    match name.rfind('.') {
        Some(i) => &name[i + 1..],
        None => name,
    }
}

fn sanitized(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

pub fn synthesize_main(names: &[String], files: &[(String, String)]) -> A::Spanned<A::Decl> {
    let width = names
        .iter()
        .map(|n| short_of(n).chars().count())
        .max()
        .unwrap_or(1);
    let mut stmts = vec![
        stmt(A::Stmt::Var {
            mutable: true,
            is_static: false,
            name: "t_all".to_string(),
            ty: Some(A::Type::named("Int")),
            value: call("__rnx_clock_mono", Vec::new()),
        }),
        counter("passed"),
        counter("failed"),
    ];
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    for name in names {
        let display = display_for(files, name);
        match groups.last_mut() {
            Some((m, list)) if *m == display => list.push(name.clone()),
            _ => groups.push((display, vec![name.clone()])),
        }
    }
    let mut first_group = true;
    for (display, tests) in &groups {
        if !first_group {
            stmts.push(print_stmt(vec![strlit("")]));
        }
        first_group = false;
        stmts.push(print_stmt(vec![strlit(display)]));
        for name in tests {
            let short = short_of(name);
            let padded = format!("{short:<width$}");
            let base = sanitized(name);
            let t0 = format!("t0_{base}");
            let dt = format!("dt_{base}");
            let hund = format!("hund_{base}");
            let whole = format!("whole_{base}");
            let frac = format!("frac_{base}");
            let fracs = format!("fracs_{base}");
            stmts.push(stmt(A::Stmt::Var {
                mutable: true,
                is_static: false,
                name: t0.clone(),
                ty: Some(A::Type::named("Int")),
                value: call("__rnx_clock_mono", Vec::new()),
            }));
            stmts.push(stmt(A::Stmt::Expr(call(name, Vec::new()))));
            stmts.push(stmt(A::Stmt::Var {
                mutable: true,
                is_static: false,
                name: dt.clone(),
                ty: Some(A::Type::named("Int")),
                value: binop(
                    A::BinOp::Sub,
                    call("__rnx_clock_mono", Vec::new()),
                    ident(&t0),
                ),
            }));
            stmts.extend(ms_format_stmts(&dt, &whole, &fracs, &frac, &hund));
            let tail = concat(vec![
                strlit(&format!(" {padded} ")),
                ident(&whole),
                strlit("."),
                ident(&fracs),
                strlit(" ms"),
            ]);
            stmts.push(if_else(
                call("__testCheck", Vec::new()),
                vec![
                    print_stmt(vec![concat(vec![strlit("  ✗"), tail.clone()])]),
                    bump("failed"),
                    print_stmt(vec![strlit(&format!("┌─ failure in {display} > {short}"))]),
                    print_stmt(vec![strlit(&format!(
                        "└─ note: rerun with `rnx test {short}`"
                    ))]),
                ],
                vec![
                    print_stmt(vec![concat(vec![strlit("  ✓"), tail])]),
                    bump("passed"),
                ],
            ));
        }
    }
    stmts.push(print_stmt(vec![strlit("")]));
    stmts.push(stmt(A::Stmt::Var {
        mutable: true,
        is_static: false,
        name: "dt_all".to_string(),
        ty: Some(A::Type::named("Int")),
        value: binop(
            A::BinOp::Sub,
            call("__rnx_clock_mono", Vec::new()),
            ident("t_all"),
        ),
    }));
    stmts.extend(ms_format_stmts(
        "dt_all",
        "whole_all",
        "fracs_all",
        "frac_all",
        "hund_all",
    ));
    stmts.push(print_stmt(vec![concat(vec![
        strlit("tests: "),
        ident("passed"),
        strlit(" passed, "),
        ident("failed"),
        strlit(" failed in "),
        ident("whole_all"),
        strlit("."),
        ident("fracs_all"),
        strlit(" ms"),
    ])]));
    stmts.push(stmt(A::Stmt::Return(Some(ident("failed")))));
    sp(
        A::Decl::Fn(A::FnDecl {
            docs: String::new(),
            access: A::Access::Internal,
            name: "Main".to_string(),
            type_params: Vec::new(),
            params: Vec::new(),
            ret: Some(A::Type::named("Int")),
            throws: false,
            is_unsafe: false,
            is_async: false,
            is_static: false,
            is_test: false,
            is_bench: false,
            body: A::FnBody::Block(A::Block { stmts }),
            attrs: Vec::new(),
        }),
        Z,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn module_of(src: &str) -> A::Module {
        crate::parser::Parser::parse_module(src).unwrap()
    }

    #[test]
    fn collects_sorts_and_filters_tests() {
        let m = module_of(
            "test fn test_zebra() { return; }\nfn helper(): Int { return 1; }\ntest fn test_alpha() { return; }\n",
        );
        let names = collect_tests(&m, None, false).unwrap();
        assert_eq!(names, vec!["test_alpha".to_string(), "test_zebra".to_string()]);
        let names = collect_tests(&m, Some("alpha"), false).unwrap();
        assert_eq!(names, vec!["test_alpha".to_string()]);
    }

    #[test]
    fn rejects_test_with_params() {
        let m = module_of("test fn bad(x: Int) { return; }\n");
        let e = collect_tests(&m, None, false).unwrap_err();
        assert_eq!(e.code, Code::E108);
    }

    #[test]
    fn strip_removes_only_tests_and_harness_replaces_main() {
        let mut stripped = module_of(
            "fn Main(): Int { return 0; }\ntest fn test_one() { return; }\nfn keep(): Int { return 1; }\n",
        );
        strip_tests(&mut stripped);
        assert_eq!(stripped.decls.len(), 2);
        let mut m = module_of(
            "fn Main(): Int { return 0; }\ntest fn test_one() { return; }\nfn keep(): Int { return 1; }\n",
        );
        let names = generate_test_harness(&mut m, None, false, &[]).unwrap();
        assert_eq!(names, vec!["test_one".to_string()]);
        let mains: Vec<_> = m
            .decls
            .iter()
            .filter(|d| matches!(&d.node, A::Decl::Fn(f) if f.name == "Main"))
            .collect();
        assert_eq!(mains.len(), 1);
    }

    #[test]
    fn harness_drops_top_level_statements() {
        let mut m = module_of(
            "let x = 1;\nprint(x);\ntest fn test_one() { return; }\n",
        );
        let names = generate_test_harness(&mut m, None, false, &[]).unwrap();
        assert_eq!(names, vec!["test_one".to_string()]);
        assert!(!m.decls.iter().any(|d| matches!(&d.node, A::Decl::Stmt(_))));
    }
}

#[cfg(test)]
mod display_tests {
    use super::display_for;

    #[test]
    fn display_prefers_longest_key_prefix() {
        let files = vec![
            ("tests".to_string(), "tests.rnx".to_string()),
            ("tests.math".to_string(), "tests/math.rnx".to_string()),
        ];
        assert_eq!(display_for(&files, "tests.math.test_add"), "tests/math.rnx");
        assert_eq!(display_for(&files, "tests.other.test_x"), "tests.rnx");
    }

    #[test]
    fn display_falls_back_without_files() {
        let files: Vec<(String, String)> = Vec::new();
        assert_eq!(display_for(&files, "tests.math.test_add"), "tests.math");
        assert_eq!(display_for(&files, "test_solo"), "root");
    }
}
