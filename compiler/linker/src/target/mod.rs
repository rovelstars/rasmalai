pub mod aarch64;
pub mod coff;
pub mod elf;
pub mod macho;
pub mod x86_64;

use crate::LinkError;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SymClass {
    Local,
    Ifunc,
    Tls,
    DynamicFunc,
    DynamicData,
    WeakUndef,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Plan {
    Direct,
    Got,
    Plt,
    TlsGd,
    TlsLd,
    TpoffRelax,
    IFuncGot,
    IFuncPlt,
    Skip,
}

#[derive(Clone, Copy, Debug)]
pub struct ApplyCtx {
    pub sym_addr: u64,
    pub place_addr: u64,
    pub slot_addr: u64,
    pub addend: i64,
    pub tpoff: i64,
    pub dtpoff: u64,
}

pub trait Target: Sync {
    fn machine(&self) -> u16;
    fn page_size(&self) -> u64;
    fn is_static(&self) -> bool;
    fn plan(&self, kind: u32, sym: SymClass) -> Result<Plan, LinkError>;
    fn apply(&self, kind: u32, plan: Plan, ctx: &ApplyCtx, bytes: &mut [u8], field: usize) -> Result<(), LinkError>;
    fn plt_entry(&self, entry_addr: u64, slot_addr: u64, index: u32, plt0_addr: u64) -> [u8; 16];
    fn iplt_entry(&self, entry_addr: u64, slot_addr: u64) -> [u8; 16];
    fn rel_jmpslot(&self) -> u32;
    fn rel_globdat(&self) -> u32;
    fn rel_relative(&self) -> u32;
    fn rel_irelative(&self) -> u32;
    fn rel_copy(&self) -> u32;
    fn global_offset_label(&self) -> &'static str;
}
