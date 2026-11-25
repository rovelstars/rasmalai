use crate::ast::{Attr, Decl, Expr, Spanned, Type};
use diagnostics::{Code, Diagnostic};
use std::collections::BTreeMap;

pub fn type_suffix(name: &str) -> String {
    match name {
        "Int" => "I32".to_string(),
        "UInt" => "U32".to_string(),
        "Float" => "F64".to_string(),
        "Bool" => "Bool".to_string(),
        "String" => "String".to_string(),
        other => other.to_string(),
    }
}

fn attr_types(attrs: &[Attr]) -> Vec<String> {
    let mut out = Vec::new();
    for attr in attrs {
        if attr.name != "Monomorph" {
            continue;
        }
        for arg in &attr.args {
            if let Expr::Ident(name) = &arg.node {
                out.push(name.clone());
            }
        }
    }
    out
}

fn subst_type(ty: &mut Type, subst: &BTreeMap<String, Type>) {
    if ty.path.len() == 1
        && ty.args.is_empty()
        && ty.fn_sig.is_none()
        && ty.tuple.is_empty()
        && let Some(replacement) = subst.get(&ty.path[0])
    {
        let nullable = ty.nullable;
        *ty = replacement.clone();
        ty.nullable |= nullable;
        return;
    }
    for arg in &mut ty.args {
        subst_type(arg, subst);
    }
    for item in &mut ty.tuple {
        subst_type(item, subst);
    }
    if let Some(sig) = &mut ty.fn_sig {
        for param in &mut sig.params {
            subst_type(param, subst);
        }
        if let Some(ret) = &mut sig.ret {
            subst_type(ret, subst);
        }
    }
}

pub fn expand(decls: &mut Vec<Spanned<Decl>>, diags: &mut Vec<Diagnostic>) {
    let mut additions = Vec::new();
    for decl in decls.iter() {
        let Decl::Fn(f) = &decl.node else {
            continue;
        };
        if f.type_params.is_empty() {
            continue;
        }
        let concrete = attr_types(&f.attrs);
        if concrete.is_empty() {
            continue;
        }
        if concrete.len() % f.type_params.len() != 0 {
            diags.push(
                Diagnostic::new(
                    Code::E108,
                    format!(
                        "`{}` has {} type parameters but Monomorph lists {} concrete types",
                        f.name,
                        f.type_params.len(),
                        concrete.len()
                    ),
                )
                .with_span(decl.span),
            );
            continue;
        }
        for types in concrete.chunks(f.type_params.len()) {
            let mut subst = BTreeMap::new();
            for (param, ty) in f.type_params.iter().zip(types.iter()) {
                subst.insert(param.clone(), Type::named(ty));
            }
            let mut specialized = f.clone();
            specialized.name = format!(
                "{}{}",
                f.name,
                types.iter().map(|ty| type_suffix(ty)).collect::<String>()
            );
            specialized.type_params.clear();
            specialized.attrs.retain(|attr| attr.name != "Monomorph");
            for param in &mut specialized.params {
                if let Some(ty) = &mut param.ty {
                    subst_type(ty, &subst);
                }
            }
            if let Some(ret) = &mut specialized.ret {
                subst_type(ret, &subst);
            }
            additions.push(Spanned { node: Decl::Fn(specialized), span: decl.span });
        }
    }
    let existing: std::collections::BTreeSet<String> = decls
        .iter()
        .filter_map(|decl| match &decl.node {
            Decl::Fn(f) => Some(f.name.clone()),
            _ => None,
        })
        .collect();
    for addition in additions {
        let Decl::Fn(f) = &addition.node else {
            continue;
        };
        if existing.contains(&f.name) {
            diags.push(
                Diagnostic::new(
                    Code::E108,
                    format!("monomorphization would define duplicate function `{}`", f.name),
                )
                .with_span(addition.span),
            );
            continue;
        }
        decls.push(addition);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::Parser;

    #[test]
    fn specializes_a_generic_function_with_concrete_signatures() {
        let module = Parser::parse_module(
            "#[Monomorph(Int, Float)]\nfn scale<T>(value: T, by: T): T { return value * by }\nfn Main(): Int { return 0 }\n",
        )
        .unwrap();
        let mut decls = module.decls;
        let mut diags = Vec::new();
        expand(&mut decls, &mut diags);
        assert!(diags.is_empty(), "{diags:?}");
        let names: Vec<&str> = decls
            .iter()
            .filter_map(|decl| match &decl.node {
                Decl::Fn(f) => Some(f.name.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(names, vec!["scale", "Main", "scaleI32", "scaleF64"]);
        let specialized = decls
            .iter()
            .find_map(|decl| match &decl.node {
                Decl::Fn(f) if f.name == "scaleF64" => Some(f),
                _ => None,
            })
            .unwrap();
        assert!(specialized.type_params.is_empty());
        assert_eq!(specialized.params[0].ty.as_ref().unwrap().path, vec!["Float"]);
        assert_eq!(specialized.ret.as_ref().unwrap().path, vec!["Float"]);
        assert!(specialized.attrs.iter().all(|attr| attr.name != "Monomorph"));
    }

    #[test]
    fn rejects_a_mismatched_monomorph_list() {
        let module =
            Parser::parse_module("#[Monomorph(Int, Float, Bool)]\nfn pair<T, U>(a: T, b: U): T { return a }\n")
                .unwrap();
        let mut decls = module.decls;
        let mut diags = Vec::new();
        expand(&mut decls, &mut diags);
        assert!(!decls.iter().any(|decl| matches!(&decl.node, Decl::Fn(f) if f.name.contains("I32"))));
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("2 type parameters"));
    }

    #[test]
    fn type_suffix_matches_language_types() {
        assert_eq!(type_suffix("Int"), "I32");
        assert_eq!(type_suffix("Float"), "F64");
        assert_eq!(type_suffix("Bool"), "Bool");
    }
}
