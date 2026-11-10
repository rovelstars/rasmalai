#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

use linker::pe::{PeInput, link};
use std::path::{Path, PathBuf};
use std::process::Command;

fn tool(name: &str) -> Option<PathBuf> {
    let out = Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {name}"))
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!path.is_empty()).then(|| PathBuf::from(path))
}

fn assemble(dir: &Path, name: &str, src: &str) -> PathBuf {
    let mc = tool("llvm-mc").expect("llvm-mc checked by caller");
    let source = dir.join(format!("{name}.s"));
    let object = dir.join(format!("{name}.obj"));
    std::fs::write(&source, src).unwrap();
    let out = Command::new(mc)
        .args(["-triple=x86_64-pc-windows-gnu", "-filetype=obj", "-o"])
        .arg(&object)
        .arg(&source)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "assemble {name}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    object
}

fn readobj(args: &[&str], file: &Path) -> String {
    let readobj = tool("llvm-readobj").expect("llvm-readobj checked by caller");
    let out = Command::new(readobj).args(args).arg(file).output().unwrap();
    assert!(
        out.status.success(),
        "llvm-readobj: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

struct Pe {
    machine: u16,
    subsystem: u16,
    magic: u16,
    entry: u32,
    sections: Vec<PeSection>,
    directories: Vec<(u32, u32)>,
}

struct PeSection {
    name: String,
    rva: u32,
    vsize: u32,
    raw_off: u32,
    chars: u32,
}

fn parse_pe(path: &Path) -> Pe {
    let bytes = std::fs::read(path).unwrap();
    let pe_at = u32::from_le_bytes([bytes[0x3c], bytes[0x3d], bytes[0x3e], bytes[0x3f]]) as usize;
    assert_eq!(&bytes[pe_at..pe_at + 4], b"PE\0\0");
    let machine = u16::from_le_bytes([bytes[pe_at + 4], bytes[pe_at + 5]]);
    let count = u16::from_le_bytes([bytes[pe_at + 6], bytes[pe_at + 7]]) as usize;
    let opt_size = u16::from_le_bytes([bytes[pe_at + 20], bytes[pe_at + 21]]) as usize;
    let opt = pe_at + 24;
    let magic = u16::from_le_bytes([bytes[opt], bytes[opt + 1]]);
    let entry = u32::from_le_bytes(bytes[opt + 16..opt + 20].try_into().unwrap());
    let subsystem = u16::from_le_bytes([bytes[opt + 68], bytes[opt + 69]]);
    let dir_count = u32::from_le_bytes(bytes[opt + 108..opt + 112].try_into().unwrap()) as usize;
    let mut directories = Vec::new();
    for i in 0..dir_count.min(16) {
        let at = opt + 112 + i * 8;
        directories.push((
            u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()),
            u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()),
        ));
    }
    let table = opt + opt_size;
    let mut sections = Vec::new();
    for i in 0..count {
        let at = table + i * 40;
        let raw = &bytes[at..at + 40];
        let end = raw[0..8].iter().position(|&b| b == 0).unwrap_or(8);
        sections.push(PeSection {
            name: String::from_utf8_lossy(&raw[..end]).into_owned(),
            vsize: u32::from_le_bytes(raw[8..12].try_into().unwrap()),
            rva: u32::from_le_bytes(raw[12..16].try_into().unwrap()),
            raw_off: u32::from_le_bytes(raw[20..24].try_into().unwrap()),
            chars: u32::from_le_bytes(raw[36..40].try_into().unwrap()),
        });
    }
    Pe {
        machine,
        subsystem,
        magic,
        entry,
        sections,
        directories,
    }
}

fn section<'a>(pe: &'a Pe, name: &str) -> &'a PeSection {
    pe.sections
        .iter()
        .find(|section| section.name == name)
        .unwrap_or_else(|| panic!("missing section {name}"))
}

fn dlltool(dir: &Path, dll: &str, exports: &[&str]) -> PathBuf {
    let dlltool = tool("llvm-dlltool").expect("llvm-dlltool checked by caller");
    let mut def = format!("LIBRARY {dll}\nEXPORTS\n");
    for export in exports {
        def.push_str(export);
        def.push('\n');
    }
    let def_path = dir.join(format!("{dll}.def"));
    std::fs::write(&def_path, def).unwrap();
    let lib = dir.join(format!("{dll}.lib"));
    let out = Command::new(dlltool)
        .args(["-m", "i386:x86-64", "-d"])
        .arg(&def_path)
        .arg("-l")
        .arg(&lib)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "dlltool: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    lib
}

const EXIT: &str = r#"
.intel_syntax noprefix
.text
.globl start
start:
    ret
.data
.globl ptr
ptr:
    .quad start
"#;

#[test]
fn minimal_pe32_plus_with_base_relocations() {
    let Some(_) = tool("llvm-mc") else {
        return eprintln!("skipping: llvm-mc is absent");
    };
    let Some(_) = tool("llvm-readobj") else {
        return eprintln!("skipping: llvm-readobj is absent");
    };
    let dir = crate::scratch::dir("pe-min");
    let object = assemble(&dir, "min", EXIT);
    let exe = dir.join("min.exe");
    let stats = link(&[PeInput::ObjectPath(object)], &exe, "start", 3).unwrap();
    assert_eq!(stats.imports, 0);
    let pe = parse_pe(&exe);
    assert_eq!(pe.machine, 0x8664);
    assert_eq!(pe.magic, 0x20b);
    assert_eq!(pe.subsystem, 3);
    let text = section(&pe, ".text");
    assert!(
        pe.entry >= text.rva && pe.entry < text.rva + text.vsize,
        "entry {:#x} outside .text",
        pe.entry
    );
    assert_ne!(text.chars & 0x2000_0000, 0, ".text is not executable");
    assert_ne!(text.chars & 0x20, 0, ".text is not code");
    let data = section(&pe, ".data");
    assert_ne!(data.chars & 0x8000_0000, 0, ".data is not writable");
    assert_ne!(pe.directories[5].0, 0, "no base relocation directory");
    let report = readobj(&["--coff-basereloc"], &exe);
    assert!(report.contains("DIR64"), "{report}");
}

#[test]
fn import_directory_names_exit_process() {
    let Some(_) = tool("llvm-mc") else {
        return eprintln!("skipping: llvm-mc is absent");
    };
    let Some(_) = tool("llvm-dlltool") else {
        return eprintln!("skipping: llvm-dlltool is absent");
    };
    let dir = crate::scratch::dir("pe-import");
    let lib = dlltool(&dir, "kernel32.dll", &["ExitProcess"]);
    let src = r#"
.intel_syntax noprefix
.text
.globl start
start:
    sub rsp, 40
    mov ecx, 0
    call ExitProcess
    add rsp, 40
    ret
"#;
    let object = assemble(&dir, "import", src);
    let exe = dir.join("import.exe");
    let stats = link(
        &[PeInput::ObjectPath(object), PeInput::LibraryPath(lib)],
        &exe,
        "start",
        3,
    )
    .unwrap();
    assert_eq!(stats.imports, 1);
    assert_ne!(parse_pe(&exe).directories[1].0, 0, "no import directory");
    let report = readobj(&["--coff-imports"], &exe);
    assert!(report.contains("Symbol: ExitProcess"), "{report}");
    assert!(report.contains("kernel32.dll"), "{report}");
}

#[test]
fn import_through_the_iat_symbol() {
    let Some(_) = tool("llvm-mc") else {
        return eprintln!("skipping: llvm-mc is absent");
    };
    let Some(_) = tool("llvm-dlltool") else {
        return eprintln!("skipping: llvm-dlltool is absent");
    };
    let dir = crate::scratch::dir("pe-imp-slot");
    let lib = dlltool(&dir, "kernel32.dll", &["ExitProcess"]);
    let src = r#"
.intel_syntax noprefix
.text
.globl start
start:
    call qword ptr [rip + __imp_ExitProcess]
    ret
"#;
    let object = assemble(&dir, "slot", src);
    let exe = dir.join("slot.exe");
    link(
        &[PeInput::ObjectPath(object), PeInput::LibraryPath(lib)],
        &exe,
        "start",
        3,
    )
    .unwrap();
    let report = readobj(&["--coff-imports"], &exe);
    assert!(report.contains("Symbol: ExitProcess"), "{report}");
}

#[test]
fn pdata_and_xdata_are_preserved() {
    let Some(_) = tool("llvm-mc") else {
        return eprintln!("skipping: llvm-mc is absent");
    };
    let Some(_) = tool("llvm-readobj") else {
        return eprintln!("skipping: llvm-readobj is absent");
    };
    let dir = crate::scratch::dir("pe-unwind");
    let src = r#"
.intel_syntax noprefix
.text
.globl start
start:
    ret
.section .pdata,"dr"
.globl pdata_entry
pdata_entry:
    .long start@IMGREL
    .long start@IMGREL
    .long 0
.section .xdata,"dr"
.globl unwind_blob
unwind_blob:
    .byte 1, 0, 0, 0
"#;
    let object = assemble(&dir, "unwind", src);
    let exe = dir.join("unwind.exe");
    link(&[PeInput::ObjectPath(object)], &exe, "start", 3).unwrap();
    let pe = parse_pe(&exe);
    let pdata = section(&pe, ".pdata");
    let xdata = section(&pe, ".xdata");
    assert!(pdata.vsize >= 12, "pdata missing");
    assert!(xdata.vsize >= 4, "xdata missing");
    assert_ne!(pe.directories[3].0, 0, "no exception directory");
    let bytes = std::fs::read(&exe).unwrap();
    let at = pdata.raw_off as usize;
    let function_rva = u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
    assert_eq!(function_rva, section(&pe, ".text").rva);
}

#[test]
fn two_links_are_byte_identical() {
    let Some(_) = tool("llvm-mc") else {
        return eprintln!("skipping: llvm-mc is absent");
    };
    let dir = crate::scratch::dir("pe-det");
    let object = assemble(&dir, "det", EXIT);
    let first = dir.join("first.exe");
    let second = dir.join("second.exe");
    let bytes = std::fs::read(&object).unwrap();
    link(&[PeInput::Object(bytes.clone())], &first, "start", 3).unwrap();
    link(&[PeInput::Object(bytes)], &second, "start", 3).unwrap();
    assert_eq!(
        std::fs::read(&first).unwrap(),
        std::fs::read(&second).unwrap(),
        "two PE links differ"
    );
}

#[test]
fn differential_against_lld_link() {
    let Some(_) = tool("llvm-mc") else {
        return eprintln!("skipping: llvm-mc is absent");
    };
    let Some(_) = tool("lld-link") else {
        return eprintln!("skipping: lld-link is absent");
    };
    let dir = crate::scratch::dir("pe-diff");
    let object = assemble(&dir, "diff", EXIT);
    let ours = dir.join("ours.exe");
    link(&[PeInput::ObjectPath(object.clone())], &ours, "start", 3).unwrap();
    let theirs = dir.join("theirs.exe");
    let out = Command::new(tool("lld-link").unwrap())
        .arg(format!("/entry:start"))
        .arg("/subsystem:console")
        .arg("/nodefaultlib")
        .arg(format!("/out:{}", theirs.display()))
        .arg(&object)
        .output()
        .unwrap();
    if !out.status.success() {
        eprintln!(
            "skipping differential: lld-link rejected the fixture: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        return;
    }
    let a = parse_pe(&ours);
    let b = parse_pe(&theirs);
    assert_eq!(a.machine, b.machine);
    assert_eq!(a.magic, b.magic);
    assert_eq!(a.subsystem, b.subsystem);
    let names = |pe: &Pe| -> Vec<(String, bool, bool, bool)> {
        pe.sections
            .iter()
            .map(|section| {
                (
                    section.name.clone(),
                    section.chars & 0x20 != 0,
                    section.chars & 0x2000_0000 != 0,
                    section.chars & 0x8000_0000 != 0,
                )
            })
            .collect()
    };
    assert_eq!(names(&a), names(&b));
    assert_ne!(a.entry, 0);
    assert_ne!(b.entry, 0);
    let ours_report = readobj(&["--coff-basereloc"], &ours);
    let theirs_report = readobj(&["--coff-basereloc"], &theirs);
    assert!(ours_report.contains("DIR64"));
    assert!(theirs_report.contains("DIR64"));
}

const UEFI_ENTRY: &str = r#"
.intel_syntax noprefix
.text
.globl efi_main
efi_main:
    xor eax, eax
    ret
"#;

#[test]
fn uefi_image_is_subsystem_10_at_efi_main() {
    let Some(_) = tool("llvm-mc") else {
        return eprintln!("skipping: llvm-mc is absent");
    };
    let Some(_) = tool("llvm-readobj") else {
        return eprintln!("skipping: llvm-readobj is absent");
    };
    let dir = crate::scratch::dir("pe-uefi");
    let object = assemble(&dir, "uefi", UEFI_ENTRY);
    let exe = dir.join("bootx64.efi");
    let stats = link(&[PeInput::ObjectPath(object)], &exe, "efi_main", 10).unwrap();
    assert_eq!(stats.imports, 0);
    let pe = parse_pe(&exe);
    assert_eq!(pe.machine, 0x8664);
    assert_eq!(pe.magic, 0x20b);
    assert_eq!(pe.subsystem, 10);
    let text = section(&pe, ".text");
    assert_eq!(pe.entry, text.rva, "entry must be the first efi_main byte");
    assert!(
        pe.sections.iter().all(|section| section.name != ".idata"),
        "a UEFI image with no imports must not carry an import directory"
    );
    assert_eq!(pe.directories[1].0, 0, "import directory must be empty");
    let bytes = std::fs::read(&exe).unwrap();
    let at = (text.raw_off + (pe.entry - text.rva)) as usize;
    assert_eq!(&bytes[at..at + 3], &[0x31, 0xc0, 0xc3], "entry code");
    let report = readobj(&["--file-headers"], &exe);
    assert!(report.contains("IMAGE_SUBSYSTEM_EFI_APPLICATION"), "{report}");
}

#[test]
fn missing_entry_symbol_names_itself() {
    let Some(_) = tool("llvm-mc") else {
        return eprintln!("skipping: llvm-mc is absent");
    };
    let dir = crate::scratch::dir("pe-uefi-missing");
    let object = assemble(
        &dir,
        "noentry",
        ".intel_syntax noprefix\n.text\n.globl start\nstart:\n    ret\n",
    );
    let exe = dir.join("empty.efi");
    let err = link(&[PeInput::ObjectPath(object)], &exe, "efi_main", 10)
        .expect_err("efi_main is not defined");
    assert!(err.to_string().contains("efi_main"), "{err}");
}

#[test]
fn archive_members_are_pulled_only_when_needed() {
    let Some(_) = tool("llvm-mc") else {
        return eprintln!("skipping: llvm-mc is absent");
    };
    let Some(_) = tool("llvm-ar") else {
        return eprintln!("skipping: llvm-ar is absent");
    };
    let dir = crate::scratch::dir("pe-pull");
    let entry = assemble(&dir, "entry", UEFI_ENTRY);
    let helper = assemble(
        &dir,
        "helper",
        ".intel_syntax noprefix\n.text\n.globl helper\nhelper:\n    ret\n",
    );
    let unused = assemble(
        &dir,
        "unused",
        ".intel_syntax noprefix\n.text\n.globl helper\nhelper:\n    mov eax, 1\n    ret\n",
    );
    let lib = dir.join("libextra.a");
    let out = Command::new(tool("llvm-ar").unwrap())
        .arg("cr")
        .arg(&lib)
        .arg(&helper)
        .arg(&unused)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let exe = dir.join("pulled.efi");
    link(
        &[PeInput::ObjectPath(entry), PeInput::LibraryPath(lib)],
        &exe,
        "efi_main",
        10,
    )
    .expect("the duplicate helper in the unused member must not be pulled");
}
