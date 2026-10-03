use frontend::ast as A;
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
}

fn collect(m: &A::Module) -> Tables<'_> {
    let mut t = Tables { fns: BTreeMap::new(), types: BTreeMap::new(), consts: BTreeMap::new() };
    for d in &m.decls {
        match &d.node {
            A::Decl::Fn(f) => {
                t.fns.insert(f.name.clone(), f);
            }
            A::Decl::Class { name, members, .. }
            | A::Decl::Struct { name, members, .. }
            | A::Decl::Trait { name, members, .. }
            | A::Decl::Interface { name, members, .. } => {
                let (fields, methods) = member_shapes(members);
                t.types.insert(name.clone(), TypeShape { fields, methods, variants: Vec::new() });
            }
            A::Decl::Enum { name, members, .. } => {
                let variants = members.iter().map(|e| (e.name.clone(), e.payload.clone())).collect();
                t.types.insert(name.clone(), TypeShape { fields: Vec::new(), methods: Vec::new(), variants });
            }
            A::Decl::Record { name, fields, .. } => {
                let fields = fields.iter().map(|f| (f.name.clone(), Some(f.ty.clone()))).collect();
                t.types.insert(name.clone(), TypeShape { fields, methods: Vec::new(), variants: Vec::new() });
            }
            A::Decl::Extension { target, members, .. } => {
                let (fields, methods) = member_shapes(members);
                let key = format!("extension:{target:?}");
                t.types.insert(key, TypeShape { fields, methods, variants: Vec::new() });
            }
            A::Decl::Const { name, ty, .. } => {
                t.consts.insert(name.clone(), ty.clone());
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

pub fn classify(old: &A::Module, new: &A::Module) -> Delta {
    let a = collect(old);
    let b = collect(new);
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
    for (name, f_old) in &a.fns {
        let f_new = b.fns[name.as_str()];
        if format!("{:?}", f_old.body) != format!("{:?}", f_new.body) {
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
