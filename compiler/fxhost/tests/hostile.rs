use fx::{
    decode_frame, encode_call_body, encode_frame, ArgValue, FnSpec, FX_FOREIGN, FX_MSG_CALL, FX_OK,
};
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rnx-fxhost-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn build_hostile(dir: &std::path::Path) -> std::path::PathBuf {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("hostile.c");
    let so = dir.join("libhostile.so");
    let status = std::process::Command::new("cc")
        .args(["-shared", "-fPIC", "-O1", "-o"])
        .arg(&so)
        .arg(&src)
        .status()
        .expect("cc must be installed for the helper test");
    assert!(status.success(), "hostile fixture did not compile");
    so
}

struct Session {
    stream: UnixStream,
    next_id: u64,
    specs: Vec<FnSpec>,
}

impl Session {
    fn call(&mut self, index: u32, args: Vec<u8>) -> (u8, u64, Vec<u8>) {
        let id = self.next_id;
        self.next_id += 1;
        let body = encode_call_body(index, &args, "test").unwrap();
        let frame = encode_frame(FX_MSG_CALL, id, &body, "test").unwrap();
        self.stream.write_all(&frame).unwrap();
        self.stream.flush().unwrap();
        let mut len = [0u8; 4];
        self.stream.read_exact(&mut len).unwrap();
        let total = u32::from_le_bytes(len) as usize;
        let mut body = vec![0u8; total];
        self.stream.read_exact(&mut body).unwrap();
        let (status, rid, payload) = decode_frame(&body).unwrap();
        assert_eq!(rid, id);
        (status, rid, payload.to_vec())
    }

    fn call_values(&mut self, index: u32, values: &[ArgValue]) -> (u8, Vec<u8>) {
        let args = fx::encode_args(&self.specs[index as usize], values, self.next_id).unwrap();
        let (status, _, payload) = self.call(index, args);
        (status, payload)
    }
}

fn start_session(dir: &std::path::Path, so: &std::path::Path) -> (std::process::Child, Session) {
    let sock = dir.join("fx.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    let helper = std::path::Path::new(env!("CARGO_BIN_EXE_rnx-fx-host"));
    let mut child = std::process::Command::new(helper)
        .arg("--lib")
        .arg(so)
        .arg("--connect")
        .arg(&sock)
        .arg("--fn")
        .arg("fx_add:int:int,int")
        .arg("--fn")
        .arg("fx_try_open_etc:int:")
        .arg("--fn")
        .arg("fx_try_connect:int:")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("helper must spawn");
    listener.set_nonblocking(false).unwrap();
    let (stream, _) = listener.accept().expect("helper must connect");
    let specs = ["fx_add:int:int,int", "fx_try_open_etc:int:", "fx_try_connect:int:"]
        .iter()
        .map(|s| fx::parse_fn_spec(s).unwrap())
        .collect();
    (child, Session { stream, next_id: 1, specs })
}

#[test]
fn hostile_library_fails_inside_helper_while_host_keeps_authority() {
    let dir = scratch("hostile");
    let so = build_hostile(&dir);
    let host_can_read = std::fs::read_to_string("/etc/passwd").is_ok();
    assert!(host_can_read, "host must keep its own file access");
    let (mut child, mut session) = start_session(&dir, &so);
    let (status, payload) = session.call_values(0, &[ArgValue::Int(2), ArgValue::Int(3)]);
    assert_eq!(status, FX_OK, "benign call must pass through");
    assert_eq!(payload.len(), 4);
    assert_eq!(i32::from_le_bytes(payload[..4].try_into().unwrap()), 5);
    let (status, payload) = session.call_values(1, &[]);
    assert_eq!(status, FX_FOREIGN, "open(/etc/passwd) must fail inside the helper");
    assert_eq!(payload.len(), 4);
    let errno = u32::from_le_bytes(payload[..4].try_into().unwrap());
    assert!(errno == 13 || errno == 1, "errno must be EACCES or EPERM, got {errno}");
    let (status, payload) = session.call_values(2, &[]);
    assert_eq!(status, FX_FOREIGN, "connect must fail inside the helper");
    assert_eq!(payload.len(), 4);
    let errno = u32::from_le_bytes(payload[..4].try_into().unwrap());
    assert!(errno == 13 || errno == 1 || errno == 111, "errno must deny the connect, got {errno}");
    drop(session);
    let wait = child.wait().expect("helper must exit");
    assert!(wait.success(), "helper must exit cleanly after the peer closes");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn unknown_symbol_refuses_startup() {
    let dir = scratch("badsym");
    let so = build_hostile(&dir);
    let sock = dir.join("fx.sock");
    let _listener = UnixListener::bind(&sock).unwrap();
    let helper = std::path::Path::new(env!("CARGO_BIN_EXE_rnx-fx-host"));
    let mut child = std::process::Command::new(helper)
        .arg("--lib")
        .arg(&so)
        .arg("--connect")
        .arg(&sock)
        .arg("--fn")
        .arg("no_such_symbol:int:")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("helper must spawn");
    let wait = child.wait().expect("helper must exit");
    assert!(!wait.success(), "unknown symbol must fail closed");
    let _ = std::fs::remove_dir_all(&dir);
}
