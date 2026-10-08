pub mod machine;
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
}
