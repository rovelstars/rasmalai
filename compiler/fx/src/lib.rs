use diagnostics::{Code, Diagnostic};
use std::collections::BTreeMap;

pub const FX_FRAME_CAP: usize = 16 * 1024 * 1024;
pub const FX_MAX_ARGS: usize = 6;
pub const FX_MSG_CALL: u8 = 0;
pub const FX_MSG_RETAIN: u8 = 1;
pub const FX_MSG_RELEASE: u8 = 2;
pub const FX_OK: u8 = 0;
pub const FX_FOREIGN: u8 = 1;
pub const FX_TRANSPORT: u8 = 2;
pub const FX_T_OVERSIZE: u32 = 1;
pub const FX_T_STALE: u32 = 2;
pub const FX_T_BAD_INDEX: u32 = 3;
pub const FX_T_MALFORMED: u32 = 4;

pub fn transport(detail: impl Into<String>) -> Diagnostic {
    Diagnostic::new(Code::E209, format!("helper transport failure: {}", detail.into()))
}

pub fn transport_call(call_id: u64, detail: impl Into<String>) -> Diagnostic {
    Diagnostic::new(
        Code::E209,
        format!("helper transport failure on call {call_id}: {}", detail.into()),
    )
}

pub fn transport_fn(func: &str, size: usize) -> Diagnostic {
    Diagnostic::new(
        Code::E209,
        format!("helper transport failure: call to `{func}` needs {size} bytes over the 16 MiB frame cap"),
    )
}

pub fn encode_frame(kind: u8, call_id: u64, body: &[u8], func: &str) -> Result<Vec<u8>, Diagnostic> {
    let total = 1usize + 8 + body.len();
    if total > FX_FRAME_CAP {
        return Err(transport_fn(func, total));
    }
    let mut out = Vec::with_capacity(4 + total);
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.push(kind);
    out.extend_from_slice(&call_id.to_le_bytes());
    out.extend_from_slice(body);
    Ok(out)
}

pub fn decode_frame(bytes: &[u8]) -> Result<(u8, u64, &[u8]), Diagnostic> {
    if bytes.len() < 9 {
        return Err(transport("short frame"));
    }
    let kind = bytes[0];
    if kind != FX_MSG_CALL && kind != FX_MSG_RETAIN && kind != FX_MSG_RELEASE {
        return Err(transport("bad message kind"));
    }
    let mut id = [0u8; 8];
    id.copy_from_slice(&bytes[1..9]);
    Ok((kind, u64::from_le_bytes(id), &bytes[9..]))
}

pub fn encode_call_body(fn_index: u32, args: &[u8], func: &str) -> Result<Vec<u8>, Diagnostic> {
    let total = 4usize + args.len();
    if total > FX_FRAME_CAP {
        return Err(transport_fn(func, total));
    }
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(&fn_index.to_le_bytes());
    out.extend_from_slice(args);
    Ok(out)
}

pub fn decode_call_body(body: &[u8]) -> Result<(u32, &[u8]), Diagnostic> {
    if body.len() < 4 {
        return Err(transport("short call body"));
    }
    let mut index = [0u8; 4];
    index.copy_from_slice(&body[..4]);
    Ok((u32::from_le_bytes(index), &body[4..]))
}

pub fn encode_id_body(id: u64) -> Vec<u8> {
    id.to_le_bytes().to_vec()
}

pub fn decode_id_body(body: &[u8]) -> Result<u64, Diagnostic> {
    if body.len() != 8 {
        return Err(transport("bad handle body"));
    }
    let mut id = [0u8; 8];
    id.copy_from_slice(body);
    Ok(u64::from_le_bytes(id))
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ArgType {
    Int,
    Long,
    Float,
    Byte,
    Bool,
    Buf,
    Handle,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RetType {
    Void,
    Int,
    Long,
    Float,
    Byte,
    Bool,
    Handle,
}

pub fn parse_arg_type(text: &str) -> Option<ArgType> {
    Some(match text {
        "int" => ArgType::Int,
        "long" => ArgType::Long,
        "float" => ArgType::Float,
        "byte" => ArgType::Byte,
        "bool" => ArgType::Bool,
        "buf" => ArgType::Buf,
        "handle" => ArgType::Handle,
        _ => return None,
    })
}

pub fn parse_ret_type(text: &str) -> Option<RetType> {
    Some(match text {
        "void" => RetType::Void,
        "int" => RetType::Int,
        "long" => RetType::Long,
        "float" => RetType::Float,
        "byte" => RetType::Byte,
        "bool" => RetType::Bool,
        "handle" => RetType::Handle,
        _ => return None,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FnSpec {
    pub name: String,
    pub ret: RetType,
    pub params: Vec<ArgType>,
}

pub fn parse_fn_spec(text: &str) -> Result<FnSpec, String> {
    let mut parts = text.splitn(3, ':');
    let name = parts.next().unwrap_or_default().trim();
    let ret = parts.next().unwrap_or_default().trim();
    let params = parts.next().unwrap_or_default().trim();
    if name.is_empty() {
        return Err(format!("bad helper function spec `{text}` (expected `name:ret:params`)"));
    }
    if !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
        return Err(format!("bad helper function name `{name}`"));
    }
    let Some(ret) = parse_ret_type(ret) else {
        return Err(format!("bad helper return type in `{text}`"));
    };
    let mut out = Vec::new();
    if !params.is_empty() {
        for p in params.split(',') {
            let p = p.trim();
            if p == "ptr" || p == "voidptr" {
                return Err(format!("unpaired pointer parameter in `{text}` (declare `buf` or `handle`)"));
            }
            match parse_arg_type(p) {
                Some(t) => out.push(t),
                None => return Err(format!("bad helper parameter type `{p}` in `{text}`")),
            }
        }
    }
    Ok(FnSpec { name: name.to_string(), ret, params: out })
}

#[derive(Clone, Debug, PartialEq)]
pub enum ArgValue {
    Int(i64),
    Float(f64),
    Byte(u8),
    Bool(bool),
    Buf(Vec<u8>),
    Handle(u64),
}

fn arg_matches(ty: ArgType, value: &ArgValue) -> bool {
    matches!(
        (ty, value),
        (ArgType::Int, ArgValue::Int(_))
            | (ArgType::Long, ArgValue::Int(_))
            | (ArgType::Float, ArgValue::Float(_))
            | (ArgType::Byte, ArgValue::Byte(_))
            | (ArgType::Bool, ArgValue::Bool(_))
            | (ArgType::Buf, ArgValue::Buf(_))
            | (ArgType::Handle, ArgValue::Handle(_))
    )
}

fn encode_int(value: i64, call_id: u64, name: &str) -> Result<[u8; 4], Diagnostic> {
    if value < i64::from(i32::MIN) || value > i64::from(i32::MAX) {
        return Err(transport_call(call_id, format!("`{name}` int argument {value} is out of 32-bit range")));
    }
    Ok((value as i32).to_le_bytes())
}

pub fn encode_args(spec: &FnSpec, args: &[ArgValue], call_id: u64) -> Result<Vec<u8>, Diagnostic> {
    if args.len() != spec.params.len() {
        return Err(transport_call(
            call_id,
            format!("`{}` takes {} arguments, got {}", spec.name, spec.params.len(), args.len()),
        ));
    }
    if spec.params.len() > FX_MAX_ARGS {
        return Err(transport_call(call_id, format!("`{}` takes more than {FX_MAX_ARGS} arguments", spec.name)));
    }
    let mut out = Vec::new();
    for (ty, value) in spec.params.iter().zip(args.iter()) {
        if !arg_matches(*ty, value) {
            return Err(transport_call(call_id, format!("bad argument type for `{}`", spec.name)));
        }
        match value {
            ArgValue::Int(n) => match ty {
                ArgType::Int => out.extend_from_slice(&encode_int(*n, call_id, &spec.name)?),
                _ => out.extend_from_slice(&n.to_le_bytes()),
            },
            ArgValue::Float(f) => out.extend_from_slice(&f.to_le_bytes()),
            ArgValue::Byte(b) => out.push(*b),
            ArgValue::Bool(b) => out.push(u8::from(*b)),
            ArgValue::Buf(bytes) => {
                out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
                out.extend_from_slice(bytes);
            }
            ArgValue::Handle(id) => out.extend_from_slice(&id.to_le_bytes()),
        }
    }
    Ok(out)
}

pub fn decode_args(spec: &FnSpec, mut bytes: &[u8], call_id: u64) -> Result<Vec<ArgValue>, Diagnostic> {
    let bad = || transport_call(call_id, format!("malformed arguments for `{}`", spec.name));
    let mut out = Vec::with_capacity(spec.params.len());
    for ty in &spec.params {
        match ty {
            ArgType::Int => {
                if bytes.len() < 4 {
                    return Err(bad());
                }
                let mut n = [0u8; 4];
                n.copy_from_slice(&bytes[..4]);
                out.push(ArgValue::Int(i64::from(i32::from_le_bytes(n))));
                bytes = &bytes[4..];
            }
            ArgType::Long => {
                if bytes.len() < 8 {
                    return Err(bad());
                }
                let mut n = [0u8; 8];
                n.copy_from_slice(&bytes[..8]);
                out.push(ArgValue::Int(i64::from_le_bytes(n)));
                bytes = &bytes[8..];
            }
            ArgType::Float => {
                if bytes.len() < 8 {
                    return Err(bad());
                }
                let mut n = [0u8; 8];
                n.copy_from_slice(&bytes[..8]);
                out.push(ArgValue::Float(f64::from_le_bytes(n)));
                bytes = &bytes[8..];
            }
            ArgType::Byte => {
                if bytes.is_empty() {
                    return Err(bad());
                }
                out.push(ArgValue::Byte(bytes[0]));
                bytes = &bytes[1..];
            }
            ArgType::Bool => {
                if bytes.is_empty() {
                    return Err(bad());
                }
                out.push(ArgValue::Bool(bytes[0] != 0));
                bytes = &bytes[1..];
            }
            ArgType::Buf => {
                if bytes.len() < 4 {
                    return Err(bad());
                }
                let mut n = [0u8; 4];
                n.copy_from_slice(&bytes[..4]);
                let len = u32::from_le_bytes(n) as usize;
                if bytes.len() < 4 + len {
                    return Err(bad());
                }
                out.push(ArgValue::Buf(bytes[4..4 + len].to_vec()));
                bytes = &bytes[4 + len..];
            }
            ArgType::Handle => {
                if bytes.len() < 8 {
                    return Err(bad());
                }
                let mut n = [0u8; 8];
                n.copy_from_slice(&bytes[..8]);
                out.push(ArgValue::Handle(u64::from_le_bytes(n)));
                bytes = &bytes[8..];
            }
        }
    }
    if !bytes.is_empty() {
        return Err(bad());
    }
    Ok(out)
}

pub fn encode_ret_value(ret: RetType, int: i64, float: f64) -> Vec<u8> {
    match ret {
        RetType::Void => Vec::new(),
        RetType::Int => (int as i32).to_le_bytes().to_vec(),
        RetType::Long | RetType::Handle => int.to_le_bytes().to_vec(),
        RetType::Float => float.to_le_bytes().to_vec(),
        RetType::Byte | RetType::Bool => vec![int as u8],
    }
}

#[derive(Clone, Debug)]
struct HandleEntry {
    addr: usize,
    refs: u32,
}

#[derive(Clone, Debug, Default)]
pub struct HandleTable {
    next: u64,
    live: BTreeMap<u64, HandleEntry>,
}

impl HandleTable {
    pub fn new() -> HandleTable {
        HandleTable { next: 1, live: BTreeMap::new() }
    }

    pub fn intern(&mut self, addr: usize) -> u64 {
        let id = self.next;
        self.next += 1;
        self.live.insert(id, HandleEntry { addr, refs: 1 });
        id
    }

    pub fn resolve(&self, id: u64) -> Option<usize> {
        self.live.get(&id).map(|e| e.addr)
    }

    pub fn retain(&mut self, id: u64) -> bool {
        match self.live.get_mut(&id) {
            Some(e) => {
                e.refs = e.refs.saturating_add(1);
                true
            }
            None => false,
        }
    }

    pub fn release(&mut self, id: u64) -> bool {
        match self.live.get_mut(&id) {
            Some(e) => {
                e.refs -= 1;
                if e.refs == 0 {
                    self.live.remove(&id);
                }
                true
            }
            None => false,
        }
    }

    pub fn len(&self) -> usize {
        self.live.len()
    }

    pub fn is_empty(&self) -> bool {
        self.live.is_empty()
    }
}

pub fn is_gate_refusal(reason: &str) -> bool {
    matches!(reason, "variadic" | "callback" | "voidptr" | "va_list")
}

pub fn refused_surface(used: &[&str], skipped: &[(String, String)]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for name in used {
        for (skipped_name, reason) in skipped {
            if skipped_name == name && is_gate_refusal(reason) {
                out.push(((*name).to_string(), reason.clone()));
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

pub fn check_proxy_surface(used: &[&str], skipped: &[(String, String)]) -> Result<(), Diagnostic> {
    let refused = refused_surface(used, skipped);
    if refused.is_empty() {
        return Ok(());
    }
    let mut names: Vec<String> = refused.iter().map(|(n, r)| format!("`{n}` ({r})")).collect();
    names.sort();
    Err(Diagnostic::new(
        Code::E208,
        format!("foreign surface is not proxyable: {} cannot cross the helper boundary", names.join(", ")),
    ))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GateOutcome {
    Proxyable,
    ForcedNone { refused: Vec<(String, String)> },
}

pub fn gate_outcome(used: &[&str], skipped: &[(String, String)]) -> GateOutcome {
    let refused = refused_surface(used, skipped);
    if refused.is_empty() {
        GateOutcome::Proxyable
    } else {
        GateOutcome::ForcedNone { refused }
    }
}

pub fn unpaired_param(func: &str, param: &str) -> Diagnostic {
    Diagnostic::new(
        Code::E208,
        format!("foreign function `{func}` takes an unpaired pointer `{param}` with no recorded size; sized buffers cross as `buf`, opaque handles as `handle`"),
    )
}

pub fn enforce_link(lib: &str, artifact: &str, isolation: &str, via_helper: bool) -> Option<Diagnostic> {
    if isolation == "process" && !via_helper {
        Some(Diagnostic::new(
            Code::E207,
            format!("artifact `{artifact}` requires `isolation: process` but `-l{lib}` links outside the helper; route the call through `rnx-fx-host`"),
        ))
    } else {
        None
    }
}

pub fn require_live(artifact: &str, live: bool) -> Option<Diagnostic> {
    if live {
        None
    } else {
        Some(Diagnostic::new(
            Code::E207,
            format!("artifact `{artifact}` is not live; its blobs are refused"),
        ))
    }
}

pub fn check_profile_drift(artifact: &str, recorded: &str, current: &str) -> Option<Diagnostic> {
    if recorded == current {
        None
    } else {
        Some(Diagnostic::new(
            Code::E208,
            format!("artifact `{artifact}` changed classifier bucket since image build; re-run bindgen into a new image and a new version"),
        ))
    }
}

pub fn parse_isolation(value: &str) -> Result<&'static str, String> {
    match value {
        "none" => Ok("none"),
        "process" => Ok("process"),
        _ => Err(format!("bad run.rasmalai.isolation `{value}` (expected `none` or `process`)")),
    }
}

pub fn stricter_isolation(a: &str, b: &str) -> &'static str {
    if a == "process" || b == "process" {
        "process"
    } else {
        "none"
    }
}

pub fn lib_name_from_title(title: &str) -> Option<String> {
    let base = title.rsplit('/').next().unwrap_or(title);
    let core = match base.find(".so.") {
        Some(i) => &base[..i + 3],
        None => base,
    };
    let stripped = core
        .strip_prefix("lib")
        .and_then(|s| s.strip_suffix(".so"))
        .or_else(|| core.strip_suffix(".so"))
        .or_else(|| core.strip_prefix("lib").and_then(|s| s.strip_suffix(".dylib")))
        .or_else(|| core.strip_suffix(".dylib"))
        .or_else(|| core.strip_suffix(".dll"))?;
    let stem = stripped.split('.').next().unwrap_or(stripped);
    if stem.is_empty() || !stem.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') {
        return None;
    }
    Some(stem.to_string())
}

pub fn marker_isolation(marker_dir: &std::path::Path) -> Option<String> {
    let text = std::fs::read_to_string(marker_dir.join(".rnx-artifact")).ok()?;
    for line in text.lines().skip(1) {
        if let Some(value) = line.strip_prefix("isolation=") {
            let value = value.trim();
            if value == "none" || value == "process" {
                return Some(value.to_string());
            }
        }
    }
    None
}

pub fn process_libs(cache_root: &std::path::Path) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut stack = vec![cache_root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let mut entries = match std::fs::read_dir(&dir) {
            Ok(e) => e.flatten().map(|e| e.path()).collect::<Vec<_>>(),
            Err(_) => continue,
        };
        entries.sort();
        for path in entries {
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.file_name().and_then(|n| n.to_str()) != Some(".rnx-artifact") {
                continue;
            }
            let Some(pkg_dir) = path.parent() else { continue };
            if marker_isolation(pkg_dir) != Some("process".to_string()) {
                continue;
            }
            let label = pkg_dir.to_string_lossy().into_owned();
            for title in role_titles(pkg_dir) {
                if let Some(lib) = lib_name_from_title(&title) {
                    out.entry(lib).or_insert_with(|| label.clone());
                }
            }
        }
    }
    out
}

fn role_titles(pkg_dir: &std::path::Path) -> Vec<String> {
    let mut out = Vec::new();
    for role in ["runtime", "wasm"] {
        let role_dir = pkg_dir.join(".artifacts").join(role);
        let mut blobs = match std::fs::read_dir(&role_dir) {
            Ok(e) => e.flatten().map(|e| e.path()).collect::<Vec<_>>(),
            Err(_) => continue,
        };
        blobs.sort();
        for blob in blobs {
            if !blob.is_file() {
                continue;
            }
            if let Some(title) = blob.file_name().and_then(|n| n.to_str()) {
                out.push(title.to_string());
            }
        }
    }
    out
}

fn integrity_pair(pkg_dir: &std::path::Path) -> Option<(String, String)> {
    let text = std::fs::read_to_string(pkg_dir.join(".rnx-integrity")).ok()?;
    let mut lines = text.lines();
    let full = lines.next()?.trim().to_string();
    let version = lines.next()?.trim().to_string();
    if full.is_empty() || version.is_empty() {
        return None;
    }
    Some((full, version))
}

pub fn link_violations_for_lock(
    wanted: &std::collections::BTreeSet<(String, String)>,
    cache_root: &std::path::Path,
    native_libs: &[String],
) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let mut stack = vec![cache_root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let mut entries = match std::fs::read_dir(&dir) {
            Ok(e) => e.flatten().map(|e| e.path()).collect::<Vec<_>>(),
            Err(_) => continue,
        };
        entries.sort();
        for path in entries {
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.file_name().and_then(|n| n.to_str()) != Some(".rnx-artifact") {
                continue;
            }
            let Some(pkg_dir) = path.parent() else { continue };
            if marker_isolation(pkg_dir) != Some("process".to_string()) {
                continue;
            }
            let Some((full, version)) = integrity_pair(pkg_dir) else { continue };
            if !wanted.contains(&(full.clone(), version.clone())) {
                continue;
            }
            let artifact = format!("{full}@{version}");
            for title in role_titles(pkg_dir) {
                if let Some(lib) = lib_name_from_title(&title) {
                    if native_libs.contains(&lib) {
                        if let Some(d) = enforce_link(&lib, &artifact, "process", false) {
                            out.push(d);
                        }
                    }
                }
            }
        }
    }
    out.sort_by(|a, b| a.message.cmp(&b.message));
    out.dedup_by(|a, b| a.message == b.message);
    out
}

#[cfg(not(target_arch = "wasm32"))]
pub fn helper_binary_name() -> &'static str {
    if cfg!(windows) { "rnx-fx-host.exe" } else { "rnx-fx-host" }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn helper_binary_path() -> std::path::PathBuf {
    match std::env::current_exe() {
        Ok(exe) => exe
            .parent()
            .map(|d| d.join(helper_binary_name()))
            .unwrap_or_else(|| std::path::PathBuf::from(helper_binary_name())),
        Err(_) => std::path::PathBuf::from(helper_binary_name()),
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn spawn_helper(
    lib: &str,
    sock_path: &std::path::Path,
    specs: &[String],
    allow: &[String],
    artifact: &str,
    live: bool,
) -> std::io::Result<std::process::Child> {
    if let Some(d) = require_live(artifact, live) {
        return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, d.message));
    }
    let mut cmd = std::process::Command::new(helper_binary_path());
    cmd.arg("--lib").arg(lib);
    cmd.arg("--connect").arg(sock_path);
    for spec in specs {
        cmd.arg("--fn").arg(spec);
    }
    for dir in allow {
        cmd.arg("--allow").arg(dir);
    }
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
}

#[cfg(not(target_arch = "wasm32"))]
pub fn send_frame(stream: &mut impl std::io::Write, frame: &[u8]) -> std::io::Result<()> {
    stream.write_all(frame)?;
    stream.flush()
}

#[cfg(not(target_arch = "wasm32"))]
pub fn recv_frame(stream: &mut impl std::io::Read) -> Result<Vec<u8>, Diagnostic> {
    let mut len = [0u8; 4];
    stream.read_exact(&mut len).map_err(|e| transport(format!("helper reply unreadable: {e}")))?;
    let total = u32::from_le_bytes(len) as usize;
    if total < 9 || total > FX_FRAME_CAP {
        return Err(transport("helper reply frame has a bad length"));
    }
    let mut body = vec![0u8; total];
    stream.read_exact(&mut body).map_err(|e| transport(format!("helper reply truncated: {e}")))?;
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("rnx-fx-test-{}-{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn frame_round_trips() {
        let body = encode_call_body(7, &[1, 2, 3], "f").unwrap();
        let frame = encode_frame(FX_MSG_CALL, 42, &body, "f").unwrap();
        let len = u32::from_le_bytes(frame[..4].try_into().unwrap()) as usize;
        assert_eq!(len, frame.len() - 4);
        let (kind, id, rest) = decode_frame(&frame[4..]).unwrap();
        assert_eq!((kind, id), (FX_MSG_CALL, 42));
        let (index, args) = decode_call_body(rest).unwrap();
        assert_eq!((index, args), (7, &[1, 2, 3][..]));
    }

    #[test]
    fn oversize_frame_fails_closed_naming_function() {
        let big = vec![0u8; FX_FRAME_CAP];
        let err = encode_frame(FX_MSG_CALL, 1, &big, "bigfn").unwrap_err();
        assert_eq!(err.code, Code::E209);
        assert!(err.message.contains("bigfn"), "{}", err.message);
        assert!(err.message.contains("16 MiB"), "{}", err.message);
    }

    #[test]
    fn bad_frames_fail_closed() {
        assert!(decode_frame(&[FX_MSG_CALL, 0]).is_err());
        assert!(decode_frame(&[9u8, 0, 0, 0, 0, 0, 0, 0, 0]).is_err());
        assert!(decode_call_body(&[1, 2]).is_err());
        assert!(decode_id_body(&[1, 2, 3]).is_err());
        assert_eq!(decode_id_body(&7u64.to_le_bytes()).unwrap(), 7);
    }

    #[test]
    fn fn_spec_rejects_raw_pointers() {
        assert!(parse_fn_spec("add:int:int,int").is_ok());
        assert!(parse_fn_spec("v:void:").is_ok());
        assert!(parse_fn_spec("get:handle:handle").is_ok());
        assert!(parse_fn_spec("bad:int:ptr").is_err());
        assert!(parse_fn_spec("bad:int:voidptr").is_err());
        assert!(parse_fn_spec("bad:ptr:int").is_err());
        assert!(parse_fn_spec(":int:int").is_err());
    }

    #[test]
    fn args_round_trip_all_shapes() {
        let spec = parse_fn_spec("f:int:int,long,float,byte,bool,buf").unwrap();
        let args = vec![
            ArgValue::Int(-5),
            ArgValue::Int(i64::MIN),
            ArgValue::Float(1.5),
            ArgValue::Byte(65),
            ArgValue::Bool(true),
            ArgValue::Buf(vec![9, 8, 7]),
        ];
        let bytes = encode_args(&spec, &args, 3).unwrap();
        assert_eq!(decode_args(&spec, &bytes, 3).unwrap(), args);
        assert!(encode_args(&spec, &args[..5], 3).is_err());
        let mut bad = bytes.clone();
        bad.push(0);
        assert!(decode_args(&spec, &bad, 3).is_err());
        let wide = parse_fn_spec("f:int:int").unwrap();
        assert!(encode_args(&wide, &[ArgValue::Int(i64::from(i32::MAX) + 1)], 3).is_err());
        let many = parse_fn_spec("f:int:int,int,int,int,int,int,int").unwrap();
        let zeros = vec![ArgValue::Int(0); 7];
        assert!(encode_args(&many, &zeros, 3).is_err());
        assert_eq!(encode_ret_value(RetType::Int, -1, 0.0), vec![0xff, 0xff, 0xff, 0xff]);
        assert_eq!(encode_ret_value(RetType::Long, -1, 0.0).len(), 8);
    }

    #[test]
    fn handle_table_interns_and_dies_with_exit() {
        let mut table = HandleTable::new();
        let a = table.intern(0x1000);
        let b = table.intern(0x2000);
        assert_ne!(a, b);
        assert_eq!(table.resolve(a), Some(0x1000));
        assert!(table.retain(a));
        assert!(table.release(a));
        assert_eq!(table.resolve(a), Some(0x1000));
        assert!(table.release(a));
        assert_eq!(table.resolve(a), None);
        assert!(!table.retain(a));
        assert!(!table.release(a));
        drop(table);
        let fresh = HandleTable::new();
        assert!(fresh.is_empty());
        assert_eq!(fresh.resolve(b), None);
    }

    #[test]
    fn gate_refuses_callbacks_variadics_and_voidptr() {
        let skipped = vec![
            ("cb".to_string(), "callback".to_string()),
            ("flex".to_string(), "variadic".to_string()),
            ("alloc".to_string(), "voidptr".to_string()),
            ("fill".to_string(), "podptr".to_string()),
        ];
        let refused = refused_surface(&["cb", "flex", "alloc", "fill"], &skipped);
        assert_eq!(refused.len(), 3);
        let err = check_proxy_surface(&["cb", "fill"], &skipped).unwrap_err();
        assert_eq!(err.code, Code::E208);
        assert!(err.message.contains("`cb`"), "{}", err.message);
        assert!(check_proxy_surface(&["fill"], &skipped).is_ok());
        assert!(matches!(gate_outcome(&["fill"], &skipped), GateOutcome::Proxyable));
        match gate_outcome(&["cb"], &skipped) {
            GateOutcome::ForcedNone { refused } => assert_eq!(refused.len(), 1),
            GateOutcome::Proxyable => panic!("expected forced none"),
        }
    }

    #[test]
    fn enforcement_names_artifact_and_requirement() {
        let err = enforce_link("sqlite3", "ghcr.io/org/sqlite3", "process", false).unwrap();
        assert_eq!(err.code, Code::E207);
        assert!(err.message.contains("ghcr.io/org/sqlite3"), "{}", err.message);
        assert!(err.message.contains("process"), "{}", err.message);
        assert!(enforce_link("sqlite3", "ghcr.io/org/sqlite3", "process", true).is_none());
        assert!(enforce_link("sqlite3", "ghcr.io/org/sqlite3", "none", false).is_none());
        let err = require_live("ghcr.io/org/sqlite3", false).unwrap();
        assert_eq!(err.code, Code::E207);
        assert!(require_live("ghcr.io/org/sqlite3", true).is_none());
        let err = check_profile_drift("a", "h1", "h2").unwrap();
        assert_eq!(err.code, Code::E208);
        assert!(check_profile_drift("a", "h1", "h1").is_none());
        assert_eq!(parse_isolation("process"), Ok("process"));
        assert!(parse_isolation("proxyable").is_err());
        assert_eq!(stricter_isolation("none", "process"), "process");
    }

    #[test]
    fn title_maps_to_lib_name() {
        assert_eq!(lib_name_from_title("libfoo.so"), Some("foo".to_string()));
        assert_eq!(lib_name_from_title("libfoo.so.1"), Some("foo".to_string()));
        assert_eq!(lib_name_from_title("foo.dll"), Some("foo".to_string()));
        assert_eq!(lib_name_from_title("libfoo.dylib"), Some("foo".to_string()));
        assert_eq!(lib_name_from_title("notes.txt"), None);
    }

    #[test]
    fn link_violations_are_scoped_to_the_lock_closure() {
        let root = scratch("violations");
        let cache = root.join("cache");
        for (pkg, full, version, isolation, titles) in [
            ("host-a", "org/hostile", "1.0.0", "process", vec!["libhostile.so"]),
            ("host-b", "org/other", "2.0.0", "process", vec!["libother.so"]),
            ("mild", "org/mild", "1.0.0", "none", vec!["libmild.so"]),
        ] {
            let dir = cache.join("example.invalid").join(pkg);
            std::fs::create_dir_all(dir.join(".artifacts").join("runtime")).unwrap();
            std::fs::write(dir.join(".rnx-artifact"), format!("sha256:{}\nisolation={isolation}\n", "ab".repeat(32))).unwrap();
            std::fs::write(dir.join(".rnx-integrity"), format!("{full}\n{version}\n{}\n", "cd".repeat(32))).unwrap();
            for title in titles {
                std::fs::write(dir.join(".artifacts").join("runtime").join(title), b"x").unwrap();
            }
        }
        let wanted: std::collections::BTreeSet<(String, String)> =
            [("org/hostile".to_string(), "1.0.0".to_string())].into_iter().collect();
        let libs = ["hostile".to_string(), "other".to_string(), "mild".to_string(), "z".to_string()];
        let found = link_violations_for_lock(&wanted, &cache, &libs);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].code, Code::E207);
        assert!(found[0].message.contains("org/hostile@1.0.0"), "{}", found[0].message);
        assert!(found[0].message.contains("-lhostile"), "{}", found[0].message);
        assert!(link_violations_for_lock(&wanted, &cache, &["z".to_string()]).is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn cache_scan_finds_process_libs_only() {
        let root = scratch("cache");
        for (pkg, isolation, titles) in [
            ("pkg-a", "process", vec!["libhostile.so"]),
            ("pkg-b", "none", vec!["libmild.so"]),
        ] {
            let dir = root.join(pkg);
            std::fs::create_dir_all(dir.join(".artifacts").join("runtime")).unwrap();
            std::fs::write(dir.join(".rnx-artifact"), format!("sha256:{}\nisolation={isolation}\n", "ab".repeat(32))).unwrap();
            for title in titles {
                std::fs::write(dir.join(".artifacts").join("runtime").join(title), b"x").unwrap();
            }
        }
        let found = process_libs(&root);
        assert_eq!(found.len(), 1);
        assert!(found.contains_key("hostile"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
