use std::io::{Read, Write};
use std::sync::atomic::Ordering;

use super::{set_last_error, Inner, Op, OpSlot, Request, MAX_CHUNK};

pub struct TlsSession {
    stream: mio::net::TcpStream,
    conn: rustls::ClientConnection,
    read_buf: Vec<u8>,
    error: String,
    closed: bool,
}

fn tls_config() -> std::sync::Arc<rustls::ClientConfig> {
    static TLS_CONFIG: std::sync::OnceLock<std::sync::Arc<rustls::ClientConfig>> =
        std::sync::OnceLock::new();
    TLS_CONFIG
        .get_or_init(|| {
            let provider = std::sync::Arc::new(rustls::crypto::ring::default_provider());
            let mut roots = rustls::RootCertStore::empty();
            if let Ok(ca_path) = std::env::var("RNX_TEST_TLS_CA_DER") {
                if let Ok(der) = std::fs::read(&ca_path) {
                    roots.add(rustls::pki_types::CertificateDer::from(der)).ok();
                }
            } else {
                roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            }
            std::sync::Arc::new(
                rustls::ClientConfig::builder_with_provider(provider)
                    .with_protocol_versions(rustls::ALL_VERSIONS)
                    .expect("tls versions")
                    .with_root_certificates(roots)
                    .with_no_client_auth(),
            )
        })
        .clone()
}

enum Pump {
    Done,
    Need,
    Fatal(String),
}

fn pump_handshake(s: &mut TlsSession) -> Pump {
    for _ in 0..32 {
        if !s.conn.is_handshaking() {
            return Pump::Done;
        }
        while s.conn.wants_write() {
            match s.conn.write_tls(&mut s.stream) {
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Pump::Need,
                Err(e) => return Pump::Fatal(format!("write failed: {e}")),
            }
        }
        match s.conn.read_tls(&mut s.stream) {
            Ok(0) => return Pump::Fatal("closed during handshake".to_string()),
            Ok(_) => {
                if let Err(e) = s.conn.process_new_packets() {
                    return Pump::Fatal(format!("{e}"));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Pump::Need,
            Err(e) => return Pump::Fatal(format!("read failed: {e}")),
        }
    }
    if s.conn.is_handshaking() {
        Pump::Need
    } else {
        Pump::Done
    }
}

fn drain_plaintext(s: &mut TlsSession, max: i64) -> i64 {
    let cap = (max.max(0) as usize).min(MAX_CHUNK).max(1);
    let mut buf = vec![0u8; cap];
    match s.conn.reader().read(&mut buf) {
        Ok(n) => {
            buf.truncate(n);
            s.read_buf = buf;
            n as i64
        }
        Err(_) => 0,
    }
}

fn try_tls(inner: &std::sync::Arc<Inner>, req: &Request) -> Option<i64> {
    let mut sessions = match inner.sessions.lock() {
        Ok(g) => g,
        Err(_) => return Some(-1),
    };
    let Some(s) = sessions.get_mut(&req.handle) else { return Some(-1) };
    if s.closed {
        return Some(-1);
    }
    if s.conn.is_handshaking() {
        match pump_handshake(s) {
            Pump::Done => {}
            Pump::Need => return None,
            Pump::Fatal(msg) => {
                s.error = msg;
                return Some(-1);
            }
        }
    }
    match req.op {
        Op::TlsHandshake => Some(0),
        Op::TlsRead => {
            let n = drain_plaintext(s, req.max);
            if n > 0 {
                return Some(n);
            }
            match s.conn.read_tls(&mut s.stream) {
                Ok(0) => Some(drain_plaintext(s, req.max)),
                Ok(_) => {
                    if let Err(e) = s.conn.process_new_packets() {
                        s.error = format!("read failed: {e}");
                        return Some(-1);
                    }
                    let n = drain_plaintext(s, req.max);
                    if n > 0 {
                        Some(n)
                    } else {
                        None
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => None,
                Err(e) => {
                    s.error = format!("read failed: {e}");
                    Some(-1)
                }
            }
        }
        Op::TlsWrite => {
            if s.conn.writer().write_all(&[req.byte as u8]).is_err() {
                s.error = "write failed".to_string();
                return Some(-1);
            }
            match s.conn.write_tls(&mut s.stream) {
                Ok(_) => Some(1),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => None,
                Err(e) => {
                    s.error = format!("write failed: {e}");
                    Some(-1)
                }
            }
        }
        _ => Some(-1),
    }
}

pub fn retry_tls(inner: &std::sync::Arc<Inner>, req: Request, keep: &mut Vec<Request>) {
    match try_tls(inner, &req) {
        Some(result) => req.slot.complete(result),
        None => keep.push(req),
    }
}

fn tls_live(inner: &std::sync::Arc<Inner>, handle: i64) -> bool {
    match inner.sessions.lock() {
        Ok(sessions) => matches!(sessions.get(&handle), Some(s) if !s.closed),
        Err(_) => false,
    }
}

pub fn tls_connect_start(tcp_handle: i64, domain: &str) -> i64 {
    let r = super::reactor();
    let entry = match r.inner.table.lock() {
        Ok(mut t) => t.remove(&tcp_handle),
        Err(_) => return -1,
    };
    let Some(entry) = entry else {
        set_last_error(&r.inner, format!("tls upgrade of missing stream {tcp_handle}"));
        return -1;
    };
    if entry.closed {
        set_last_error(&r.inner, "tls upgrade of closed stream".to_string());
        return -1;
    }
    let name = match rustls::pki_types::ServerName::try_from(domain.to_string()) {
        Ok(n) => n,
        Err(_) => {
            set_last_error(&r.inner, format!("tls invalid domain `{domain}`"));
            return -1;
        }
    };
    let conn = match rustls::ClientConnection::new(tls_config(), name) {
        Ok(c) => c,
        Err(e) => {
            set_last_error(&r.inner, format!("tls init failed: {e}"));
            return -1;
        }
    };
    let tls_handle = r.inner.next_handle.fetch_add(1, Ordering::SeqCst);
    if let Ok(mut sessions) = r.inner.sessions.lock() {
        sessions.insert(tls_handle, TlsSession {
            stream: entry.stream,
            conn,
            read_buf: Vec::new(),
            error: String::new(),
            closed: false,
        });
    }
    if let Ok(mut m) = r.inner.token_map.lock() {
        for h in m.values_mut() {
            if *h == tcp_handle {
                *h = tls_handle;
            }
        }
    }
    tls_handle
}

pub fn tls_handshake_start(handle: i64) -> i64 {
    let r = super::reactor();
    if !tls_live(&r.inner, handle) {
        return -1;
    }
    let slot = std::sync::Arc::new(OpSlot::new());
    let probe = Request { handle, op: Op::TlsHandshake, max: 0, byte: 0, slot: slot.clone() };
    match try_tls(&r.inner, &probe) {
        Some(result) => result,
        None => {
            let slot_id = r.inner.next_slot.fetch_add(1, Ordering::SeqCst);
            if let Ok(mut slots) = r.inner.parked_slots.lock() {
                slots.insert(slot_id, (handle, slot.clone()));
            }
            {
                let mut queue = match r.inner.queue.lock() {
                    Ok(g) => g,
                    Err(_) => return -1,
                };
                queue.push(Request { handle, op: Op::TlsHandshake, max: 0, byte: 0, slot });
            }
            let _ = r.waker.wake();
            -slot_id
        }
    }
}

pub fn tls_handshake_wait(slot_id: i64) -> i64 {
    let r = super::reactor();
    super::parked_wait(&r.inner, slot_id, Op::TlsHandshake)
}

pub fn tls_recv_or_wait(handle: i64, max: i64) -> i64 {
    let r = super::reactor();
    if !tls_live(&r.inner, handle) {
        return -1;
    }
    let slot = std::sync::Arc::new(OpSlot::new());
    super::submit_op(r, Request { handle, op: Op::TlsRead, max, byte: 0, slot })
}

pub fn tls_send_or_wait(handle: i64, byte: i64) -> i64 {
    let r = super::reactor();
    if !tls_live(&r.inner, handle) {
        return -1;
    }
    let slot = std::sync::Arc::new(OpSlot::new());
    super::submit_op(r, Request { handle, op: Op::TlsWrite, max: 0, byte, slot })
}

pub fn tls_recv_get(handle: i64, idx: i64) -> i64 {
    let r = super::reactor();
    let sessions = match r.inner.sessions.lock() {
        Ok(g) => g,
        Err(_) => return -1,
    };
    let Some(s) = sessions.get(&handle) else { return -1 };
    if s.closed {
        return -1;
    }
    s.read_buf.get(idx as usize).copied().map(|b| b as i64).unwrap_or(-1)
}

pub fn tls_error_text(handle: i64) -> String {
    let r = super::reactor();
    if handle < 0 {
        return r.inner.last_error.lock().map(|mut g| std::mem::take(&mut *g)).unwrap_or_default();
    }
    let mut sessions = match r.inner.sessions.lock() {
        Ok(g) => g,
        Err(_) => return String::new(),
    };
    match sessions.get_mut(&handle) {
        Some(s) => std::mem::take(&mut s.error),
        None => String::new(),
    }
}

pub fn tls_close(handle: i64) -> i64 {
    let r = super::reactor();
    let entry = match r.inner.sessions.lock() {
        Ok(mut t) => t.remove(&handle),
        Err(_) => return -1,
    };
    let Some(mut s) = entry else { return -1 };
    s.closed = true;
    let _ = s.conn.send_close_notify();
    let _ = r.inner.registry.deregister(&mut s.stream);
    if let Ok(mut q) = r.inner.queue.lock() {
        let mut rest = Vec::new();
        for req in q.drain(..) {
            if req.handle == handle {
                req.slot.complete(-1);
            } else {
                rest.push(req);
            }
        }
        *q = rest;
    }
    if let Ok(mut slots) = r.inner.parked_slots.lock() {
        slots.retain(|_, (h, slot)| {
            if *h == handle {
                slot.complete(-1);
                false
            } else {
                true
            }
        });
    }
    if let Ok(mut m) = r.inner.token_map.lock() {
        m.retain(|_, h| *h != handle);
    }
    0
}

