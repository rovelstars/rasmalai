pub const ET_EXEC: u16 = 2;
pub const ET_DYN: u16 = 3;

pub const PT_NULL: u32 = 0;
pub const PT_LOAD: u32 = 1;
pub const PT_DYNAMIC: u32 = 2;
pub const PT_INTERP: u32 = 3;
pub const PT_NOTE: u32 = 4;
pub const PT_PHDR: u32 = 6;
pub const PT_TLS: u32 = 7;
pub const PT_GNU_EH_FRAME: u32 = 0x6474e550;
pub const PT_GNU_STACK: u32 = 0x6474e551;
pub const PT_GNU_RELRO: u32 = 0x6474e552;
pub const PT_GNU_PROPERTY: u32 = 0x6474e553;

pub const PF_X: u32 = 1;
pub const PF_W: u32 = 2;
pub const PF_R: u32 = 4;

pub const DT_NULL: u64 = 0;
pub const DT_NEEDED: u64 = 1;
pub const DT_PLTRELSZ: u64 = 2;
pub const DT_PLTGOT: u64 = 3;
pub const DT_HASH: u64 = 4;
pub const DT_STRTAB: u64 = 5;
pub const DT_SYMTAB: u64 = 6;
pub const DT_RELA: u64 = 7;
pub const DT_RELASZ: u64 = 8;
pub const DT_RELAENT: u64 = 9;
pub const DT_STRSZ: u64 = 10;
pub const DT_SYMENT: u64 = 11;
pub const DT_INIT: u64 = 12;
pub const DT_FINI: u64 = 13;
pub const DT_SONAME: u64 = 14;
pub const DT_RPATH: u64 = 15;
pub const DT_SYMBOLIC: u64 = 16;
pub const DT_REL: u64 = 17;
pub const DT_RELSZ: u64 = 18;
pub const DT_RELENT: u64 = 19;
pub const DT_PLTREL: u64 = 20;
pub const DT_JMPREL: u64 = 23;
pub const DT_INIT_ARRAY: u64 = 25;
pub const DT_FINI_ARRAY: u64 = 26;
pub const DT_INIT_ARRAYSZ: u64 = 27;
pub const DT_FINI_ARRAYSZ: u64 = 28;
pub const DT_RUNPATH: u64 = 29;
pub const DT_FLAGS: u64 = 30;
pub const DT_PREINIT_ARRAY: u64 = 32;
pub const DT_PREINIT_ARRAYSZ: u64 = 33;
pub const DT_GNU_HASH: u64 = 0x6ffffef5;
pub const DT_VERSYM: u64 = 0x6ffffff0;
pub const DT_VERNEED: u64 = 0x6ffffffe;
pub const DT_VERNEEDNUM: u64 = 0x6fffffff;
pub const DT_FLAGS_1: u64 = 0x6ffffffb;
pub const DT_RELACOUNT: u64 = 0x6ffffff9;
pub const DT_DEBUG: u64 = 21;
pub const DF_BIND_NOW: u64 = 0x8;
pub const DF_1_NOW: u64 = 0x1;
pub const DF_1_PIE: u64 = 0x08000000;

pub fn write_ehdr(out: &mut [u8], exec: bool, machine: u16, entry: u64, phoff: u64, shoff: u64, phnum: u16, shnum: u16, shstrndx: u16, osabi: u8) {
    out[0..16].copy_from_slice(&[0x7f, b'E', b'L', b'F', 2, 1, 1, osabi, 0, 0, 0, 0, 0, 0, 0, 0]);
    out[16..18].copy_from_slice(&(if exec { ET_EXEC } else { ET_DYN }).to_le_bytes());
    out[18..20].copy_from_slice(&machine.to_le_bytes());
    out[20..24].copy_from_slice(&1u32.to_le_bytes());
    out[24..32].copy_from_slice(&entry.to_le_bytes());
    out[32..40].copy_from_slice(&phoff.to_le_bytes());
    out[40..48].copy_from_slice(&shoff.to_le_bytes());
    out[48..52].copy_from_slice(&0u32.to_le_bytes());
    out[52..54].copy_from_slice(&64u16.to_le_bytes());
    out[54..56].copy_from_slice(&56u16.to_le_bytes());
    out[56..58].copy_from_slice(&phnum.to_le_bytes());
    out[58..60].copy_from_slice(&64u16.to_le_bytes());
    out[60..62].copy_from_slice(&shnum.to_le_bytes());
    out[62..64].copy_from_slice(&shstrndx.to_le_bytes());
}

pub fn write_phdr(out: &mut [u8], kind: u32, flags: u32, offset: u64, vaddr: u64, filesz: u64, memsz: u64, align: u64) {
    out[0..4].copy_from_slice(&kind.to_le_bytes());
    out[4..8].copy_from_slice(&flags.to_le_bytes());
    out[8..16].copy_from_slice(&offset.to_le_bytes());
    out[16..24].copy_from_slice(&vaddr.to_le_bytes());
    out[24..32].copy_from_slice(&vaddr.to_le_bytes());
    out[32..40].copy_from_slice(&filesz.to_le_bytes());
    out[40..48].copy_from_slice(&memsz.to_le_bytes());
    out[48..56].copy_from_slice(&align.to_le_bytes());
}

pub fn write_shdr(
    out: &mut [u8],
    name: u32,
    kind: u32,
    flags: u64,
    addr: u64,
    offset: u64,
    size: u64,
    link: u32,
    info: u32,
    align: u64,
    entsize: u64,
) {
    out[0..4].copy_from_slice(&name.to_le_bytes());
    out[4..8].copy_from_slice(&kind.to_le_bytes());
    out[8..16].copy_from_slice(&flags.to_le_bytes());
    out[16..24].copy_from_slice(&addr.to_le_bytes());
    out[24..32].copy_from_slice(&offset.to_le_bytes());
    out[32..40].copy_from_slice(&size.to_le_bytes());
    out[40..44].copy_from_slice(&link.to_le_bytes());
    out[44..48].copy_from_slice(&info.to_le_bytes());
    out[48..56].copy_from_slice(&align.to_le_bytes());
    out[56..64].copy_from_slice(&entsize.to_le_bytes());
}

pub fn write_sym(out: &mut [u8], name: u32, info: u8, other: u8, shndx: u16, value: u64, size: u64) {
    out[0..4].copy_from_slice(&name.to_le_bytes());
    out[4] = info;
    out[5] = other;
    out[6..8].copy_from_slice(&shndx.to_le_bytes());
    out[8..16].copy_from_slice(&value.to_le_bytes());
    out[16..24].copy_from_slice(&size.to_le_bytes());
}

pub fn write_rela(out: &mut [u8], offset: u64, kind: u32, sym: u32, addend: i64) {
    out[0..8].copy_from_slice(&offset.to_le_bytes());
    out[8..12].copy_from_slice(&kind.to_le_bytes());
    out[12..16].copy_from_slice(&sym.to_le_bytes());
    out[16..24].copy_from_slice(&(addend as u64).to_le_bytes());
}

pub fn write_dyn(out: &mut [u8], tag: i64, val: u64) {
    out[0..8].copy_from_slice(&(tag as u64).to_le_bytes());
    out[8..16].copy_from_slice(&val.to_le_bytes());
}

pub fn elf_hash(name: &[u8]) -> u32 {
    let mut h = 0u32;
    for &b in name {
        h = h.wrapping_mul(16).wrapping_add(b as u32);
        let g = h & 0xf0000000;
        if g != 0 {
            h ^= g >> 24;
        }
        h &= !g;
    }
    h
}

pub fn sysv_hash(names: &[Vec<u8>]) -> Vec<u8> {
    let nsym = names.len();
    let nbucket = (nsym + 1).max(1);
    let nchain = nsym.max(1);
    let mut buckets = vec![0u32; nbucket];
    let mut chains = vec![0u32; nchain];
    for i in 1..nsym {
        let h = elf_hash(&names[i]);
        let b = (h as usize) % nbucket;
        chains[i] = buckets[b];
        buckets[b] = i as u32;
    }
    let mut out = Vec::with_capacity(8 + 4 * (nbucket + nchain));
    out.extend_from_slice(&(nbucket as u32).to_le_bytes());
    out.extend_from_slice(&(nchain as u32).to_le_bytes());
    for b in buckets {
        out.extend_from_slice(&b.to_le_bytes());
    }
    for c in chains {
        out.extend_from_slice(&c.to_le_bytes());
    }
    out
}

pub fn hash_size(nsym: usize) -> u64 {
    let nbucket = (nsym + 1).max(1);
    let nchain = nsym.max(1);
    (8 + 4 * (nbucket + nchain)) as u64
}
