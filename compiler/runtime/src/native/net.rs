use super::common::*;
use super::reactor;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_net_connect_start(host: *const u8, port: i64) -> i64 {
    reactor::net_connect_start(&native_str(host), port)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_net_take_error(handle: i64) -> i64 {
    reactor::net_take_error(handle)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_net_connect_wait(handle: i64) -> i64 {
    reactor::net_connect_wait(handle)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_net_recv_or_wait(handle: i64, max: i64) -> i64 {
    reactor::net_recv_or_wait(handle, max)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_net_send_or_wait(handle: i64, byte: i64) -> i64 {
    reactor::net_send_or_wait(handle, byte)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_net_recv_get(handle: i64, idx: i64) -> i64 {
    reactor::net_recv_get(handle, idx)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_net_error_text(handle: i64) -> *mut u8 {
    alloc_str(&reactor::net_error_text(handle))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_net_close(handle: i64) -> i64 {
    reactor::net_close(handle)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_dns_lookup_start(host: *const u8) -> i64 {
    reactor::dns_lookup_start(&native_str(host))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_dns_lookup_wait(slot: i64) -> i64 {
    reactor::dns_lookup_wait(slot)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_dns_lookup_get(slot: i64) -> *mut u8 {
    alloc_str(&reactor::dns_lookup_get(slot))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_dns_lookup_error(slot: i64) -> *mut u8 {
    alloc_str(&reactor::dns_lookup_error(slot))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_net_listener_bind(host: *const u8, port: i64) -> i64 {
    reactor::net_listener_bind(&native_str(host), port)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_net_listener_port(handle: i64) -> i64 {
    reactor::net_listener_port(handle)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_net_listener_accept_start(handle: i64) -> i64 {
    reactor::net_listener_accept_start(handle)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_net_listener_accept_wait(slot: i64) -> i64 {
    reactor::net_listener_accept_wait(slot)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_net_listener_close(handle: i64) -> i64 {
    reactor::net_listener_close(handle)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_tls_connect_start(tcp_handle: i64, domain: *const u8) -> i64 {
    reactor::tls::tls_connect_start(tcp_handle, &native_str(domain))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_tls_handshake_start(handle: i64) -> i64 {
    reactor::tls::tls_handshake_start(handle)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_tls_handshake_wait(slot: i64) -> i64 {
    reactor::tls::tls_handshake_wait(slot)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_tls_recv_or_wait(handle: i64, max: i64) -> i64 {
    reactor::tls::tls_recv_or_wait(handle, max)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_tls_send_or_wait(handle: i64, byte: i64) -> i64 {
    reactor::tls::tls_send_or_wait(handle, byte)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_tls_recv_get(handle: i64, idx: i64) -> i64 {
    reactor::tls::tls_recv_get(handle, idx)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_tls_error_text(handle: i64) -> *mut u8 {
    alloc_str(&reactor::tls::tls_error_text(handle))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_tls_close(handle: i64) -> i64 {
    reactor::tls::tls_close(handle)
}
