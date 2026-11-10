//! Firmware entry point, console output, and serial fallback for
//! `x86_64-unknown-uefi`.
//!
//! Everything here talks to the structures OVMF hands `efi_main`. No
//! operating system is involved, so the console is the ConOut protocol and
//! the only other byte sink is the 16550 UART at COM1 that qemu wires to
//! `-serial stdio`.

use core::arch::asm;
use core::fmt::Write;

pub const EFI_SUCCESS: usize = 0;
pub const EFI_ABORTED: usize = 21;
pub const EFI_LOADER_DATA: u32 = 2;

#[repr(C)]
pub struct TableHeader {
    pub signature: u64,
    pub revision: u32,
    pub header_size: u32,
    pub crc32: u32,
    pub reserved: u32,
}

#[repr(C)]
pub struct SimpleTextOutput {
    pub reset: usize,
    pub output_string: usize,
    pub test_string: usize,
    pub query_mode: usize,
    pub set_mode: usize,
    pub set_attribute: usize,
    pub clear_screen: usize,
    pub set_cursor_position: usize,
    pub enable_cursor: usize,
    pub mode: usize,
}

type AllocatePool = unsafe extern "efiapi" fn(u32, usize, *mut *mut u8) -> usize;
type FreePool = unsafe extern "efiapi" fn(*mut u8) -> usize;
type OutputString = unsafe extern "efiapi" fn(*mut SimpleTextOutput, *const u16) -> usize;
type Exit = unsafe extern "efiapi" fn(usize, usize, usize, *const u16) -> usize;

#[repr(C)]
pub struct BootServices {
    pub hdr: TableHeader,
    pub raise_tpl: usize,
    pub restore_tpl: usize,
    pub allocate_pages: usize,
    pub free_pages: usize,
    pub get_memory_map: usize,
    pub allocate_pool: AllocatePool,
    pub free_pool: FreePool,
    pub create_event: usize,
    pub set_timer: usize,
    pub wait_for_event: usize,
    pub signal_event: usize,
    pub close_event: usize,
    pub check_event: usize,
    pub install_protocol_interface: usize,
    pub reinstall_protocol_interface: usize,
    pub uninstall_protocol_interface: usize,
    pub handle_protocol: usize,
    pub reserved: usize,
    pub register_protocol_notify: usize,
    pub locate_handle: usize,
    pub locate_device_path: usize,
    pub install_configuration_table: usize,
    pub load_image: usize,
    pub start_image: usize,
    pub exit: Exit,
}

#[repr(C)]
pub struct SystemTable {
    pub hdr: TableHeader,
    pub firmware_vendor: *const u16,
    pub firmware_revision: u32,
    pub console_in_handle: usize,
    pub con_in: usize,
    pub console_out_handle: usize,
    pub con_out: *mut SimpleTextOutput,
    pub standard_error_handle: usize,
    pub std_err: *mut SimpleTextOutput,
    pub runtime_services: usize,
    pub boot_services: *mut BootServices,
    pub number_of_table_entries: usize,
    pub configuration_table: usize,
}

static mut SYSTEM_TABLE: *mut SystemTable = core::ptr::null_mut();
static mut IMAGE_HANDLE: usize = 0;
static mut SERIAL_READY: bool = false;

pub fn install_system_table(image_handle: usize, table: *mut SystemTable) {
    unsafe {
        SYSTEM_TABLE = table;
        IMAGE_HANDLE = image_handle;
    }
    init_serial();
}

pub fn boot_services() -> *mut BootServices {
    unsafe {
        let table = SYSTEM_TABLE;
        if table.is_null() {
            return core::ptr::null_mut();
        }
        (*table).boot_services
    }
}

pub fn image_handle() -> usize {
    unsafe { IMAGE_HANDLE }
}

const COM1: u16 = 0x3F8;

unsafe fn outb(port: u16, value: u8) {
    unsafe {
        asm!("out dx, al", in("dx") port, in("al") value, options(nomem, nostack, preserves_flags));
    }
}

unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    unsafe {
        asm!("in al, dx", out("al") value, in("dx") port, options(nomem, nostack, preserves_flags));
    }
    value
}

fn init_serial() {
    unsafe {
        outb(COM1 + 1, 0x00);
        outb(COM1 + 3, 0x80);
        outb(COM1 + 0, 0x01);
        outb(COM1 + 1, 0x00);
        outb(COM1 + 3, 0x03);
        outb(COM1 + 2, 0xC7);
        outb(COM1 + 4, 0x1E);
        SERIAL_READY = inb(COM1 + 4) == 0x1E;
        outb(COM1 + 4, SERIAL_READY as u8 * 0x0F);
    }
}

fn serial_write_byte(byte: u8) {
    unsafe {
        if !SERIAL_READY {
            return;
        }
        let mut spins = 0u32;
        while spins < 100_000 {
            if inb(COM1 + 5) & 0x20 != 0 {
                outb(COM1, byte);
                return;
            }
            spins += 1;
        }
        SERIAL_READY = false;
    }
}

fn serial_write(text: &[u8]) {
    for &byte in text {
        serial_write_byte(byte);
    }
}

fn con_out() -> *mut SimpleTextOutput {
    unsafe {
        let table = SYSTEM_TABLE;
        if table.is_null() {
            core::ptr::null_mut()
        } else {
            (*table).con_out
        }
    }
}

fn con_write(text: &str) {
    let out = con_out();
    if out.is_null() {
        return;
    }
    let output: OutputString = unsafe { core::mem::transmute((*out).output_string) };
    let mut buf = [0u16; 128];
    let mut used = 0usize;
    for ch in text.chars() {
        let mut encoded = [0u16; 2];
        let units: &[u16];
        if ch == '\n' {
            buf[used] = b'\r' as u16;
            used += 1;
            encoded[0] = b'\n' as u16;
            units = &encoded[..1];
        } else {
            let count = ch.encode_utf16(&mut encoded).len();
            units = &encoded[..count];
        }
        if used + units.len() + 1 > buf.len() {
            buf[used] = 0;
            unsafe { output(out, buf.as_ptr()) };
            used = 0;
        }
        buf[used..used + units.len()].copy_from_slice(units);
        used += units.len();
    }
    if used > 0 {
        buf[used] = 0;
        unsafe { output(out, buf.as_ptr()) };
    }
}

pub fn write_str(text: &str) {
    con_write(text);
    serial_write(text.as_bytes());
}

pub fn write_bytes(bytes: &[u8]) {
    let text = core::str::from_utf8(bytes).unwrap_or("?");
    write_str(text);
}

struct StackBuffer {
    data: [u8; 64],
    len: usize,
}

impl Write for StackBuffer {
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

pub fn format_i64(value: i64, out: &mut [u8; 32]) -> &[u8] {
    let mut buffer = StackBuffer {
        data: [0u8; 64],
        len: 0,
    };
    if write!(buffer, "{value}").is_err() {
        return b"?";
    }
    let mut n = 0;
    while n < buffer.len && n < out.len() {
        out[n] = buffer.data[n];
        n += 1;
    }
    &out[..n]
}

pub fn format_f64(value: f64, out: &mut [u8; 32]) -> &[u8] {
    let mut buffer = StackBuffer {
        data: [0u8; 64],
        len: 0,
    };
    if write!(buffer, "{value}").is_err() {
        return b"?";
    }
    let mut n = 0;
    while n < buffer.len && n < out.len() {
        out[n] = buffer.data[n];
        n += 1;
    }
    &out[..n]
}

pub fn exit_image(code: usize) -> ! {
    let table = unsafe { SYSTEM_TABLE };
    if !table.is_null() {
        let services = unsafe { (*table).boot_services };
        if !services.is_null() {
            let exit: Exit = unsafe { core::mem::transmute((*services).exit) };
            unsafe { exit(image_handle(), code, 0, core::ptr::null()) };
        }
    }
    loop {
        core::hint::spin_loop();
    }
}
