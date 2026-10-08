use super::{ApplyCtx, Plan, SymClass, Target};
use crate::LinkError;

pub struct Aarch64;

fn unimplemented() -> LinkError {
    LinkError::Native("native backend not yet implemented for aarch64".to_string())
}

impl Target for Aarch64 {
    fn machine(&self) -> u16 {
        183
    }
    fn page_size(&self) -> u64 {
        0x1000
    }
    fn is_static(&self) -> bool {
        true
    }
    fn global_offset_label(&self) -> &'static str {
        "_GLOBAL_OFFSET_TABLE_"
    }
    fn plan(&self, _kind: u32, _sym: SymClass) -> Result<Plan, LinkError> {
        Err(unimplemented())
    }
    fn apply(&self, _kind: u32, _plan: Plan, _ctx: &ApplyCtx, _bytes: &mut [u8], _field: usize) -> Result<(), LinkError> {
        Err(unimplemented())
    }
    fn plt_entry(&self, _entry_addr: u64, _slot_addr: u64, _index: u32, _plt0_addr: u64) -> [u8; 16] {
        [0u8; 16]
    }
    fn iplt_entry(&self, _entry_addr: u64, _slot_addr: u64) -> [u8; 16] {
        [0u8; 16]
    }
    fn rel_jmpslot(&self) -> u32 {
        0
    }
    fn rel_globdat(&self) -> u32 {
        0
    }
    fn rel_relative(&self) -> u32 {
        0
    }
    fn rel_irelative(&self) -> u32 {
        0
    }
    fn rel_copy(&self) -> u32 {
        0
    }
}
