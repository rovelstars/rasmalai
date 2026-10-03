pub fn net_connect_start(_host: &str, _port: i64) -> i64 {
    -1
}

static LISTENER_FAILED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

pub fn net_take_error(_handle: i64) -> i64 {
    -1
}

pub fn net_connect_wait(_handle: i64) -> i64 {
    -1
}

pub fn net_recv_or_wait(_handle: i64, _max: i64) -> i64 {
    -1
}

pub fn net_send_or_wait(_handle: i64, _byte: i64) -> i64 {
    -1
}

pub fn net_recv_get(_handle: i64, _idx: i64) -> i64 {
    -1
}

pub fn net_error_text(_handle: i64) -> String {
    if LISTENER_FAILED.swap(false, std::sync::atomic::Ordering::SeqCst) {
        return "Server sockets are unsupported in WebAssembly".to_string();
    }
    "networking is not supported on this target".to_string()
}

pub fn net_close(_handle: i64) -> i64 {
    -1
}

pub fn net_listener_bind(_host: &str, _port: i64) -> i64 {
    LISTENER_FAILED.store(true, std::sync::atomic::Ordering::SeqCst);
    -1
}

pub fn net_listener_port(_handle: i64) -> i64 {
    -1
}

pub fn net_listener_accept_start(_handle: i64) -> i64 {
    -1
}

pub fn net_listener_accept_wait(_slot: i64) -> i64 {
    -1
}

pub fn net_listener_close(_handle: i64) -> i64 {
    -1
}

pub fn dns_lookup_start(_host: &str) -> i64 {
    -1
}

pub fn dns_lookup_wait(_slot: i64) -> i64 {
    -1
}

pub fn dns_lookup_get(_slot: i64) -> String {
    String::new()
}

pub fn dns_lookup_error(_slot: i64) -> String {
    "DNS resolution is unsupported in WebAssembly".to_string()
}

pub mod tls {
    pub fn tls_connect_start(_tcp_handle: i64, _domain: &str) -> i64 {
        -1
    }

    pub fn tls_handshake_start(_handle: i64) -> i64 {
        -1
    }

    pub fn tls_handshake_wait(_slot: i64) -> i64 {
        -1
    }

    pub fn tls_recv_or_wait(_handle: i64, _max: i64) -> i64 {
        -1
    }

    pub fn tls_send_or_wait(_handle: i64, _byte: i64) -> i64 {
        -1
    }

    pub fn tls_recv_get(_handle: i64, _idx: i64) -> i64 {
        -1
    }

    pub fn tls_error_text(_handle: i64) -> String {
        "TLS is not supported on this target".to_string()
    }

    pub fn tls_close(_handle: i64) -> i64 {
        -1
    }
}
