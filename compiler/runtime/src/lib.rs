#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

pub mod machine;
pub mod comptime;
pub mod native;
pub use native::guard;
pub mod lir_pipeline;
pub mod stdlib_cache;
pub mod threads;
pub mod value;
pub mod ichan;

pub mod archive {
    pub static BYTES: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/libruntime_native.a"));
    pub const SHA256: &str = env!("RNX_RUNTIME_ARCHIVE_SHA256");

    const fn abi_value(text: &str) -> u32 {
        let bytes = text.as_bytes();
        let mut value: u32 = 0;
        let mut i = 0;
        while i < bytes.len() {
            let digit = (bytes[i] as u32).wrapping_sub(b'0' as u32);
            value = value * 10 + digit;
            i += 1;
        }
        value
    }

    pub const ABI: u32 = abi_value(env!("RNX_RUNTIME_ARCHIVE_ABI"));
}
