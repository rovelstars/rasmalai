use crate::instr::*;
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShakeStats {
    pub kept: usize,
    pub dropped: usize,
}

pub fn reachable_set(module: &Module, entry: &str, _is_library: bool) -> BTreeSet<usize> {
    let mut reach = Reach::new(module);
    reach.roots(entry);
    if reach.reachable.is_empty() {
        return (0..module.functions.len()).collect();
    }
    reach.closure();
    reach.reachable
}

pub fn shake_module(module: &mut Module, entry: &str, is_library: bool) -> ShakeStats {
    let reachable = reachable_set(module, entry, is_library);
    if reachable.len() == module.functions.len() {
        return ShakeStats { kept: reachable.len(), dropped: 0 };
    }
    let mut map = vec![usize::MAX; module.functions.len()];
    let mut kept: Vec<Function> = Vec::new();
    for (i, func) in module.functions.drain(..).enumerate() {
        if reachable.contains(&i) {
            map[i] = kept.len();
            kept.push(func);
        }
    }
    let dropped = map.iter().filter(|m| **m == usize::MAX).count();
    module.functions = kept;
    for func in &mut module.functions {
        for block in &mut func.blocks {
            for ins in &mut block.instrs {
                remap_targets(ins, &map);
            }
        }
    }
    for class in &mut module.classes {
        class.methods.retain(|_, m| map.get(m.id).copied().unwrap_or(usize::MAX) != usize::MAX);
        for method in class.methods.values_mut() {
            if let Some(mapped) = map.get(method.id).copied() {
                method.id = mapped;
            }
        }
        for slot in [&mut class.deinit, &mut class.dtor] {
            *slot = slot.and_then(|id| {
                let mapped = map.get(id).copied().unwrap_or(usize::MAX);
                (mapped != usize::MAX).then_some(mapped)
            });
        }
    }
    for enu in &mut module.enums {
        enu.dtor = enu.dtor.and_then(|id| {
            let mapped = map.get(id).copied().unwrap_or(usize::MAX);
            (mapped != usize::MAX).then_some(mapped)
        });
    }
    module.array_dtors.retain(|_, id| map.get(*id).copied().unwrap_or(usize::MAX) != usize::MAX);
    for id in module.array_dtors.values_mut() {
        if let Some(mapped) = map.get(*id).copied() {
            *id = mapped;
        }
    }
    module.fn_index.clear();
    for (i, func) in module.functions.iter().enumerate() {
        module.fn_index.insert(func.name.clone(), i);
    }
    ShakeStats { kept: module.functions.len(), dropped }
}

struct Reach<'a> {
    module: &'a Module,
    reachable: BTreeSet<usize>,
    worklist: Vec<usize>,
    scanned_fns: BTreeSet<usize>,
    scanned_classes: BTreeSet<usize>,
    scanned_enums: BTreeSet<usize>,
    ref_classes: BTreeSet<usize>,
    ref_enums: BTreeSet<usize>,
    ref_arrays: BTreeSet<String>,
    dyn_names: BTreeSet<String>,
    unknown_dyn: BTreeSet<String>,
    address_taken: BTreeSet<usize>,
    saw_value_call: bool,
}

fn collect_address_taken(module: &Module) -> BTreeSet<usize> {
    let mut taken = BTreeSet::new();
    for (i, f) in module.functions.iter().enumerate() {
        if f.is_closure {
            taken.insert(i);
        }
        for block in &f.blocks {
            walk_instrs(&block.instrs, &mut |ins| {
                if let Instr::ClosureNew { func, .. } = ins {
                    taken.insert(*func);
                }
            });
        }
    }
    taken.retain(|id| *id < module.functions.len());
    taken
}

impl<'a> Reach<'a> {
    fn new(module: &'a Module) -> Self {
        Reach {
            module,
            reachable: BTreeSet::new(),
            worklist: Vec::new(),
            scanned_fns: BTreeSet::new(),
            scanned_classes: BTreeSet::new(),
            scanned_enums: BTreeSet::new(),
            ref_classes: BTreeSet::new(),
            ref_enums: BTreeSet::new(),
            ref_arrays: BTreeSet::new(),
            dyn_names: BTreeSet::new(),
            unknown_dyn: BTreeSet::new(),
            address_taken: collect_address_taken(module),
            saw_value_call: false,
        }
    }

    fn mark(&mut self, id: usize) -> bool {
        if id < self.module.functions.len() && self.reachable.insert(id) {
            self.worklist.push(id);
            return true;
        }
        false
    }

    fn roots(&mut self, entry: &str) {
        if let Some(id) = self.module.fn_id(entry) {
            self.mark(id);
        }
        for (i, f) in self.module.functions.iter().enumerate() {
            if f.is_pub {
                self.mark(i);
            }
        }
        for ci in 0..self.module.classes.len() {
            let ifaces = match self.module.classes.get(ci) {
                Some(c) => c.ifaces.clone(),
                None => continue,
            };
            for ii in ifaces {
                let names: Vec<String> = match self.module.interfaces.get(ii) {
                    Some(iface) => iface.methods.iter().map(|m| m.name.clone()).collect(),
                    None => continue,
                };
                for name in names {
                    if let Some(id) = self.iface_impl(ci, &name) {
                        self.mark(id);
                    }
                }
            }
        }
    }

    fn iface_impl(&self, ci: usize, name: &str) -> Option<usize> {
        let mut cur = Some(ci);
        while let Some(c) = cur {
            let class = self.module.classes.get(c)?;
            if let Some(mr) = class.methods.get(name) {
                return Some(mr.id);
            }
            cur = class.parent;
        }
        None
    }

    fn closure(&mut self) {
        loop {
            let mut progress = false;
            while let Some(fi) = self.worklist.pop() {
                let blocks = match self.module.functions.get(fi) {
                    Some(f) => &f.blocks,
                    None => continue,
                };
                for block in blocks {
                    for ins in &block.instrs {
                        progress |= self.mark_ins(ins);
                    }
                }
            }
            let pending: Vec<usize> = self.reachable.difference(&self.scanned_fns).copied().collect();
            for fi in pending {
                self.scanned_fns.insert(fi);
                self.scan_fn(fi);
            }
            progress |= self.expand_types();
            // Value calls dispatch only over closure bodies, and ClosureNew
            // is the single IR point that materializes a function reference
            // into a value (there are no fn-pointer constants), so a reachable
            // indirect call keeps the address-taken set, not the whole module.
            if self.saw_value_call {
                let taken: Vec<usize> = self.address_taken.iter().copied().collect();
                for id in taken {
                    progress |= self.mark(id);
                }
            }
            // An unknown-Dyn receiver (Any-typed) can only hold an instance of
            // a class the reachable code actually references, so candidates
            // are same-named methods of retained classes, not every class.
            let unknown: Vec<String> = self.unknown_dyn.iter().cloned().collect();
            let live: Vec<usize> = self.ref_classes.iter().copied().collect();
            for name in &unknown {
                for ci in &live {
                    if let Some(mr) = self.module.classes.get(*ci).and_then(|c| c.methods.get(name)) {
                        progress |= self.mark(mr.id);
                    }
                }
            }
            if !progress {
                break;
            }
        }
    }

    fn mark_ins(&mut self, ins: &Instr) -> bool {
        let mut progress = false;
        match ins {
            Instr::Call { target, .. } => match target {
                CallTarget::Fn(id) => progress |= self.mark(*id),
                CallTarget::Method { method, .. } => progress |= self.mark(*method),
                _ => {}
            },
            Instr::ThreadSpawn { func, .. } => progress |= self.mark(*func),
            Instr::PoolSubmit { func, .. } | Instr::PoolParallelFor { func, .. } => {
                progress |= self.mark(*func);
            }
            Instr::ClosureNew { func, .. } => progress |= self.mark(*func),
            Instr::Defer { body, .. } => {
                for nested in body {
                    progress |= self.mark_ins(nested);
                }
            }
            _ => {}
        }
        progress
    }

    fn scan_fn(&mut self, fi: usize) {
        let f = match self.module.functions.get(fi) {
            Some(f) => f,
            None => return,
        };
        for t in f.params.iter().chain(&f.sig_params).chain(std::iter::once(&f.ret)).chain(&f.locals) {
            collect_type_refs(self.module, t, &mut self.ref_classes, &mut self.ref_enums, &mut self.ref_arrays);
        }
        for block in &f.blocks {
            for ins in &block.instrs {
                self.scan_ins(f, ins);
            }
            if let Terminator::Switch { cases, .. } = &block.term {
                for (pat, _) in cases {
                    match pat {
                        SwitchPat::Enum { enu, .. } => {
                            if *enu < self.module.enums.len() {
                                self.ref_enums.insert(*enu);
                            }
                        }
                        SwitchPat::Is { tag, .. } => {
                            if let Some(&ci) = self.module.class_index.get(tag) {
                                self.ref_classes.insert(ci);
                            } else if let Some(&ii) = self.module.interface_index.get(tag) {
                                for ci in 0..self.module.classes.len() {
                                    let implements = self
                                        .module
                                        .classes
                                        .get(ci)
                                        .is_some_and(|c| c.ifaces.contains(&ii));
                                    if implements {
                                        self.ref_classes.insert(ci);
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    fn scan_ins(&mut self, f: &Function, ins: &Instr) {
        match ins {
            Instr::ObjNew { class, .. } | Instr::StackAlloc { class, .. } => {
                if *class < self.module.classes.len() {
                    self.ref_classes.insert(*class);
                }
            }
            Instr::EnumNew { enu, .. } => {
                if *enu < self.module.enums.len() {
                    self.ref_enums.insert(*enu);
                }
            }
            Instr::Call { target: CallTarget::Method { class, .. }, .. } => {
                if *class < self.module.classes.len() {
                    self.ref_classes.insert(*class);
                }
            }
            Instr::Call { target: CallTarget::Dyn { obj, method }, .. } => {
                self.dyn_names.insert(method.clone());
                match f.locals.get(*obj as usize) {
                    Some(LirType::Obj(n)) => match self.module.class_index.get(n) {
                        Some(&ci) => {
                            self.ref_classes.insert(ci);
                        }
                        None => {
                            self.unknown_dyn.insert(method.clone());
                        }
                    },
                    _ => {
                        self.unknown_dyn.insert(method.clone());
                    }
                }
            }
            Instr::Call { target: CallTarget::Value(_), .. } => {
                self.saw_value_call = true;
            }
            Instr::Defer { body, .. } => {
                for nested in body {
                    self.scan_ins(f, nested);
                }
            }
            _ => {}
        }
    }

    fn expand_types(&mut self) -> bool {
        let mut progress = false;
        let pending_classes: Vec<usize> = self.ref_classes.difference(&self.scanned_classes).copied().collect();
        for ci in pending_classes {
            self.scanned_classes.insert(ci);
            let class = match self.module.classes.get(ci) {
                Some(c) => c,
                None => continue,
            };
            if let Some(parent) = class.parent {
                if parent < self.module.classes.len() {
                    self.ref_classes.insert(parent);
                }
            }
            for fld in &class.fields {
                collect_type_refs(self.module, &fld.ty, &mut self.ref_classes, &mut self.ref_enums, &mut self.ref_arrays);
            }
        }
        let pending_enums: Vec<usize> = self.ref_enums.difference(&self.scanned_enums).copied().collect();
        for ei in pending_enums {
            self.scanned_enums.insert(ei);
            let enu = match self.module.enums.get(ei) {
                Some(e) => e,
                None => continue,
            };
            for v in &enu.variants {
                for t in &v.payload {
                    collect_type_refs(self.module, t, &mut self.ref_classes, &mut self.ref_enums, &mut self.ref_arrays);
                }
            }
        }
        let classes: Vec<usize> = self.ref_classes.iter().copied().collect();
        let dyn_names: Vec<String> = self.dyn_names.iter().cloned().collect();
        for ci in classes {
            let class = match self.module.classes.get(ci) {
                Some(c) => c,
                None => continue,
            };
            if let Some(id) = class.deinit {
                progress |= self.mark(id);
            }
            if let Some(id) = class.dtor {
                progress |= self.mark(id);
            }
            for m in &dyn_names {
                if let Some(mr) = class.methods.get(m) {
                    progress |= self.mark(mr.id);
                }
            }
        }
        let enums: Vec<usize> = self.ref_enums.iter().copied().collect();
        for ei in enums {
            if let Some(id) = self.module.enums.get(ei).and_then(|e| e.dtor) {
                progress |= self.mark(id);
            }
        }
        let arrays: Vec<String> = self.ref_arrays.iter().cloned().collect();
        for key in &arrays {
            if let Some(&id) = self.module.array_dtors.get(key) {
                progress |= self.mark(id);
            }
        }
        progress
    }
}

fn collect_type_refs(
    module: &Module,
    ty: &LirType,
    classes: &mut BTreeSet<usize>,
    enums: &mut BTreeSet<usize>,
    arrays: &mut BTreeSet<String>,
) {
    match ty {
        LirType::Obj(n) => {
            if let Some(&ci) = module.class_index.get(n) {
                classes.insert(ci);
            }
        }
        LirType::Enum(ei) => {
            if *ei < module.enums.len() {
                enums.insert(*ei);
            }
        }
        LirType::Array(inner) => {
            arrays.insert(type_key(inner));
            collect_type_refs(module, inner, classes, enums, arrays);
        }
        LirType::Tuple(items) => {
            for t in items {
                collect_type_refs(module, t, classes, enums, arrays);
            }
        }
        LirType::GenRef(Some(n)) => {
            if let Some(&ci) = module.class_index.get(n) {
                classes.insert(ci);
            }
        }
        _ => {}
    }
}

fn remap_targets(ins: &mut Instr, map: &[usize]) {
    let get = |id: usize| map.get(id).copied().unwrap_or(usize::MAX);
    match ins {
        Instr::Call { target, .. } => match target {
            CallTarget::Fn(id) => *id = get(*id),
            CallTarget::Method { method, .. } => *method = get(*method),
            _ => {}
        },
        Instr::ThreadSpawn { func, .. } => {
            if *func != usize::MAX {
                *func = get(*func);
            }
        }
        Instr::PoolSubmit { func, .. } | Instr::PoolParallelFor { func, .. } => {
            if *func != usize::MAX {
                *func = get(*func);
            }
        }
        Instr::ClosureNew { func, .. } => *func = get(*func),
        Instr::Defer { body, .. } => {
            for nested in body {
                remap_targets(nested, map);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn func(name: &str, calls: &[usize]) -> Function {
        let instrs: Vec<Instr> = calls
            .iter()
            .map(|id| Instr::Call { span: UNKNOWN_SPAN,
                dsts: vec![],
                err: None,
                target: CallTarget::Fn(*id),
                args: Vec::new(),
            })
            .collect();
        Function {
            name: name.to_string(),
            params: Vec::new(),
            sig_params: Vec::new(),
            ret: LirType::I64,
            throws: false,
            is_unsafe: false,
            method_self: false,
            is_pub: false,
            is_closure: false,
            locals: Vec::new(),
            blocks: vec![Block { instrs, term: Terminator::Ret(vec![]) }],
        }
    }

    fn indexed(functions: Vec<Function>) -> Module {
        let mut module = Module { functions, ..Default::default() };
        module.fn_index.clear();
        for (i, f) in module.functions.iter().enumerate() {
            module.fn_index.insert(f.name.clone(), i);
        }
        module
    }

    fn names(module: &Module) -> Vec<String> {
        module.functions.iter().map(|f| f.name.clone()).collect()
    }

    #[test]
    fn drops_unreachable_fn_keeps_transitive() {
        let mut module = indexed(vec![
            func("Main", &[1]),
            func("A", &[2]),
            func("B", &[]),
            func("orphan", &[]),
        ]);
        let stats = shake_module(&mut module, "Main", false);
        assert_eq!(names(&module), vec!["Main", "A", "B"]);
        assert_eq!((stats.kept, stats.dropped), (3, 1));
        assert_eq!(module.fn_index.get("B"), Some(&2));
    }

    #[test]
    fn keeps_pub_export_in_app_mode() {
        let mut module = indexed(vec![func("Main", &[]), func("helper", &[])]);
        module.functions[1].is_pub = true;
        shake_module(&mut module, "Main", false);
        assert_eq!(names(&module), vec!["Main", "helper"]);
    }

    fn iface_module() -> Module {
        let mut module = indexed(vec![
            func("Main", &[]),
            func("K.render", &[]),
            func("K.other", &[]),
        ]);
        module.interfaces.push(InterfaceDesc {
            name: "Drawable".to_string(),
            type_params: Vec::new(),
            methods: vec![IfaceMethod { name: "render".to_string(), params: Vec::new(), ret: LirType::I64 }],
            method_index: BTreeMap::from([("render".to_string(), 0)]),
        });
        module.interface_index.insert("Drawable".to_string(), 0);
        module.class_index.insert("K".to_string(), 0);
        module.classes.push(ClassDesc {
            name: "K".to_string(),
            type_params: Vec::new(),
            is_struct: false,
            fields: Vec::new(),
            field_index: BTreeMap::new(),
            methods: BTreeMap::from([
                ("render".to_string(), MethodRef { id: 1, private: false, owner: "K".to_string() }),
                ("other".to_string(), MethodRef { id: 2, private: false, owner: "K".to_string() }),
            ]),
            deinit: None,
            dtor: None,
            ifaces: vec![0],
            parent: None,
        });
        module
    }

    #[test]
    fn keeps_iface_impl_without_direct_call() {
        let mut module = iface_module();
        shake_module(&mut module, "Main", false);
        assert_eq!(names(&module), vec!["Main", "K.render"]);
        assert_eq!(module.classes[0].methods.len(), 1);
    }

    #[test]
    fn value_call_keeps_only_address_taken() {
        let mut module = indexed(vec![func("Main", &[]), func("cb", &[]), func("orphan", &[])]);
        module.functions[0].blocks[0].instrs.push(Instr::ClosureNew { span: UNKNOWN_SPAN,
            dst: 0,
            func: 1,
            captures: Vec::new(),
            decay: false,
            decay_this: false,
        });
        module.functions[0].blocks[0].instrs.push(Instr::Call { span: UNKNOWN_SPAN,
            dsts: vec![],
            err: None,
            target: CallTarget::Value(0),
            args: Vec::new(),
        });
        let stats = shake_module(&mut module, "Main", false);
        assert_eq!(stats.dropped, 1);
        assert_eq!(names(&module), vec!["Main", "cb"]);
    }

    #[test]
    fn unknown_dyn_receiver_keeps_only_retained_class_methods() {
        let mut main = func("Main", &[]);
        main.locals.push(LirType::Any);
        main.blocks[0].instrs.push(Instr::ObjNew { span: UNKNOWN_SPAN,
            dst: 0,
            class: 0,
            instance_size: 0,
        });
        main.blocks[0].instrs.push(Instr::Call { span: UNKNOWN_SPAN,
            dsts: vec![],
            err: None,
            target: CallTarget::Dyn { obj: 0, method: "render".to_string() },
            args: Vec::new(),
        });
        let mut module = indexed(vec![main, func("K.render", &[]), func("J.render", &[]), func("J.other", &[])]);
        module.class_index.insert("K".to_string(), 0);
        module.class_index.insert("J".to_string(), 1);
        module.classes.push(ClassDesc {
            name: "K".to_string(),
            type_params: Vec::new(),
            is_struct: false,
            fields: Vec::new(),
            field_index: BTreeMap::new(),
            methods: BTreeMap::from([("render".to_string(), MethodRef { id: 1, private: false, owner: "K".to_string() })]),
            deinit: None,
            dtor: None,
            ifaces: Vec::new(),
            parent: None,
        });
        module.classes.push(ClassDesc {
            name: "J".to_string(),
            type_params: Vec::new(),
            is_struct: false,
            fields: Vec::new(),
            field_index: BTreeMap::new(),
            methods: BTreeMap::from([
                ("render".to_string(), MethodRef { id: 2, private: false, owner: "J".to_string() }),
                ("other".to_string(), MethodRef { id: 3, private: false, owner: "J".to_string() }),
            ]),
            deinit: None,
            dtor: None,
            ifaces: Vec::new(),
            parent: None,
        });
        shake_module(&mut module, "Main", false);
        assert_eq!(names(&module), vec!["Main", "K.render"]);
    }
}
