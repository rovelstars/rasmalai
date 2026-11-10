//! WASI preview 1 process surface for `wasm32-unknown-wasi`.
//!
//! The module imports exactly two functions, both from
//! `wasi_snapshot_preview1`: `fd_write` renders console bytes and
//! `proc_exit` ends the process with a status. The target is built for
//! `wasm32-unknown-unknown` with these imports declared by hand, because
//! the `wasm32-wasip2` standard library emits preview 2 component imports
//! that a preview 1 host such as Node's `node:wasi` cannot satisfy.

const FD_STDOUT: i32 = 1;
const FD_STDERR: i32 = 2;

#[repr(C)]
struct Iovec {
    ptr: u32,
    len: u32,
}

#[link(wasm_import_module = "wasi_snapshot_preview1")]
unsafe extern "C" {
    fn fd_write(fd: i32, iovs: *const Iovec, iovs_len: i32, nwritten: *mut u32) -> i32;
    fn proc_exit(code: i32) -> !;
}

fn write_fd(fd: i32, mut bytes: &[u8]) {
    while !bytes.is_empty() {
        let iovec = Iovec {
            ptr: bytes.as_ptr() as u32,
            len: bytes.len() as u32,
        };
        let mut written = 0u32;
        let errno = unsafe { fd_write(fd, &iovec, 1, &mut written) };
        if errno != 0 || written == 0 {
            return;
        }
        bytes = &bytes[(written as usize).min(bytes.len())..];
    }
}

pub fn write_bytes(bytes: &[u8]) {
    write_fd(FD_STDOUT, bytes);
}

pub fn write_error_bytes(bytes: &[u8]) {
    write_fd(FD_STDERR, bytes);
}

pub fn write_str(text: &str) {
    write_bytes(text.as_bytes());
}

pub fn write_error_str(text: &str) {
    write_error_bytes(text.as_bytes());
}

pub fn exit(code: i32) -> ! {
    unsafe { proc_exit(code) }
}

pub fn format_i64(value: i64, out: &mut [u8; 32]) -> &[u8] {
    let mut buffer = StackBuffer::new();
    if core::fmt::Write::write_fmt(&mut buffer, format_args!("{value}")).is_err() {
        return b"?";
    }
    buffer.copy_into(out)
}

pub fn format_f64(value: f64, out: &mut [u8; 32]) -> &[u8] {
    let mut buffer = StackBuffer::new();
    if core::fmt::Write::write_fmt(&mut buffer, format_args!("{value}")).is_err() {
        return b"?";
    }
    buffer.copy_into(out)
}

struct StackBuffer {
    data: [u8; 64],
    len: usize,
}

impl StackBuffer {
    fn new() -> StackBuffer {
        StackBuffer {
            data: [0; 64],
            len: 0,
        }
    }

    fn copy_into<'a>(&self, out: &'a mut [u8; 32]) -> &'a [u8] {
        let mut n = 0;
        while n < self.len && n < out.len() {
            out[n] = self.data[n];
            n += 1;
        }
        &out[..n]
    }
}

impl core::fmt::Write for StackBuffer {
    fn write_str(&mut self, text: &str) -> core::fmt::Result {
        for &byte in text.as_bytes() {
            if self.len == self.data.len() {
                return Err(core::fmt::Error);
            }
            self.data[self.len] = byte;
            self.len += 1;
        }
        Ok(())
    }
}
