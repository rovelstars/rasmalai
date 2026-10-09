use crate::LinkError;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

pub const MANIFEST_NAME: &str = "link-plan.json";
pub const MAGIC: &str = "RNXLNK02";
pub const FORMAT_VERSION: u32 = 3;

pub struct CacheCtx {
    pub dir: PathBuf,
    pub toolchain: String,
    pub allow_write: bool,
}

#[derive(Clone)]
pub struct ArchiveIdent {
    pub tag: String,
    pub path: String,
    pub size: u64,
    pub mtime_secs: u64,
    pub mtime_nanos: u32,
}

#[derive(Clone)]
pub struct MemberKey {
    pub archive: u32,
    pub member: u32,
    pub name: String,
}

pub struct PlanData {
    pub toolchain: String,
    pub opts: String,
    pub archives: Vec<ArchiveIdent>,
    pub members: Vec<MemberKey>,
    pub user_defined: Vec<String>,
    pub user_unresolved: Vec<String>,
}

fn file_stat(path: &Path) -> Option<(u64, u64, u32)> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() {
        return None;
    }
    let size = meta.len();
    let mtime = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?;
    Some((size, mtime.as_secs(), mtime.subsec_nanos()))
}

pub fn stat_ident(tag: &str, path: &Path) -> Option<ArchiveIdent> {
    let (size, secs, nanos) = file_stat(path)?;
    Some(ArchiveIdent {
        tag: tag.to_string(),
        path: path.to_string_lossy().into_owned(),
        size,
        mtime_secs: secs,
        mtime_nanos: nanos,
    })
}

fn esc_json(out: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
}

pub fn write_manifest(dir: &Path, plan: &PlanData) -> Result<(), LinkError> {
    let mut text = String::new();
    text.push_str("{\"magic\":\"");
    text.push_str(MAGIC);
    text.push_str("\",\"version\":");
    text.push_str(&FORMAT_VERSION.to_string());
    text.push_str(",\"toolchain\":\"");
    esc_json(&mut text, &plan.toolchain);
    text.push_str("\",\"opts\":\"");
    esc_json(&mut text, &plan.opts);
    text.push_str("\",\"archives\":[");
    for (i, a) in plan.archives.iter().enumerate() {
        if i > 0 {
            text.push(',');
        }
        text.push_str("{\"tag\":\"");
        esc_json(&mut text, &a.tag);
        text.push_str("\",\"path\":\"");
        esc_json(&mut text, &a.path);
        text.push_str("\",\"size\":");
        text.push_str(&a.size.to_string());
        text.push_str(",\"mtime\":[");
        text.push_str(&a.mtime_secs.to_string());
        text.push(',');
        text.push_str(&a.mtime_nanos.to_string());
        text.push_str("]}");
    }
    text.push_str("],\"members\":[");
    for (i, m) in plan.members.iter().enumerate() {
        if i > 0 {
            text.push(',');
        }
        text.push_str("{\"a\":");
        text.push_str(&m.archive.to_string());
        text.push_str(",\"m\":");
        text.push_str(&m.member.to_string());
        text.push_str(",\"name\":\"");
        esc_json(&mut text, &m.name);
        text.push_str("\"}");
    }
    text.push_str("],\"user_defined\":[");
    for (i, s) in plan.user_defined.iter().enumerate() {
        if i > 0 {
            text.push(',');
        }
        text.push('"');
        esc_json(&mut text, s);
        text.push('"');
    }
    text.push_str("],\"user_unresolved\":[");
    for (i, s) in plan.user_unresolved.iter().enumerate() {
        if i > 0 {
            text.push(',');
        }
        text.push('"');
        esc_json(&mut text, s);
        text.push('"');
    }
    text.push_str("]}");
    if let Err(e) = std::fs::create_dir_all(dir) {
        return Err(LinkError::Io(e.to_string()));
    }
    let tmp = dir.join(format!("{MANIFEST_NAME}.tmp-{}", std::process::id()));
    if let Err(e) = std::fs::write(&tmp, text.as_bytes()) {
        let _ = std::fs::remove_file(&tmp);
        return Err(LinkError::Io(e.to_string()));
    }
    if let Err(e) = std::fs::rename(&tmp, dir.join(MANIFEST_NAME)) {
        let _ = std::fs::remove_file(&tmp);
        return Err(LinkError::Io(e.to_string()));
    }
    Ok(())
}

struct Cursor<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn err(&self, what: &str) -> LinkError {
        LinkError::Native(format!("link-plan.json: {what}"))
    }

    fn skip_ws(&mut self) {
        while self.pos < self.b.len() && matches!(self.b[self.pos], b' ' | b'\t' | b'\n' | b'\r') {
            self.pos += 1;
        }
    }

    fn expect(&mut self, tok: &[u8]) -> Result<(), LinkError> {
        self.skip_ws();
        if self.b.get(self.pos..self.pos + tok.len()) == Some(tok) {
            self.pos += tok.len();
            Ok(())
        } else {
            Err(self.err("unexpected token"))
        }
    }

    fn peek(&mut self) -> Option<u8> {
        self.skip_ws();
        self.b.get(self.pos).copied()
    }

    fn string(&mut self) -> Result<String, LinkError> {
        self.expect(b"\"")?;
        let mut out = String::new();
        loop {
            let c = *self.b.get(self.pos).ok_or_else(|| self.err("unterminated string"))?;
            self.pos += 1;
            match c {
                b'"' => return Ok(out),
                b'\\' => {
                    let e = *self.b.get(self.pos).ok_or_else(|| self.err("bad escape"))?;
                    self.pos += 1;
                    match e {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let hex = self.b.get(self.pos..self.pos + 4).ok_or_else(|| self.err("bad unicode"))?;
                            let hex = std::str::from_utf8(hex).map_err(|_| self.err("bad unicode"))?;
                            let cp = u32::from_str_radix(hex, 16).map_err(|_| self.err("bad unicode"))?;
                            self.pos += 4;
                            out.push(char::from_u32(cp).ok_or_else(|| self.err("bad unicode"))?);
                        }
                        _ => return Err(self.err("bad escape")),
                    }
                }
                0x00..=0x1f => return Err(self.err("control character")),
                _ => {
                    let start = self.pos - 1;
                    let mut end = start + 1;
                    while end < self.b.len() && self.b[end] >= 0x80 {
                        end += 1;
                        if end - start >= 4 {
                            break;
                        }
                    }
                    let s = std::str::from_utf8(self.b.get(start..end).ok_or_else(|| self.err("bad utf8"))?)
                        .map_err(|_| self.err("bad utf8"))?;
                    let ch = s.chars().next().ok_or_else(|| self.err("bad utf8"))?;
                    out.push(ch);
                    self.pos = start + ch.len_utf8();
                }
            }
        }
    }

    fn number_u64(&mut self) -> Result<u64, LinkError> {
        self.skip_ws();
        let start = self.pos;
        while self.pos < self.b.len() && self.b[self.pos].is_ascii_digit() {
            self.pos += 1;
        }
        if start == self.pos {
            return Err(self.err("expected number"));
        }
        std::str::from_utf8(self.b.get(start..self.pos).ok_or_else(|| self.err("bad number"))?)
            .map_err(|_| self.err("bad number"))?
            .parse::<u64>()
            .map_err(|_| self.err("bad number"))
    }

    fn string_array(&mut self) -> Result<Vec<String>, LinkError> {
        self.expect(b"[")?;
        let mut out = Vec::new();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(out);
        }
        loop {
            out.push(self.string()?);
            self.skip_ws();
            match self.peek() {
                Some(b',') => {
                    self.pos += 1;
                }
                Some(b']') => {
                    self.pos += 1;
                    return Ok(out);
                }
                _ => return Err(self.err("expected , or ]")),
            }
        }
    }

    fn skip_value(&mut self) -> Result<(), LinkError> {
        match self.peek().ok_or_else(|| self.err("unexpected end"))? {
            b'"' => {
                self.string()?;
                Ok(())
            }
            b'{' => {
                self.expect(b"{")?;
                if self.peek() == Some(b'}') {
                    self.pos += 1;
                    return Ok(());
                }
                loop {
                    self.string()?;
                    self.expect(b":")?;
                    self.skip_value()?;
                    match self.peek().ok_or_else(|| self.err("unexpected end"))? {
                        b',' => {
                            self.pos += 1;
                        }
                        b'}' => {
                            self.pos += 1;
                            return Ok(());
                        }
                        _ => return Err(self.err("expected , or }")),
                    }
                }
            }
            b'[' => {
                self.expect(b"[")?;
                if self.peek() == Some(b']') {
                    self.pos += 1;
                    return Ok(());
                }
                loop {
                    self.skip_value()?;
                    match self.peek().ok_or_else(|| self.err("unexpected end"))? {
                        b',' => {
                            self.pos += 1;
                        }
                        b']' => {
                            self.pos += 1;
                            return Ok(());
                        }
                        _ => return Err(self.err("expected , or ]")),
                    }
                }
            }
            c if c.is_ascii_digit() || c == b'-' => {
                if self.b[self.pos] == b'-' {
                    self.pos += 1;
                }
                while self.pos < self.b.len()
                    && (self.b[self.pos].is_ascii_digit() || matches!(self.b[self.pos], b'.' | b'e' | b'E' | b'+' | b'-'))
                {
                    self.pos += 1;
                }
                Ok(())
            }
            _ => {
                for lit in ["true", "false", "null"] {
                    if self.b.get(self.pos..self.pos + lit.len()) == Some(lit.as_bytes()) {
                        self.pos += lit.len();
                        return Ok(());
                    }
                }
                Err(self.err("unexpected value"))
            }
        }
    }
}

pub fn read_manifest(dir: &Path) -> Result<PlanData, LinkError> {
    let bytes = std::fs::read(dir.join(MANIFEST_NAME)).map_err(|e| LinkError::Io(e.to_string()))?;
    let mut c = Cursor { b: &bytes, pos: 0 };
    c.expect(b"{")?;
    let mut magic = String::new();
    let mut version = 0u64;
    let mut toolchain = String::new();
    let mut opts = String::new();
    let mut archives: Vec<ArchiveIdent> = Vec::new();
    let mut members: Vec<MemberKey> = Vec::new();
    let mut user_defined: Vec<String> = Vec::new();
    let mut user_unresolved: Vec<String> = Vec::new();
    loop {
        c.skip_ws();
        if c.peek() == Some(b'}') {
            break;
        }
        let name = c.string()?;
        c.expect(b":")?;
        match name.as_str() {
            "magic" => magic = c.string()?,
            "version" => version = c.number_u64()?,
            "toolchain" => toolchain = c.string()?,
            "opts" => opts = c.string()?,
            "user_defined" => user_defined = c.string_array()?,
            "user_unresolved" => user_unresolved = c.string_array()?,
            "archives" => {
                c.expect(b"[")?;
                if c.peek() == Some(b']') {
                    c.pos += 1;
                } else {
                    loop {
                        c.expect(b"{")?;
                        let mut tag = String::new();
                        let mut path = String::new();
                        let mut size = 0u64;
                        let mut mtime = (0u64, 0u64);
                        loop {
                            c.skip_ws();
                            if c.peek() == Some(b'}') {
                                c.pos += 1;
                                break;
                            }
                            let f = c.string()?;
                            c.expect(b":")?;
                            match f.as_str() {
                                "tag" => tag = c.string()?,
                                "path" => path = c.string()?,
                                "size" => size = c.number_u64()?,
                                "mtime" => {
                                    c.expect(b"[")?;
                                    let s = c.number_u64()?;
                                    c.expect(b",")?;
                                    let n = c.number_u64()?;
                                    c.expect(b"]")?;
                                    mtime = (s, n);
                                }
                                _ => c.skip_value()?,
                            }
                            c.skip_ws();
                            match c.peek() {
                                Some(b',') => {
                                    c.pos += 1;
                                }
                                Some(b'}') => continue,
                                _ => return Err(c.err("expected , or }")),
                            }
                        }
                        archives.push(ArchiveIdent {
                            tag,
                            path,
                            size,
                            mtime_secs: mtime.0,
                            mtime_nanos: mtime.1 as u32,
                        });
                        c.skip_ws();
                        match c.peek() {
                            Some(b',') => {
                                c.pos += 1;
                            }
                            Some(b']') => {
                                c.pos += 1;
                                break;
                            }
                            _ => return Err(c.err("expected , or ]")),
                        }
                    }
                }
            }
            "members" => {
                c.expect(b"[")?;
                if c.peek() == Some(b']') {
                    c.pos += 1;
                } else {
                    loop {
                        c.expect(b"{")?;
                        let (mut a, mut m, mut name) = (0u64, 0u64, String::new());
                        loop {
                            c.skip_ws();
                            if c.peek() == Some(b'}') {
                                c.pos += 1;
                                break;
                            }
                            let f = c.string()?;
                            c.expect(b":")?;
                            match f.as_str() {
                                "a" => a = c.number_u64()?,
                                "m" => m = c.number_u64()?,
                                "name" => name = c.string()?,
                                _ => c.skip_value()?,
                            }
                            c.skip_ws();
                            match c.peek() {
                                Some(b',') => {
                                    c.pos += 1;
                                }
                                Some(b'}') => continue,
                                _ => return Err(c.err("expected , or }")),
                            }
                        }
                        members.push(MemberKey { archive: a as u32, member: m as u32, name });
                        c.skip_ws();
                        match c.peek() {
                            Some(b',') => {
                                c.pos += 1;
                            }
                            Some(b']') => {
                                c.pos += 1;
                                break;
                            }
                            _ => return Err(c.err("expected , or ]")),
                        }
                    }
                }
            }
            _ => c.skip_value()?,
        }
        c.skip_ws();
        match c.peek() {
            Some(b',') => {
                c.pos += 1;
            }
            Some(b'}') => continue,
            None => return Err(c.err("unexpected end")),
            _ => return Err(c.err("expected , or }")),
        }
    }
    if magic != MAGIC || version != u64::from(FORMAT_VERSION) {
        return Err(LinkError::Native("link-plan.json: version mismatch".to_string()));
    }
    Ok(PlanData { toolchain, opts, archives, members, user_defined, user_unresolved })
}

pub struct LivePlan {
    toolchain: String,
    opts: String,
    archives: Vec<ArchiveIdent>,
    user_defined: Vec<String>,
    user_unresolved: Vec<String>,
    order: Vec<MemberKey>,
    pub loaded: bool,
    pub extended: bool,
}

fn sorted_vec(set: &BTreeSet<String>) -> Vec<String> {
    set.iter().cloned().collect()
}

impl LivePlan {
    pub fn load(
        ctx: &CacheCtx,
        opts: &str,
        idents: &[ArchiveIdent],
        user_defined: &BTreeSet<String>,
        user_unresolved: &BTreeSet<String>,
    ) -> Option<LivePlan> {
        let manifest = read_manifest(&ctx.dir).ok()?;
        if manifest.toolchain != ctx.toolchain || manifest.opts != opts {
            return None;
        }
        if manifest.user_defined != sorted_vec(user_defined) || manifest.user_unresolved != sorted_vec(user_unresolved) {
            return None;
        }
        if manifest.archives.len() != idents.len() {
            return None;
        }
        for (a, b) in manifest.archives.iter().zip(idents.iter()) {
            if a.tag != b.tag || a.path != b.path || a.size != b.size || a.mtime_secs != b.mtime_secs || a.mtime_nanos != b.mtime_nanos
            {
                return None;
            }
        }
        let order = manifest.members;
        Some(LivePlan {
            toolchain: ctx.toolchain.clone(),
            opts: opts.to_string(),
            archives: idents.to_vec(),
            user_defined: manifest.user_defined,
            user_unresolved: manifest.user_unresolved,
            order,
            loaded: true,
            extended: false,
        })
    }

    pub fn blank(
        ctx: &CacheCtx,
        opts: &str,
        idents: Vec<ArchiveIdent>,
        user_defined: &BTreeSet<String>,
        user_unresolved: &BTreeSet<String>,
    ) -> LivePlan {
        LivePlan {
            toolchain: ctx.toolchain.clone(),
            opts: opts.to_string(),
            archives: idents,
            user_defined: sorted_vec(user_defined),
            user_unresolved: sorted_vec(user_unresolved),
            order: Vec::new(),
            loaded: false,
            extended: false,
        }
    }

    pub fn ordered_members(&self) -> &[MemberKey] {
        &self.order
    }

    pub fn storable(&self) -> bool {
        self.archives.iter().all(|a| a.tag != "unhashed")
    }

    pub fn flush(&self, ctx: &CacheCtx, pulls: &[(u32, u32, usize, String)]) -> Result<(), LinkError> {
        let mut members: Vec<MemberKey> = Vec::with_capacity(pulls.len());
        for (ai, mi, _, name) in pulls {
            members.push(MemberKey { archive: *ai, member: *mi, name: name.clone() });
        }
        let plan = PlanData {
            toolchain: self.toolchain.clone(),
            opts: self.opts.clone(),
            archives: self
                .archives
                .iter()
                .map(|a| ArchiveIdent {
                    tag: a.tag.clone(),
                    path: a.path.clone(),
                    size: a.size,
                    mtime_secs: a.mtime_secs,
                    mtime_nanos: a.mtime_nanos,
                })
                .collect(),
            members,
            user_defined: self.user_defined.clone(),
            user_unresolved: self.user_unresolved.clone(),
        };
        write_manifest(&ctx.dir, &plan)?;
        let stale = ctx.dir.join("link-plan.bin");
        if stale.is_file() {
            let _ = std::fs::remove_file(&stale);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rnx-plan-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn manifest_roundtrip() {
        let dir = tmpdir("roundtrip");
        let plan = PlanData {
            toolchain: "rnx=0.1.0 llvm=22 rt=abc\"def\\ghi".to_string(),
            opts: "entry=_start icf=1".to_string(),
            archives: vec![ArchiveIdent {
                tag: "sha256:00".to_string(),
                path: "/usr/lib/libc.a".to_string(),
                size: 7000522,
                mtime_secs: 123,
                mtime_nanos: 456,
            }],
            members: vec![
                MemberKey { archive: 0, member: 7, name: "foo.o".to_string() },
                MemberKey { archive: 1, member: 0, name: "bar/baz.o".to_string() },
            ],
            user_defined: vec!["_start".to_string(), "main".to_string()],
            user_unresolved: vec!["printf".to_string()],
        };
        write_manifest(&dir, &plan).unwrap();
        let back = read_manifest(&dir).unwrap();
        assert_eq!(back.toolchain, plan.toolchain);
        assert_eq!(back.opts, plan.opts);
        assert_eq!(back.archives.len(), 1);
        assert_eq!(back.archives[0].tag, "sha256:00");
        assert_eq!(back.archives[0].path, "/usr/lib/libc.a");
        assert_eq!(back.archives[0].size, 7000522);
        assert_eq!(back.archives[0].mtime_secs, 123);
        assert_eq!(back.archives[0].mtime_nanos, 456);
        assert_eq!(back.members.len(), 2);
        assert_eq!(back.members[0].archive, 0);
        assert_eq!(back.members[0].member, 7);
        assert_eq!(back.members[0].name, "foo.o");
        assert_eq!(back.members[1].name, "bar/baz.o");
        assert_eq!(back.user_defined, vec!["_start".to_string(), "main".to_string()]);
        assert_eq!(back.user_unresolved, vec!["printf".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_inputs_are_errors() {
        let dir = tmpdir("corrupt");
        assert!(read_manifest(&dir).is_err());
        std::fs::write(dir.join(MANIFEST_NAME), b"{not json").unwrap();
        assert!(read_manifest(&dir).is_err());
        std::fs::write(dir.join(MANIFEST_NAME), b"{\"magic\":\"NOPE\",\"version\":2}").unwrap();
        assert!(read_manifest(&dir).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn live_plan_roundtrip_through_flush() {
        let dir = tmpdir("live");
        let ctx = CacheCtx { dir: dir.clone(), toolchain: "t".to_string(), allow_write: true };
        let mut defined: BTreeSet<String> = BTreeSet::new();
        defined.insert("main".to_string());
        let mut unresolved: BTreeSet<String> = BTreeSet::new();
        unresolved.insert("printf".to_string());
        let idents = vec![ArchiveIdent {
            tag: "sha256:00".to_string(),
            path: String::new(),
            size: 10,
            mtime_secs: 0,
            mtime_nanos: 0,
        }];
        let blank = LivePlan::blank(&ctx, "o", idents, &defined, &unresolved);
        assert!(!blank.loaded);
        blank
            .flush(&ctx, &[(0, 3, 9, "x.o".to_string()), (0, 5, 10, "y.o".to_string())])
            .unwrap();
        let back = LivePlan::load(&ctx, "o", &blank.archives, &defined, &unresolved).unwrap();
        assert!(back.loaded);
        assert_eq!(back.ordered_members().len(), 2);
        assert_eq!(back.ordered_members()[0].name, "x.o");
        assert_eq!(back.ordered_members()[1].member, 5);
        let mut other = unresolved.clone();
        other.insert("extra".to_string());
        assert!(LivePlan::load(&ctx, "o", &blank.archives, &defined, &other).is_none());
        assert!(LivePlan::load(&ctx, "other-opts", &blank.archives, &defined, &unresolved).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
