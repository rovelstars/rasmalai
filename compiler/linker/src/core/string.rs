use std::collections::BTreeMap;

pub struct StrTab {
    bytes: Vec<u8>,
    index: BTreeMap<String, u32>,
}

impl StrTab {
    pub fn new() -> StrTab {
        let mut tab = StrTab { bytes: Vec::new(), index: BTreeMap::new() };
        tab.bytes.push(0);
        tab
    }

    pub fn add(&mut self, s: &str) -> u32 {
        if let Some(&off) = self.index.get(s) {
            return off;
        }
        let off = self.bytes.len() as u32;
        self.bytes.extend_from_slice(s.as_bytes());
        self.bytes.push(0);
        self.index.insert(s.to_string(), off);
        off
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn len(&self) -> usize {
        self.bytes.len()
    }
}
