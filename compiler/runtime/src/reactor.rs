use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::ToSocketAddrs;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::Duration;

#[cfg(not(target_arch = "wasm32"))]
#[path = "tls.rs"]
pub mod tls;

const WAKE_TOKEN: mio::Token = mio::Token(0);
const MAX_CHUNK: usize = 65536;
const PARK_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) struct SlotState {
    done: bool,
    result: i64,
}

pub(crate) struct OpSlot {
    state: Mutex<SlotState>,
    cond: Condvar,
}

impl OpSlot {
    pub(crate) fn new() -> OpSlot {
        OpSlot { state: Mutex::new(SlotState { done: false, result: -1 }), cond: Condvar::new() }
    }

    pub(crate) fn complete(&self, result: i64) {
        let mut s = match self.state.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        if s.done {
            return;
        }
        s.done = true;
        s.result = result;
        self.cond.notify_all();
    }

    pub(crate) fn park_timeout(&self, _op: Op, _handle: i64, timeout: Duration) -> i64 {
        let mut s = match self.state.lock() {
            Ok(g) => g,
            Err(_) => return -1,
        };
        while !s.done {
            let (g, res) = match self.cond.wait_timeout(s, timeout) {
                Ok(v) => v,
                Err(_) => return -1,
            };
            s = g;
            if res.timed_out() && !s.done {
                continue;
            }
        }
        s.result
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Op {
    Connect,
    Read,
    Write,
    Accept,
    TlsHandshake,
    TlsRead,
    TlsWrite,
    Dns,
}

pub(crate) struct Request {
    pub(crate) handle: i64,
    pub(crate) op: Op,
    pub(crate) max: i64,
    pub(crate) byte: i64,
    pub(crate) slot: std::sync::Arc<OpSlot>,
}

pub(crate) struct SocketEntry {
    pub(crate) stream: mio::net::TcpStream,
    read_buf: Vec<u8>,
    error: String,
    closed: bool,
}

struct ListenerEntry {
    listener: mio::net::TcpListener,
    closed: bool,
}

pub(crate) struct Inner {
    pub(crate) table: Mutex<HashMap<i64, SocketEntry>>,
    listeners: Mutex<HashMap<i64, ListenerEntry>>,
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) sessions: Mutex<HashMap<i64, tls::TlsSession>>,
    pub(crate) parked_slots: Mutex<HashMap<i64, (i64, std::sync::Arc<OpSlot>)>>,
    pub(crate) token_map: Mutex<HashMap<mio::Token, i64>>,
    pub(crate) queue: Mutex<Vec<Request>>,
    pub(crate) next_handle: AtomicI64,
    pub(crate) next_slot: AtomicI64,
    pub(crate) next_token: AtomicUsize,
    pub(crate) last_error: Mutex<String>,
    pub(crate) registry: mio::Registry,
    dns: Mutex<HashMap<i64, DnsEntry>>,
}

pub(crate) struct DnsEntry {
    slot: std::sync::Arc<OpSlot>,
    ip: Option<String>,
    err: Option<String>,
}

pub struct Reactor {
    pub(crate) registry: mio::Registry,
    pub(crate) waker: mio::Waker,
    pub(crate) inner: std::sync::Arc<Inner>,
}

static REACTOR: OnceLock<Reactor> = OnceLock::new();

fn reactor() -> &'static Reactor {
    REACTOR.get_or_init(|| {
        let mut poll = mio::Poll::new().expect("net reactor poll");
        let registry = poll.registry().try_clone().expect("net reactor registry");
        let waker = mio::Waker::new(&registry, WAKE_TOKEN).expect("net reactor waker");
        let inner = std::sync::Arc::new(Inner {
            table: Mutex::new(HashMap::new()),
            listeners: Mutex::new(HashMap::new()),
            #[cfg(not(target_arch = "wasm32"))]
            sessions: Mutex::new(HashMap::new()),
            parked_slots: Mutex::new(HashMap::new()),
            token_map: Mutex::new(HashMap::new()),
            queue: Mutex::new(Vec::new()),
            next_handle: AtomicI64::new(1),
            next_slot: AtomicI64::new(2),
            next_token: AtomicUsize::new(1),
            last_error: Mutex::new(String::new()),
            registry: registry.try_clone().expect("net reactor registry clone"),
            dns: Mutex::new(HashMap::new()),
        });
        let thread_inner = inner.clone();
        std::thread::Builder::new()
            .name("rnx-reactor".to_string())
            .spawn(move || run_loop(&mut poll, &thread_inner))
            .expect("net reactor thread");
        Reactor { registry, waker, inner }
    })
}

fn try_request(table: &mut HashMap<i64, SocketEntry>, req: &Request) -> Option<i64> {
    let entry = table.get_mut(&req.handle)?;
    if entry.closed {
        return Some(-1);
    }
    match req.op {
        Op::Accept => Some(-1),
        Op::TlsHandshake | Op::TlsRead | Op::TlsWrite | Op::Dns => Some(-1),
        Op::Connect => match entry.stream.take_error() {
            Ok(Some(e)) => {
                let code = e.raw_os_error().map(|c| c as i64).unwrap_or(-2);
                entry.error = format!("connect failed: {e}");
                Some(code)
            }
            Ok(None) => match entry.stream.peer_addr() {
                Ok(_) => Some(0),
                Err(e) if e.kind() == std::io::ErrorKind::NotConnected => None,
                Err(e) => {
                    entry.error = format!("peer check failed: {e}");
                    Some(-1)
                }
            },
            Err(e) => {
                entry.error = format!("take_error failed: {e}");
                Some(-2)
            }
        },
        Op::Read => {
            let cap = (req.max.max(0) as usize).min(MAX_CHUNK).max(1);
            let mut buf = vec![0u8; cap];
            match entry.stream.read(&mut buf) {
                Ok(n) => {
                    buf.truncate(n);
                    entry.read_buf = buf;
                    Some(n as i64)
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => None,
                Err(e) => {
                    entry.error = format!("recv failed: {e}");
                    Some(-1)
                }
            }
        }
        Op::Write => match entry.stream.write(&[req.byte as u8]) {
            Ok(1) => Some(1),
            Ok(_) => None,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => None,
            Err(e) => {
                entry.error = format!("send failed: {e}");
                Some(-1)
            }
        },
    }
}

fn try_accept(inner: &std::sync::Arc<Inner>, handle: i64) -> Option<i64> {
    let mut stream = {
        let mut listeners = match inner.listeners.lock() {
            Ok(g) => g,
            Err(_) => return Some(-1),
        };
        let Some(entry) = listeners.get_mut(&handle) else { return Some(-1) };
        if entry.closed {
            return Some(-1);
        }
        match entry.listener.accept() {
            Ok((s, _)) => s,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return None,
            Err(e) => {
                set_last_error(inner, format!("accept failed: {e}"));
                return Some(-1);
            }
        }
    };
    let stream_handle = inner.next_handle.fetch_add(1, Ordering::SeqCst);
    let token = mio::Token(inner.next_token.fetch_add(1, Ordering::SeqCst));
    if inner
        .registry
        .register(&mut stream, token, mio::Interest::READABLE | mio::Interest::WRITABLE)
        .is_err()
    {
        set_last_error(inner, "reactor register failed".to_string());
        return Some(-1);
    }
    if let Ok(mut t) = inner.table.lock() {
        t.insert(stream_handle, SocketEntry {
            stream,
            read_buf: Vec::new(),
            error: String::new(),
            closed: false,
        });
    }
    if let Ok(mut m) = inner.token_map.lock() {
        m.insert(token, stream_handle);
    }
    Some(stream_handle)
}

fn drain_queue(inner: &std::sync::Arc<Inner>) {
    let pending: Vec<Request> = match inner.queue.lock() {
        Ok(mut q) => std::mem::take(&mut *q),
        Err(_) => return,
    };
    if pending.is_empty() {
        return;
    }
    let mut accepts = Vec::new();
    let mut streams = Vec::new();
    for req in pending {
        if req.op == Op::Accept {
            accepts.push(req);
        } else {
            streams.push(req);
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    let mut tls_reqs = Vec::new();
    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut rest = Vec::new();
        for req in streams {
            match req.op {
                Op::TlsHandshake | Op::TlsRead | Op::TlsWrite => tls_reqs.push(req),
                _ => rest.push(req),
            }
        }
        streams = rest;
    }
    let mut keep = Vec::new();
    for req in accepts {
        match try_accept(inner, req.handle) {
            Some(result) => req.slot.complete(result),
            None => keep.push(req),
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    for req in tls_reqs {
        tls::retry_tls(inner, req, &mut keep);
    }
    if streams.is_empty() {
        if !keep.is_empty() {
            match inner.queue.lock() {
                Ok(mut q) => q.extend(keep),
                Err(_) => {
                    for req in keep {
                        req.slot.complete(-1);
                    }
                }
            }
        }
        return;
    }
    let mut table = match inner.table.lock() {
        Ok(g) => g,
        Err(_) => {
            for req in streams {
                req.slot.complete(-1);
            }
            for req in keep {
                req.slot.complete(-1);
            }
            return;
        }
    };
    for req in streams {
        match try_request(&mut table, &req) {
            Some(result) => req.slot.complete(result),
            None => match table.get(&req.handle) {
                Some(entry) if !entry.closed => keep.push(req),
                _ => req.slot.complete(-1),
            },
        }
    }
    drop(table);
    if !keep.is_empty() {
        match inner.queue.lock() {
            Ok(mut q) => q.extend(keep),
            Err(_) => {
                for req in keep {
                    req.slot.complete(-1);
                }
            }
        }
    }
}

fn retry_handle(inner: &std::sync::Arc<Inner>, handle: i64) {
    let pending: Vec<Request> = match inner.queue.lock() {
        Ok(mut q) => {
            let mut out = Vec::new();
            let mut rest = Vec::new();
            for req in q.drain(..) {
                if req.handle == handle {
                    out.push(req);
                } else {
                    rest.push(req);
                }
            }
            *q = rest;
            out
        }
        Err(_) => return,
    };
    if pending.is_empty() {
        return;
    }
    let mut accepts = Vec::new();
    let mut streams = Vec::new();
    for req in pending {
        if req.op == Op::Accept {
            accepts.push(req);
        } else {
            streams.push(req);
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    let mut tls_reqs = Vec::new();
    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut rest = Vec::new();
        for req in streams {
            match req.op {
                Op::TlsHandshake | Op::TlsRead | Op::TlsWrite => tls_reqs.push(req),
                _ => rest.push(req),
            }
        }
        streams = rest;
    }
    let mut keep = Vec::new();
    for req in accepts {
        match try_accept(inner, req.handle) {
            Some(result) => req.slot.complete(result),
            None => keep.push(req),
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    for req in tls_reqs {
        tls::retry_tls(inner, req, &mut keep);
    }
    if streams.is_empty() {
        if !keep.is_empty() {
            match inner.queue.lock() {
                Ok(mut q) => q.extend(keep),
                Err(_) => {
                    for req in keep {
                        req.slot.complete(-1);
                    }
                }
            }
        }
        return;
    }
    let mut table = match inner.table.lock() {
        Ok(g) => g,
        Err(_) => {
            for req in streams {
                req.slot.complete(-1);
            }
            for req in keep {
                req.slot.complete(-1);
            }
            return;
        }
    };
    for req in streams {
        match try_request(&mut table, &req) {
            Some(result) => req.slot.complete(result),
            None => match table.get(&req.handle) {
                Some(entry) if !entry.closed => keep.push(req),
                _ => req.slot.complete(-1),
            },
        }
    }
    drop(table);
    if !keep.is_empty() {
        match inner.queue.lock() {
            Ok(mut q) => q.extend(keep),
            Err(_) => {
                for req in keep {
                    req.slot.complete(-1);
                }
            }
        }
    }
}

fn run_loop(poll: &mut mio::Poll, inner: &std::sync::Arc<Inner>) {
    let mut events = mio::Events::with_capacity(1024);
    loop {
        if poll.poll(&mut events, None).is_err() {
            continue;
        }
        drain_queue(inner);
        for ev in events.iter() {
            if ev.token() == WAKE_TOKEN {
                continue;
            }
            let ready = ev.is_readable()
                || ev.is_writable()
                || ev.is_read_closed()
                || ev.is_write_closed()
                || ev.is_error();
            if !ready {
                continue;
            }
            let handle = inner.token_map.lock().map(|m| m.get(&ev.token()).copied()).unwrap_or(None);
            if let Some(h) = handle {
                retry_handle(inner, h);
            }
        }
    }
}

fn set_last_error(inner: &std::sync::Arc<Inner>, msg: String) {
    if let Ok(mut g) = inner.last_error.lock() {
        *g = msg;
    }
}

pub(crate) fn resolve_host(host: &str) -> Result<std::net::IpAddr, String> {
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        return Ok(ip);
    }
    let mut addrs = match (host, 0).to_socket_addrs() {
        Ok(a) => a,
        Err(e) => return Err(format!("cannot resolve `{host}`: {e}")),
    };
    let mut first: Option<std::net::IpAddr> = None;
    for a in addrs.by_ref() {
        let ip = a.ip();
        if first.is_none() {
            first = Some(ip);
        }
        if ip.is_ipv4() {
            return Ok(ip);
        }
    }
    first.ok_or_else(|| format!("cannot resolve `{host}`"))
}

fn port_u16(port: i64) -> Result<u16, String> {
    u16::try_from(port).map_err(|_| format!("invalid port `{port}`"))
}

pub fn net_connect_start(host: &str, port: i64) -> i64 {
    let r = reactor();
    let ip = match resolve_host(host) {
        Ok(ip) => ip,
        Err(msg) => {
            set_last_error(&r.inner, msg);
            return -1;
        }
    };
    let addr = match port_u16(port) {
        Ok(p) => std::net::SocketAddr::new(ip, p),
        Err(msg) => {
            set_last_error(&r.inner, msg);
            return -1;
        }
    };
    let mut stream = match mio::net::TcpStream::connect(addr) {
        Ok(s) => s,
        Err(e) => {
            set_last_error(&r.inner, format!("connect `{addr}` failed: {e}"));
            return -1;
        }
    };
    let handle = r.inner.next_handle.fetch_add(1, Ordering::SeqCst);
    let token = mio::Token(r.inner.next_token.fetch_add(1, Ordering::SeqCst));
    if let Err(e) = r.registry.register(
        &mut stream,
        token,
        mio::Interest::READABLE | mio::Interest::WRITABLE,
    ) {
        set_last_error(&r.inner, format!("reactor register failed: {e}"));
        return -1;
    }
    if let Ok(mut t) = r.inner.table.lock() {
        t.insert(handle, SocketEntry {
            stream,
            read_buf: Vec::new(),
            error: String::new(),
            closed: false,
        });
    }
    if let Ok(mut m) = r.inner.token_map.lock() {
        m.insert(token, handle);
    }
    handle
}

pub fn net_take_error(handle: i64) -> i64 {
    let r = reactor();
    let mut table = match r.inner.table.lock() {
        Ok(g) => g,
        Err(_) => return -1,
    };
    let Some(entry) = table.get_mut(&handle) else { return -1 };
    if entry.closed {
        return -1;
    }
    match entry.stream.take_error() {
        Ok(None) => 0,
        Ok(Some(e)) => {
            let code = e.raw_os_error().map(|c| c as i64).unwrap_or(-2);
            entry.error = format!("connect failed: {e}");
            code
        }
        Err(e) => {
            entry.error = format!("take_error failed: {e}");
            -2
        }
    }
}

fn submit(r: &Reactor, req: Request) -> i64 {
    submit_op(r, req)
}

pub(crate) fn submit_op(r: &Reactor, req: Request) -> i64 {
    let slot = req.slot.clone();
    let (op, handle) = (req.op, req.handle);
    {
        let mut queue = match r.inner.queue.lock() {
            Ok(g) => g,
            Err(_) => return -1,
        };
        queue.push(req);
    }
    let _ = r.waker.wake();
    slot.park_timeout(op, handle, PARK_TIMEOUT)
}

pub fn net_connect_wait(handle: i64) -> i64 {
    let r = reactor();
    let slot = std::sync::Arc::new(OpSlot::new());
    submit(r, Request { handle, op: Op::Connect, max: 0, byte: 0, slot })
}

pub fn net_recv_or_wait(handle: i64, max: i64) -> i64 {
    let r = reactor();
    let slot = std::sync::Arc::new(OpSlot::new());
    submit(r, Request { handle, op: Op::Read, max, byte: 0, slot })
}

pub fn net_send_or_wait(handle: i64, byte: i64) -> i64 {
    let r = reactor();
    let slot = std::sync::Arc::new(OpSlot::new());
    submit(r, Request { handle, op: Op::Write, max: 0, byte, slot })
}

pub fn net_recv_get(handle: i64, idx: i64) -> i64 {
    let r = reactor();
    let table = match r.inner.table.lock() {
        Ok(g) => g,
        Err(_) => return -1,
    };
    let Some(entry) = table.get(&handle) else { return -1 };
    if entry.closed {
        return -1;
    }
    entry.read_buf.get(idx as usize).copied().map(|b| b as i64).unwrap_or(-1)
}

pub fn net_error_text(handle: i64) -> String {
    let r = reactor();
    if handle < 0 {
        return r.inner.last_error.lock().map(|mut g| std::mem::take(&mut *g)).unwrap_or_default();
    }
    let mut table = match r.inner.table.lock() {
        Ok(g) => g,
        Err(_) => return String::new(),
    };
    match table.get_mut(&handle) {
        Some(entry) => std::mem::take(&mut entry.error),
        None => String::new(),
    }
}

pub fn net_close(handle: i64) -> i64 {
    let r = reactor();
    let entry = match r.inner.table.lock() {
        Ok(mut t) => t.remove(&handle),
        Err(_) => return -1,
    };
    let Some(mut entry) = entry else { return -1 };
    let _ = r.registry.deregister(&mut entry.stream);
    entry.closed = true;
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
    if let Ok(mut m) = r.inner.token_map.lock() {
        m.retain(|_, h| *h != handle);
    }
    0
}

pub fn dns_lookup_start(host: &str) -> i64 {
    let r = reactor();
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        let slot_id = r.inner.next_slot.fetch_add(1, Ordering::SeqCst);
        let slot = std::sync::Arc::new(OpSlot::new());
        slot.complete(0);
        if let Ok(mut dns) = r.inner.dns.lock() {
            dns.insert(slot_id, DnsEntry { slot, ip: Some(ip.to_string()), err: None });
        }
        return slot_id;
    }
    let slot_id = r.inner.next_slot.fetch_add(1, Ordering::SeqCst);
    let slot = std::sync::Arc::new(OpSlot::new());
    if let Ok(mut dns) = r.inner.dns.lock() {
        dns.insert(slot_id, DnsEntry { slot: slot.clone(), ip: None, err: None });
    }
    let inner = r.inner.clone();
    let owned = host.to_string();
    match std::thread::Builder::new().name("rnx-dns".to_string()).spawn(move || {
        let result = resolve_host(&owned);
        if let Ok(mut dns) = inner.dns.lock() {
            if let Some(entry) = dns.get_mut(&slot_id) {
                match result {
                    Ok(ip) => {
                        entry.ip = Some(ip.to_string());
                        entry.slot.complete(0);
                    }
                    Err(reason) => {
                        entry.err =
                            Some(format!("DNS resolution failed for host `{owned}`: {reason}"));
                        entry.slot.complete(-1);
                    }
                }
            }
        }
    }) {
        Ok(_) => slot_id,
        Err(e) => {
            if let Ok(mut dns) = r.inner.dns.lock() {
                if let Some(entry) = dns.get_mut(&slot_id) {
                    entry.err = Some(format!("DNS worker spawn failed: {e}"));
                    entry.slot.complete(-1);
                }
            }
            slot_id
        }
    }
}

pub fn dns_lookup_wait(slot_id: i64) -> i64 {
    let r = reactor();
    let slot = match r.inner.dns.lock() {
        Ok(g) => g.get(&slot_id).map(|e| e.slot.clone()),
        Err(_) => None,
    };
    match slot {
        Some(s) => s.park_timeout(Op::Dns, slot_id, PARK_TIMEOUT),
        None => -1,
    }
}

pub fn dns_lookup_get(slot_id: i64) -> String {
    let r = reactor();
    match r.inner.dns.lock() {
        Ok(mut g) => g.remove(&slot_id).and_then(|e| e.ip).unwrap_or_default(),
        Err(_) => String::new(),
    }
}

pub fn dns_lookup_error(slot_id: i64) -> String {
    let r = reactor();
    match r.inner.dns.lock() {
        Ok(mut g) => g
            .remove(&slot_id)
            .and_then(|e| e.err)
            .unwrap_or_else(|| "DNS resolution failed".to_string()),
        Err(_) => "DNS resolution failed".to_string(),
    }
}

pub fn net_listener_bind(host: &str, port: i64) -> i64 {
    let r = reactor();
    let ip = match resolve_host(host) {
        Ok(ip) => ip,
        Err(msg) => {
            set_last_error(&r.inner, msg);
            return -1;
        }
    };
    let addr = match port_u16(port) {
        Ok(p) => std::net::SocketAddr::new(ip, p),
        Err(msg) => {
            set_last_error(&r.inner, msg);
            return -1;
        }
    };
    let mut listener = match mio::net::TcpListener::bind(addr) {
        Ok(l) => l,
        Err(e) => {
            set_last_error(&r.inner, format!("bind `{addr}` failed: {e}"));
            return -1;
        }
    };
    let handle = r.inner.next_handle.fetch_add(1, Ordering::SeqCst);
    let token = mio::Token(r.inner.next_token.fetch_add(1, Ordering::SeqCst));
    if let Err(e) = r.registry.register(&mut listener, token, mio::Interest::READABLE) {
        set_last_error(&r.inner, format!("reactor register failed: {e}"));
        return -1;
    }
    if let Ok(mut t) = r.inner.listeners.lock() {
        t.insert(handle, ListenerEntry { listener, closed: false });
    }
    if let Ok(mut m) = r.inner.token_map.lock() {
        m.insert(token, handle);
    }
    handle
}

pub fn net_listener_port(handle: i64) -> i64 {
    let r = reactor();
    let listeners = match r.inner.listeners.lock() {
        Ok(g) => g,
        Err(_) => return -1,
    };
    match listeners.get(&handle) {
        Some(entry) if !entry.closed => entry
            .listener
            .local_addr()
            .map(|a| a.port() as i64)
            .unwrap_or(-1),
        _ => -1,
    }
}

pub fn net_listener_accept_start(handle: i64) -> i64 {
    let r = reactor();
    if let Ok(listeners) = r.inner.listeners.lock() {
        match listeners.get(&handle) {
            Some(entry) if !entry.closed => {}
            _ => return -1,
        }
    } else {
        return -1;
    }
    match try_accept(&r.inner, handle) {
        Some(stream_handle) => stream_handle,
        None => {
            // Park: slot ids start at 2 so the parked encoding -slot_id
            // never collides with the -1 fatal sentinel.
            let slot_id = r.inner.next_slot.fetch_add(1, Ordering::SeqCst);
            let slot = std::sync::Arc::new(OpSlot::new());
            if let Ok(mut slots) = r.inner.parked_slots.lock() {
                slots.insert(slot_id, (handle, slot.clone()));
            }
            {
                let mut queue = match r.inner.queue.lock() {
                    Ok(g) => g,
                    Err(_) => return -1,
                };
                queue.push(Request { handle, op: Op::Accept, max: 0, byte: 0, slot });
            }
            let _ = r.waker.wake();
            -slot_id
        }
    }
}

pub fn net_listener_accept_wait(slot_id: i64) -> i64 {
    parked_wait(&reactor().inner, slot_id, Op::Accept)
}

pub(crate) fn parked_wait(inner: &std::sync::Arc<Inner>, slot_id: i64, op: Op) -> i64 {
    if slot_id <= 1 {
        return -1;
    }
    let (handle, slot) = match inner.parked_slots.lock() {
        Ok(mut slots) => match slots.remove(&slot_id) {
            Some(v) => v,
            None => return -1,
        },
        Err(_) => return -1,
    };
    slot.park_timeout(op, handle, PARK_TIMEOUT)
}

pub fn net_listener_close(handle: i64) -> i64 {
    let r = reactor();
    let entry = match r.inner.listeners.lock() {
        Ok(mut t) => t.remove(&handle),
        Err(_) => return -1,
    };
    let Some(mut entry) = entry else { return -1 };
    let _ = r.registry.deregister(&mut entry.listener);
    entry.closed = true;
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
        let mut rest = HashMap::new();
        for (id, (h, slot)) in slots.drain() {
            if h == handle {
                slot.complete(-1);
            } else {
                rest.insert(id, (h, slot));
            }
        }
        *slots = rest;
    }
    if let Ok(mut m) = r.inner.token_map.lock() {
        m.retain(|_, h| *h != handle);
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    static TEST_SERIAL: Mutex<()> = Mutex::new(());

    fn serial() -> std::sync::MutexGuard<'static, ()> {
        TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn loopback_pair() -> (std::net::TcpStream, i64) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = net_connect_start("127.0.0.1", port as i64);
        assert!(handle > 0);
        let (server, _) = listener.accept().unwrap();
        server.set_nonblocking(false).unwrap();
        (server, handle)
    }

    #[test]
    fn park_timeout_rewaits_past_deadline() {
        let _guard = serial();
        let slot = std::sync::Arc::new(OpSlot::new());
        let waker = slot.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(150));
            waker.complete(42);
        });
        let got = slot.park_timeout(Op::Accept, 7, std::time::Duration::from_millis(50));
        assert_eq!(got, 42);
    }

    #[test]
    fn idle_socket_sleeps_without_spurious_wakeups() {
        let _guard = serial();
        let (mut server, handle) = loopback_pair();
        let woke = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = woke.clone();
        let waiter = std::thread::spawn(move || {
            let st = net_recv_or_wait(handle, 16);
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
            (st, handle)
        });
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert!(!woke.load(std::sync::atomic::Ordering::SeqCst), "idle socket woke with no data");
        server.write_all(b"x").unwrap();
        let (st, handle) = waiter.join().unwrap();
        assert_eq!(st, 1);
        assert!(woke.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(net_recv_get(handle, 0), 120);
        assert_eq!(net_close(handle), 0);
    }
    #[test]
    fn listener_accept_round_trip() {
        let _guard = serial();
        let lh = net_listener_bind("127.0.0.1", 0);
        assert!(lh > 0);
        let port = net_listener_port(lh);
        assert!(port > 0, "ephemeral port assigned");
        assert_eq!(net_listener_port(-999), -1);
        let ch = net_connect_start("127.0.0.1", port);
        assert!(ch > 0);
        assert_eq!(net_connect_wait(ch), 0);
        let first = net_listener_accept_start(lh);
        let sh = if first < 0 { net_listener_accept_wait(-first) } else { first };
        assert!(sh > 0);
        assert_eq!(net_listener_accept_start(-999), -1);
        assert_eq!(net_send_or_wait(ch, 65), 1);
        assert_eq!(net_recv_or_wait(sh, 16), 1);
        assert_eq!(net_recv_get(sh, 0), 65);
        assert_eq!(net_send_or_wait(sh, 66), 1);
        assert_eq!(net_recv_or_wait(ch, 16), 1);
        assert_eq!(net_recv_get(ch, 0), 66);
        assert_eq!(net_close(ch), 0);
        assert_eq!(net_close(sh), 0);
        assert_eq!(net_listener_close(lh), 0);
        assert_eq!(net_listener_close(lh), -1);
    }

    #[test]
    fn listener_accept_parked_wakes_on_connect() {
        let _guard = serial();
        let lh = net_listener_bind("127.0.0.1", 0);
        assert!(lh > 0);
        let port = net_listener_port(lh);
        let first = net_listener_accept_start(lh);
        assert!(first < 0, "no pending connection parks, got {first}");
        let slot = -first;
        let waiter = std::thread::spawn(move || net_listener_accept_wait(slot));
        std::thread::sleep(std::time::Duration::from_millis(200));
        let ch = net_connect_start("127.0.0.1", port);
        assert!(ch > 0);
        assert_eq!(net_connect_wait(ch), 0);
        let sh = waiter.join().unwrap();
        assert!(sh > 0);
        assert_eq!(net_close(ch), 0);
        assert_eq!(net_close(sh), 0);
        assert_eq!(net_listener_close(lh), 0);
    }

    #[test]
    fn burst_100_bare_connects() {
        let _guard = serial();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for _ in 0..100 {
                let _ = listener.accept();
            }
        });
        let mut handles = Vec::new();
        for _ in 0..100 {
            let h = net_connect_start("127.0.0.1", port as i64);
            assert!(h > 0);
            handles.push(h);
        }
        let mut workers = Vec::new();
        for h in handles {
            workers.push(std::thread::spawn(move || {
                assert_eq!(net_connect_wait(h), 0);
                assert_eq!(net_take_error(h), 0);
                assert_eq!(net_close(h), 0);
            }));
        }
        for w in workers {
            w.join().unwrap();
        }
    }

    #[test]
    fn readiness_round_trip() {
        let _guard = serial();
        let (mut server, handle) = loopback_pair();
        assert_eq!(net_connect_wait(handle), 0);
        assert_eq!(net_take_error(handle), 0);
        assert_eq!(net_send_or_wait(handle, 65), 1);
        let mut one = [0u8; 1];
        server.read_exact(&mut one).unwrap();
        assert_eq!(one, [65]);
        server.write_all(b"hi").unwrap();
        assert_eq!(net_recv_or_wait(handle, 16), 2);
        assert_eq!(net_recv_get(handle, 0), 104);
        assert_eq!(net_recv_get(handle, 1), 105);
        assert_eq!(net_close(handle), 0);
        assert_eq!(net_recv_or_wait(handle, 16), -1);
    }
}
