use diagnostics::{decl_file, DeclFiles};
use frontend::ast as A;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::collections::BTreeMap;

pub enum Delta {
    Pure { swapped: Vec<String> },
    Structural { name: String },
}

type FnSigKey = (Vec<Option<A::Type>>, Option<A::Type>, bool);

fn fn_sig_key(f: &A::FnDecl) -> FnSigKey {
    let params = f.params.iter().map(|p| p.ty.clone()).collect();
    (params, f.ret.clone(), f.throws)
}

#[derive(PartialEq)]
struct TypeShape {
    fields: Vec<(String, Option<A::Type>)>,
    methods: Vec<(String, FnSigKey)>,
    variants: Vec<(String, Vec<A::Type>)>,
}

fn member_shapes(members: &[A::Spanned<A::ClassMember>]) -> (Vec<(String, Option<A::Type>)>, Vec<(String, FnSigKey)>) {
    let mut fields = Vec::new();
    let mut methods = Vec::new();
    for m in members {
        match &m.node {
            A::ClassMember::Field(f) => fields.push((f.name.clone(), f.ty.clone())),
            A::ClassMember::Method(f) => methods.push((f.name.clone(), fn_sig_key(f))),
            A::ClassMember::Init { params, .. } => {
                methods.push(("init".to_string(), (params.iter().map(|p| p.ty.clone()).collect(), None, false)));
            }
            A::ClassMember::Deinit(_) => methods.push(("deinit".to_string(), (Vec::new(), None, false))),
            A::ClassMember::OnReload { params, .. } => {
                methods.push(("onReload".to_string(), (params.iter().map(|p| p.ty.clone()).collect(), None, false)));
            }
        }
    }
    (fields, methods)
}

struct Tables<'a> {
    fns: BTreeMap<String, &'a A::FnDecl>,
    types: BTreeMap<String, TypeShape>,
    consts: BTreeMap<String, Option<A::Type>>,
    bodies: BTreeMap<String, String>,
}

fn collect<'a>(m: &'a A::Module, files: Option<&DeclFiles>, changed: Option<&BTreeSet<PathBuf>>) -> Tables<'a> {
    let include = |span: diagnostics::Span| -> bool {
        match (files, changed) {
            (Some(files), Some(changed)) => decl_file(files, span)
                .map(|path| changed.contains(&path))
                .unwrap_or(true),
            _ => true,
        }
    };
    let mut t = Tables {
        fns: BTreeMap::new(),
        types: BTreeMap::new(),
        consts: BTreeMap::new(),
        bodies: BTreeMap::new(),
    };
    for d in &m.decls {
        match &d.node {
            A::Decl::Fn(f) => {
                if include(d.span) {
                    t.fns.insert(f.name.clone(), f);
                    t.bodies.insert(f.name.clone(), format!("{:?}", f.body));
                }
            }
            A::Decl::Class { name, members, .. }
            | A::Decl::Struct { name, members, .. }
            | A::Decl::Trait { name, members, .. }
            | A::Decl::Interface { name, members, .. } => {
                if !include(d.span) {
                    continue;
                }
                let (fields, methods) = member_shapes(members);
                t.types.insert(name.clone(), TypeShape { fields, methods, variants: Vec::new() });
                for member in members {
                    if !include(member.span) {
                        continue;
                    }
                    match &member.node {
                        A::ClassMember::Method(f) => {
                            t.bodies.insert(format!("{name}.{}", f.name), format!("{:?}", f.body));
                        }
                        A::ClassMember::Init { body, .. } => {
                            t.bodies.insert(format!("{name}.init"), format!("{:?}", body));
                        }
                        A::ClassMember::Deinit(body) => {
                            t.bodies.insert(format!("{name}.deinit"), format!("{:?}", body));
                        }
                        A::ClassMember::OnReload { body, .. } => {
                            t.bodies.insert(format!("{name}.onReload"), format!("{:?}", body));
                        }
                        _ => {}
                    }
                }
            }
            A::Decl::Enum { name, members, .. } => {
                if !include(d.span) {
                    continue;
                }
                let variants = members.iter().map(|e| (e.name.clone(), e.payload.clone())).collect();
                t.types.insert(name.clone(), TypeShape { fields: Vec::new(), methods: Vec::new(), variants });
            }
            A::Decl::Record { name, fields, .. } => {
                if !include(d.span) {
                    continue;
                }
                let fields = fields.iter().map(|f| (f.name.clone(), Some(f.ty.clone()))).collect();
                t.types.insert(name.clone(), TypeShape { fields, methods: Vec::new(), variants: Vec::new() });
            }
            A::Decl::Extension { target, members, .. } => {
                if !include(d.span) {
                    continue;
                }
                let (fields, methods) = member_shapes(members);
                let key = format!("extension:{target:?}");
                t.types.insert(key, TypeShape { fields, methods, variants: Vec::new() });
            }
            A::Decl::Const { name, ty, .. } => {
                if include(d.span) {
                    t.consts.insert(name.clone(), ty.clone());
                }
            }
            _ => {}
        }
    }
    t
}

fn first_diff<V: PartialEq>(a: &BTreeMap<String, V>, b: &BTreeMap<String, V>, what: &str) -> Option<String> {
    for (k, v) in a {
        match b.get(k) {
            Some(w) if w == v => {}
            Some(_) => return Some(format!("{what} `{k}` changed")),
            None => return Some(format!("{what} `{k}` removed")),
        }
    }
    for k in b.keys() {
        if !a.contains_key(k) {
            return Some(format!("{what} `{k}` added"));
        }
    }
    None
}

pub fn classify(
    old: &A::Module,
    new: &A::Module,
    files: Option<&DeclFiles>,
    changed: Option<&BTreeSet<PathBuf>>,
) -> Delta {
    let a = collect(old, files, changed);
    let b = collect(new, files, changed);
    if let Some(name) = first_diff(&a.types, &b.types, "type") {
        return Delta::Structural { name };
    }
    if let Some(name) = first_diff(&a.consts, &b.consts, "const") {
        return Delta::Structural { name };
    }
    if let Some(name) = first_diff_by(&a.fns, &b.fns, |f, g| fn_sig_key(f) == fn_sig_key(g), "fn") {
        return Delta::Structural { name };
    }
    let mut swapped = Vec::new();
    for (name, body) in &a.bodies {
        if b.bodies.get(name).is_some_and(|new| new != body) {
            swapped.push(name.clone());
        }
    }
    Delta::Pure { swapped }
}

fn first_diff_by<T>(a: &BTreeMap<String, T>, b: &BTreeMap<String, T>, eq: impl Fn(&T, &T) -> bool, what: &str) -> Option<String> {
    for (k, v) in a {
        match b.get(k) {
            Some(w) if eq(v, w) => {}
            Some(_) => return Some(format!("signature of {what} `{k}`")),
            None => return Some(format!("removed {what} `{k}`")),
        }
    }
    for k in b.keys() {
        if !a.contains_key(k) {
            return Some(format!("added {what} `{k}`"));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use frontend::parser::Parser;

    fn parse(src: &str) -> A::Module {
        Parser::parse_module(src).expect("test source parses")
    }

    #[test]
    fn body_only_edits_are_pure() {
        let old = parse("fn Main(): Int { return 1; }");
        let new = parse("fn Main(): Int { return 2; }");
        match classify(&old, &new, None, None) {
            Delta::Pure { swapped } => assert_eq!(swapped, vec!["Main".to_string()]),
            Delta::Structural { name } => panic!("expected a body swap, got {name}"),
        }
    }

    #[test]
    fn method_body_edits_are_pure_and_qualified() {
        let old = parse("class C { fn run(): Int { return 1; } }\nfn Main(): Int { return 1; }");
        let new = parse("class C { fn run(): Int { return 2; } }\nfn Main(): Int { return 1; }");
        match classify(&old, &new, None, None) {
            Delta::Pure { swapped } => assert_eq!(swapped, vec!["C.run".to_string()]),
            Delta::Structural { name } => panic!("expected a method swap, got {name}"),
        }
    }

    #[test]
    fn unchanged_modules_swap_nothing() {
        let module = parse("fn Main(): Int { return 1; }");
        match classify(&module, &module.clone(), None, None) {
            Delta::Pure { swapped } => assert!(swapped.is_empty(), "{swapped:?}"),
            Delta::Structural { name } => panic!("expected no change, got {name}"),
        }
    }

    #[test]
    fn added_functions_require_a_restart() {
        let old = parse("fn Main(): Int { return 1; }");
        let new = parse("fn Main(): Int { return 1; }\nfn Extra(): Int { return 2; }");
        match classify(&old, &new, None, None) {
            Delta::Structural { name } => assert!(name.contains("Extra"), "{name}"),
            Delta::Pure { swapped } => panic!("expected a restart, got {swapped:?}"),
        }
    }

    #[test]
    fn type_shape_changes_require_a_restart() {
        let old = parse("class C { let x: Int }\nfn Main(): Int { return 1; }");
        let new = parse("class C { let x: Bool }\nfn Main(): Int { return 1; }");
        match classify(&old, &new, None, None) {
            Delta::Structural { name } => assert!(name.contains("type"), "{name}"),
            Delta::Pure { swapped } => panic!("expected a restart, got {swapped:?}"),
        }
    }
}
