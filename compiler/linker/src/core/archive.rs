use crate::LinkError;
use std::collections::BTreeMap;

pub struct Archive<'a> {
    pub data: &'a [u8],
    pub label: String,
    members: Vec<Member>,
    index: BTreeMap<String, Vec<usize>>,
    sysv: Option<(usize, usize)>,
}

#[derive(Clone, Debug)]
struct Member {
    name: String,
    header: usize,
    off: usize,
    size: usize,
}

fn parse_number(field: &[u8], label: &str) -> Result<usize, LinkError> {
    let text = std::str::from_utf8(field).map_err(|_| LinkError::Native(format!("{label}: bad archive header")))?;
    text.trim().parse::<usize>().map_err(|_| LinkError::Native(format!("{label}: bad archive header")))
}

impl<'a> Archive<'a> {
    pub fn parse(data: &'a [u8], label: &str) -> Result<Archive<'a>, LinkError> {
        let err = |m: &str| LinkError::Native(format!("{label}: {m}"));
        if data.len() < 8 || &data[0..8] != b"!<arch>\n" {
            return Err(err("not a System V archive"));
        }
        let mut off = 8usize;
        let mut strtab: &[u8] = &[];
        let mut members: Vec<Member> = Vec::new();
        let mut sysv: Option<(usize, usize)> = None;
        while off < data.len() {
            if off + 60 > data.len() {
                return Err(err("truncated member header"));
            }
            let hdr = &data[off..off + 60];
            if &hdr[58..60] != b"\x60\x0a" {
                return Err(err("bad member header magic"));
            }
            let size = parse_number(&hdr[48..58], label)?;
            let body_off = off + 60;
            let body_end = body_off.checked_add(size).ok_or_else(|| err("bad member size"))?;
            if body_end > data.len() {
                return Err(err("member out of range"));
            }
            let name_field = std::str::from_utf8(&hdr[0..16]).map_err(|_| err("bad member name"))?;
            let name = name_field.trim().to_string();
            if name == "//" {
                strtab = &data[body_off..body_end];
            } else if name == "/" {
                sysv = Some((body_off, body_end));
            } else if name == "__.SYMDEF" {
            } else {
                let resolved = if let Some(rest) = name.strip_prefix('/') {
                    let idx: usize = rest.trim_end_matches('/').parse().map_err(|_| err("bad long name index"))?;
                    let end = strtab.iter().skip(idx).position(|&b| b == b'\n').map(|p| idx + p).unwrap_or(strtab.len());
                    let entry = strtab.get(idx..end).ok_or_else(|| err("long name out of range"))?;
                    std::str::from_utf8(entry).map_err(|_| err("bad long member name"))?.trim_end_matches('/').to_string()
                } else {
                    name.trim_end_matches('/').to_string()
                };
                members.push(Member { name: resolved, header: off, off: body_off, size });
            }
            off = body_end + (size & 1);
        }
        Ok(Archive { data, label: label.to_string(), members, index: BTreeMap::new(), sysv })
    }

    pub fn member_count(&self) -> usize {
        self.members.len()
    }

    pub fn member_bytes(&self, idx: usize) -> Result<&'a [u8], LinkError> {
        let m = self.members.get(idx).ok_or_else(|| LinkError::Native(format!("{}: bad member index", self.label)))?;
        self.data.get(m.off..m.off + m.size).ok_or_else(|| LinkError::Native(format!("{}: member out of range", self.label)))
    }

    pub fn member_name(&self, idx: usize) -> &str {
        self.members.get(idx).map(|m| m.name.as_str()).unwrap_or("?")
    }

    pub fn build_index(&mut self, symbols_of: &dyn Fn(&[u8], &str) -> Result<Vec<String>, LinkError>) -> Result<(), LinkError> {
        if let Some((start, end)) = self.sysv {
            return self.build_index_sysv(start, end);
        }
        self.build_index_scan(symbols_of)
    }

    fn build_index_sysv(&mut self, start: usize, end: usize) -> Result<(), LinkError> {
        let err = |m: &str| LinkError::Native(format!("{}: {m}", self.label));
        let body = self.data.get(start..end).ok_or_else(|| err("member out of range"))?;
        let at = |o: usize| -> Result<u32, LinkError> {
            body.get(o..o + 4).and_then(|b| b.try_into().ok()).map(u32::from_be_bytes).ok_or_else(|| err("truncated archive index"))
        };
        let count = at(0)? as usize;
        if count > 1_000_000 {
            return Err(err("implausible archive index size"));
        }
        if 4 + count * 4 > body.len() {
            return Err(err("truncated archive index"));
        }
        let mut header_to_member: BTreeMap<usize, usize> = BTreeMap::new();
        for (idx, m) in self.members.iter().enumerate() {
            header_to_member.insert(m.header, idx);
        }
        let mut pos = 4 + count * 4;
        for i in 0..count {
            let header = at(4 + i * 4)? as usize;
            let end = body[pos..].iter().position(|&b| b == 0).map(|p| pos + p).ok_or_else(|| err("unterminated index name"))?;
            let name = std::str::from_utf8(body.get(pos..end).ok_or_else(|| err("index name out of range"))?).map_err(|_| err("non-utf8 name in archive index"))?;
            pos = end + 1;
            if name.is_empty() {
                continue;
            }
            if let Some(&idx) = header_to_member.get(&header) {
                let entry = self.index.entry(name.to_string()).or_default();
                if !entry.contains(&idx) {
                    entry.push(idx);
                }
            }
        }
        Ok(())
    }

    fn build_index_scan(&mut self, symbols_of: &dyn Fn(&[u8], &str) -> Result<Vec<String>, LinkError>) -> Result<(), LinkError> {
        for (idx, m) in self.members.iter().enumerate() {
            let body = self.data.get(m.off..m.off + m.size).ok_or_else(|| LinkError::Native(format!("{}: member out of range", self.label)))?;
            let syms = symbols_of(body, &format!("{}({})", self.label, m.name))?;
            for s in syms {
                self.index.entry(s).or_default().push(idx);
            }
        }
        Ok(())
    }

    pub fn providers(&self, symbol: &str) -> Option<&Vec<usize>> {
        self.index.get(symbol)
    }
}
