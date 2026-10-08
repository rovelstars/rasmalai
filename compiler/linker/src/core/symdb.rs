use crate::LinkError;
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct Def {
    pub obj: usize,
    pub sec: u32,
    pub value: u64,
    pub size: u64,
    pub bind: u8,
    pub kind: u8,
    pub vis: u8,
    pub tls: bool,
    pub ifunc: bool,
    pub absolute: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Entry {
    pub def: Option<Def>,
    pub strong: bool,
    pub common_size: u64,
    pub common_align: u64,
    pub refs: Vec<(usize, bool)>,
    pub hidden_ref: bool,
    pub ref_kind: u8,
    pub ref_kind_set: bool,
    pub ref_strong: bool,
    pub ref_default_vis: bool,
}

pub struct SymDb {
    map: BTreeMap<String, Entry>,
}

impl SymDb {
    pub fn new() -> SymDb {
        SymDb { map: BTreeMap::new() }
    }

    pub fn define(&mut self, name: &str, def: Def, obj_label: &str, prev_label: &dyn Fn(usize) -> String) -> Result<(), LinkError> {
        let strong = def.bind == crate::core::obj::STB_GLOBAL;
        let e = self.map.entry(name.to_string()).or_default();
        if let Some(old) = e.def.as_ref() {
            if strong && e.strong {
                return Err(LinkError::Native(format!(
                    "duplicate symbol `{name}` in {obj_label} and {}",
                    prev_label(old.obj)
                )));
            }
            if strong || !e.strong {
                e.def = Some(def);
                e.strong = strong;
            }
            return Ok(());
        }
        if e.common_size > 0 && strong {
            e.common_size = 0;
            e.common_align = 0;
        }
        e.def = Some(def);
        e.strong = strong;
        Ok(())
    }

    pub fn define_common(&mut self, name: &str, size: u64, align: u64) {
        let e = self.map.entry(name.to_string()).or_default();
        if e.def.as_ref().is_some_and(|d| d.bind == crate::core::obj::STB_GLOBAL) {
            return;
        }
        if size > e.common_size {
            e.common_size = size;
        }
        if align > e.common_align {
            e.common_align = align;
        }
    }

    pub fn note_ref(&mut self, name: &str, obj: usize, strong: bool, hidden: bool, kind: u8) {
        let e = self.map.entry(name.to_string()).or_default();
        match e.refs.iter_mut().find(|slot| slot.0 == obj) {
            Some(slot) => slot.1 |= strong,
            None => e.refs.push((obj, strong)),
        }
        e.hidden_ref |= hidden;
        e.ref_strong |= strong;
        e.ref_default_vis |= !hidden;
        if !e.ref_kind_set {
            e.ref_kind = kind;
            e.ref_kind_set = true;
        }
    }

    pub fn effective_vis(&self, name: &str) -> u8 {
        match self.map.get(name) {
            Some(e) => {
                let def_hidden = e.def.as_ref().is_some_and(|d| d.vis == crate::core::obj::STV_HIDDEN);
                if def_hidden || e.hidden_ref {
                    crate::core::obj::STV_HIDDEN
                } else {
                    0
                }
            }
            None => 0,
        }
    }

    pub fn get(&self, name: &str) -> Option<&Entry> {
        self.map.get(name)
    }

    pub fn is_unresolved(&self, name: &str) -> bool {
        match self.map.get(name) {
            Some(e) => e.def.is_none() && e.common_size == 0 && e.refs.iter().any(|&(_, s)| s),
            None => false,
        }
    }

    pub fn unresolved(&self) -> Vec<(String, Vec<usize>)> {
        let mut out = Vec::new();
        for (name, e) in self.map.iter() {
            if e.def.is_none() && e.common_size == 0 && e.refs.iter().any(|&(_, s)| s) {
                let mut objs: Vec<usize> = e.refs.iter().filter(|&&(_, s)| s).map(|&(o, _)| o).collect();
                objs.sort();
                objs.dedup();
                out.push((name.clone(), objs));
            }
        }
        out
    }

    pub fn names(&self) -> impl Iterator<Item = &String> {
        self.map.keys()
    }

    pub fn entries(&self) -> impl Iterator<Item = (&String, &Entry)> {
        self.map.iter()
    }
}
