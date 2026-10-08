use linker::{LinkInput, native};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

fn fixture_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rnx-native-diff-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_src(dir: &Path, name: &str, src: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, src).unwrap();
    path
}

fn build_objects(path: &Path, release: bool, jobs: usize) -> Vec<Vec<u8>> {
    let mut cfg = cli::CompileConfig::default();
    cfg.jobs = jobs;
    cfg.memory_cap = u64::MAX;
    cli::build_files_staged_cfg(path.to_str().unwrap(), "Main", release, 1, None, &cfg)
        .unwrap_or_else(|e| panic!("build {}: {e:?}", path.display()))
        .objects
}

fn runtime_so_dir() -> PathBuf {
    for dir in ["target/debug", "target/release"] {
        let mut root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        root.pop();
        root.pop();
        let cand = root.join(dir);
        if cand.join("libruntime_native.so").is_file() {
            return cand;
        }
    }
    let exe = std::env::current_exe().unwrap();
    let mut dir = exe.parent().unwrap().to_path_buf();
    loop {
        if dir.join("libruntime_native.so").is_file() {
            return dir;
        }
        if !dir.pop() {
            break;
        }
    }
    panic!("libruntime_native.so not found");
}

fn link_native_static(objects: &[Vec<u8>], out: &Path) {
    let mut inputs: Vec<LinkInput> = objects.iter().map(|b| LinkInput::ObjectBytes(b.clone())).collect();
    inputs.push(LinkInput::ArchiveBytes(runtime::archive::BYTES.to_vec()));
    for lib in ["pthread", "dl", "m", "c"] {
        inputs.push(LinkInput::Lib(lib.to_string()));
    }
    native::native_link_static(&inputs, out, &native::StaticOpts::default()).unwrap();
}

fn link_native_fully_static(objects: &[Vec<u8>], out: &Path) {
    let mut inputs: Vec<LinkInput> = objects.iter().map(|b| LinkInput::ObjectBytes(b.clone())).collect();
    inputs.push(LinkInput::ArchiveBytes(runtime::archive::BYTES.to_vec()));
    for lib in ["pthread", "dl", "m", "c"] {
        inputs.push(LinkInput::Lib(lib.to_string()));
    }
    native::native_link_fully_static(&inputs, out, &native::StaticOpts::default()).unwrap();
}

fn link_native_static_pie(objects: &[Vec<u8>], out: &Path) {
    let mut inputs: Vec<LinkInput> = objects.iter().map(|b| LinkInput::ObjectBytes(b.clone())).collect();
    inputs.push(LinkInput::ArchiveBytes(runtime::archive::BYTES.to_vec()));
    for lib in ["pthread", "dl", "m", "c"] {
        inputs.push(LinkInput::Lib(lib.to_string()));
    }
    native::native_link_static_pie(&inputs, out, &native::StaticOpts::default()).unwrap();
}

fn link_native_dynamic(objects: &[Vec<u8>], out: &Path, so_dir: &Path) {
    let inputs: Vec<LinkInput> = objects.iter().map(|b| LinkInput::ObjectBytes(b.clone())).collect();
    let opts = native::DynamicOpts {
        entry: "_start".to_string(),
        icf: true,
        target: "x86_64-unknown-linux-gnu".to_string(),
        lib_dirs: vec![so_dir.to_path_buf()],
        libs: vec!["runtime_native".to_string()],
        runpath: Some(so_dir.to_string_lossy().into_owned()),
        strip: false,
    };
    native::native_link_dynamic(&inputs, out, &opts).unwrap();
}

fn link_ref_dynamic(objects: &[Vec<u8>], out: &Path, dir: &Path) {
    let mut obj_paths = Vec::new();
    for (i, bytes) in objects.iter().enumerate() {
        let p = dir.join(format!("ref{i}.o"));
        std::fs::write(&p, bytes).unwrap();
        obj_paths.push(p);
    }
    let rt_a = dir.join("libruntime_native.a");
    std::fs::write(&rt_a, runtime::archive::BYTES).unwrap();
    let status = Command::new("cc")
        .arg("-fuse-ld=lld")
        .arg("-Wl,--gc-sections")
        .args(&obj_paths)
        .arg(&rt_a)
        .args(["-lpthread", "-ldl", "-lm", "-lc", "-o"])
        .arg(out)
        .output()
        .unwrap();
    assert!(status.status.success(), "ref link failed: {}", String::from_utf8_lossy(&status.stderr));
}

fn link_ref_fully_static(objects: &[Vec<u8>], out: &Path, dir: &Path) {
    let mut obj_paths = Vec::new();
    for (i, bytes) in objects.iter().enumerate() {
        let p = dir.join(format!("ref{i}.o"));
        std::fs::write(&p, bytes).unwrap();
        obj_paths.push(p);
    }
    let rt_a = dir.join("libruntime_native.a");
    std::fs::write(&rt_a, runtime::archive::BYTES).unwrap();
    let status = Command::new("cc")
        .arg("-static")
        .arg("-fuse-ld=lld")
        .arg("-Wl,--gc-sections")
        .args(&obj_paths)
        .arg(&rt_a)
        .args(["-lpthread", "-ldl", "-lm", "-lc", "-o"])
        .arg(out)
        .output()
        .unwrap();
    assert!(status.status.success(), "ref link failed: {}", String::from_utf8_lossy(&status.stderr));
}

fn link_ref_static_pie(objects: &[Vec<u8>], out: &Path, dir: &Path) {
    let mut obj_paths = Vec::new();
    for (i, bytes) in objects.iter().enumerate() {
        let p = dir.join(format!("ref{i}.o"));
        std::fs::write(&p, bytes).unwrap();
        obj_paths.push(p);
    }
    let rt_a = dir.join("libruntime_native.a");
    std::fs::write(&rt_a, runtime::archive::BYTES).unwrap();
    let status = Command::new("cc")
        .arg("-static-pie")
        .arg("-fuse-ld=lld")
        .arg("-Wl,--gc-sections")
        .args(&obj_paths)
        .arg(&rt_a)
        .args(["-lpthread", "-ldl", "-lm", "-lc", "-o"])
        .arg(out)
        .output()
        .unwrap();
    assert!(status.status.success(), "ref link failed: {}", String::from_utf8_lossy(&status.stderr));
}

fn run_bin(bin: &Path) -> (i32, String) {
    let out = Command::new(bin).output().unwrap();
    (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).into_owned())
}

fn read_sections(bin: &Path) -> Vec<(String, String, u64, u64)> {
    let out = Command::new("readelf").args(["-SW", &bin.to_string_lossy()]).output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    let mut secs = Vec::new();
    for line in text.lines().skip(3) {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 8 {
            continue;
        }
        secs.push((parts[1].to_string(), parts[2].to_string(), 0, 0));
    }
    secs
}

fn global_syms(bin: &Path, defined: bool) -> BTreeSet<(String, String)> {
    let out = Command::new("readelf").args(["-sW", &bin.to_string_lossy()]).output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    let mut set = BTreeSet::new();
    for line in text.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 8 {
            continue;
        }
        let (typ, bind, ndx, name) = (parts[3], parts[4], parts[6], parts[7].split('@').next().unwrap_or(""));
        if bind != "GLOBAL" && bind != "WEAK" {
            continue;
        }
        let is_def = ndx != "UND";
        if is_def != defined {
            continue;
        }
        set.insert((name.to_string(), typ.to_string()));
    }
    set
}

fn check_permissions(bin: &Path) {
    let out = Command::new("readelf").args(["-SW", &bin.to_string_lossy()]).output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines().skip(3) {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 8 {
            continue;
        }
        let (name, flags) = (parts[1], parts[7]);
        assert!(!(flags.contains('W') && flags.contains('X')), "{bin:?} section {name} is W+X");
        if name == ".text" {
            assert!(flags.contains('X') && !flags.contains('W'), "{bin:?} .text flags {flags}");
        }
    }
}

fn elf_type(bin: &Path) -> u16 {
    let bytes = std::fs::read(bin).unwrap();
    u16::from_le_bytes(bytes[16..18].try_into().unwrap())
}

fn ldd_libs(bin: &Path) -> String {
    let out = Command::new("ldd").arg(bin).output().unwrap();
    assert!(out.status.success(), "ldd failed on {bin:?}");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn dyn_syms(bin: &Path) -> (BTreeSet<(String, String)>, BTreeSet<(String, String)>) {
    let out = Command::new("readelf").args(["--dyn-syms", "-W", &bin.to_string_lossy()]).output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    let (mut def, mut und) = (BTreeSet::new(), BTreeSet::new());
    for line in text.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 8 {
            continue;
        }
        let (typ, bind, ndx, name) = (parts[3], parts[4], parts[6], parts[7].split('@').next().unwrap_or(""));
        if bind != "GLOBAL" && bind != "WEAK" {
            continue;
        }
        if name.is_empty() {
            continue;
        }
        if ndx == "UND" {
            und.insert((name.to_string(), typ.to_string()));
        } else {
            def.insert((name.to_string(), typ.to_string()));
        }
    }
    (def, und)
}

fn has_segment(bin: &Path, want: &str) -> bool {
    let out = Command::new("readelf").args(["-lW", &bin.to_string_lossy()]).output().unwrap();
    assert!(out.status.success());
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .any(|l| l.split_whitespace().next() == Some(want))
}

fn dynamic_tag(bin: &Path, tag: &str) -> Option<String> {
    let out = Command::new("readelf").args(["-dW", &bin.to_string_lossy()]).output().unwrap();
    assert!(out.status.success());
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find(|l| l.contains(&format!("({tag})")))
        .map(|l| l.trim().to_string())
}

fn rela_dyn_types(bin: &Path) -> Vec<String> {
    let out = Command::new("readelf").args(["-rW", &bin.to_string_lossy()]).output().unwrap();
    assert!(out.status.success());
    let mut in_dyn = false;
    let mut types = Vec::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        if line.contains("Relocation section") {
            if in_dyn {
                break;
            }
            in_dyn = line.contains("'.rela.dyn'");
            continue;
        }
        if in_dyn {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 3 && !parts[0].is_empty() && parts[0].chars().all(|c| c.is_ascii_hexdigit()) {
                types.push(parts[2].to_string());
            }
        }
    }
    types
}

fn check_fixture(tag: &str, files: &[(&str, &str)], release: bool, jobs: usize) {
    let dir = fixture_dir(tag);
    let mut paths = Vec::new();
    for (name, src) in files {
        paths.push(write_src(&dir, name, src));
    }
    let mut objects = Vec::new();
    for path in paths.iter() {
        let mut objs = build_objects(path, release, jobs);
        objects.append(&mut objs);
    }
    if jobs > 1 {
        assert!(objects.len() > 1, "{tag}: expected multiple codegen objects, got {}", objects.len());
    }
    let so_dir = runtime_so_dir();
    let native_st = dir.join(format!("{tag}_native_static"));
    let native_dy = dir.join(format!("{tag}_native_dynamic"));
    let ref_st = dir.join(format!("{tag}_ref_static"));
    link_native_static(&objects, &native_st);
    link_native_dynamic(&objects, &native_dy, &so_dir);
    link_ref_dynamic(&objects, &ref_st, &dir);
    let (code_st, out_st) = run_bin(&native_st);
    let (code_dy, out_dy) = run_bin(&native_dy);
    let (code_rf, out_rf) = run_bin(&ref_st);
    assert_eq!((code_st, out_st.clone()), (code_rf, out_rf.clone()), "{tag}: native-static vs ref behavior");
    assert_eq!((code_dy, out_dy.clone()), (code_rf, out_rf), "{tag}: native-dynamic vs ref behavior");
    assert_eq!(run_bin(&native_dy), (code_dy, out_dy), "{tag}: dynamic output changed between runs (ASLR-base dependent)");
    let (nat_def, nat_und) = dyn_syms(&native_st);
    let (ref_def, ref_und) = dyn_syms(&ref_st);
    assert_eq!(nat_def, ref_def, "{tag}: exported symbols differ");
    assert_eq!(nat_und, ref_und, "{tag}: imported symbols differ");
    for bin in [&native_st, &native_dy, &ref_st] {
        check_permissions(bin);
    }
    assert_eq!(elf_type(&native_st), 3, "{tag}: native static output is not ET_DYN");
    assert_eq!(elf_type(&ref_st), 3, "{tag}: reference output is not ET_DYN");
    assert_eq!(elf_type(&native_dy), 3, "{tag}: native dynamic output is not ET_DYN");
    assert!(has_segment(&native_dy, "INTERP"), "{tag}: native dynamic output lacks INTERP");
    assert!(has_segment(&native_dy, "DYNAMIC"), "{tag}: native dynamic output lacks DYNAMIC");
    assert!(
        dynamic_tag(&native_dy, "FLAGS_1").is_some_and(|l| l.contains("PIE")),
        "{tag}: native dynamic output lacks FLAGS_1 PIE"
    );
    for (name, bin) in [("native", &native_st), ("ref", &ref_st), ("native-dynamic", &native_dy)] {
        let libs = ldd_libs(bin);
        assert!(libs.contains("libc.so.6"), "{tag}: {name} ldd lacks libc: {libs}");
        assert!(!libs.contains("statically linked"), "{tag}: {name} is static: {libs}");
    }
    let _ = read_sections(&native_st);
}

fn check_static_modes(tag: &str, src: &str) {
    let dir = fixture_dir(tag);
    let path = write_src(&dir, "main.rnx", src);
    let objects = build_objects(&path, true, 1);
    let full_nat = dir.join(format!("{tag}_native_full"));
    let full_ref = dir.join(format!("{tag}_ref_full"));
    link_native_fully_static(&objects, &full_nat);
    link_ref_fully_static(&objects, &full_ref, &dir);
    assert_eq!(run_bin(&full_nat), run_bin(&full_ref), "{tag}: fully-static behavior");
    assert_eq!(elf_type(&full_nat), 2, "{tag}: fully-static output is not ET_EXEC");
    assert_eq!(
        global_syms(&full_nat, true),
        global_syms(&full_ref, true),
        "{tag}: fully-static defined globals differ"
    );
    assert_eq!(
        global_syms(&full_nat, false),
        global_syms(&full_ref, false),
        "{tag}: fully-static undefined globals differ"
    );
    for bin in [&full_nat, &full_ref] {
        check_permissions(bin);
    }
    let pie_nat = dir.join(format!("{tag}_native_pie"));
    let pie_ref = dir.join(format!("{tag}_ref_pie"));
    link_native_static_pie(&objects, &pie_nat);
    link_ref_static_pie(&objects, &pie_ref, &dir);
    assert_eq!(run_bin(&pie_nat), run_bin(&pie_ref), "{tag}: static-pie behavior");
    assert_eq!(run_bin(&pie_nat), run_bin(&pie_nat), "{tag}: static-pie output changed between runs (ASLR-base dependent)");
    assert_eq!(elf_type(&pie_nat), 3, "{tag}: static-pie output is not ET_DYN");
    assert_eq!(elf_type(&pie_ref), 3, "{tag}: static-pie reference is not ET_DYN");
    assert!(!has_segment(&pie_nat, "INTERP"), "{tag}: static-pie output has INTERP");
    assert!(has_segment(&pie_nat, "DYNAMIC"), "{tag}: static-pie output lacks DYNAMIC");
    assert!(
        dynamic_tag(&pie_nat, "FLAGS_1").is_some_and(|l| l.contains("PIE")),
        "{tag}: static-pie output lacks FLAGS_1 PIE"
    );
    let relacount: u64 = dynamic_tag(&pie_nat, "RELACOUNT")
        .and_then(|l| l.split_whitespace().last().unwrap_or("0").parse().ok())
        .unwrap_or(0);
    let types = rela_dyn_types(&pie_nat);
    let leading = types.iter().take_while(|t| *t == "R_X86_64_RELATIVE").count() as u64;
    assert!(leading > 0, "{tag}: static-pie .rela.dyn has no leading RELATIVE entries");
    assert_eq!(leading, relacount, "{tag}: static-pie RELACOUNT {relacount} != leading RELATIVE {leading}");
    let ldd = ldd_libs(&pie_nat);
    assert!(ldd.contains("statically linked"), "{tag}: static-pie ldd: {ldd}");
    for bin in [&pie_nat, &pie_ref] {
        check_permissions(bin);
    }
}

const MATH: &str = "fn Main(): Int {\n  let a = 40;\n  let b = 2;\n  let c = a + b;\n  return c * 3 - 84;\n}\n";

const STRINGS: &str = "fn Main(): Int {\n\
    let numbers = [10, 20, 30, 40, 50];\n\
    let sum = numbers.reduce(0, (acc, n) => acc + n);\n\
    print(sum);\n\
    let words = [\"hello\", \"world\", \"rasmalai\"];\n\
    print(words.join(\" \"));\n\
    let greeting = \"hello world from rasmalai\";\n\
    print(greeting.toUpperCase());\n\
    return sum - 150;\n\
}\n";

fn multi_src() -> String {
    let mut src = String::new();
    for i in 0..70 {
        src.push_str(&format!("fn F{i}(): Int {{ print({i}); return {i}; }}\n"));
    }
    src.push_str("fn Main(): Int {\n");
    for i in 0..70 {
        src.push_str(&format!("  F{i}();\n"));
    }
    src.push_str("  return 0;\n}\n");
    src
}

#[test]
fn diff_math() {
    check_fixture("math", &[("main.rnx", MATH)], true, 1);
}

#[test]
fn static_modes_math() {
    check_static_modes("stmath", MATH);
}

#[test]
fn static_modes_strings() {
    check_static_modes("ststrings", STRINGS);
}

#[test]
fn diff_strings() {
    check_fixture("strings", &[("main.rnx", STRINGS)], true, 1);
}

#[test]
fn diff_multi_object() {
    let src = multi_src();
    check_fixture("multi", &[("main.rnx", &src)], true, 8);
}

#[test]
fn static_links_without_explicit_system_libs() {
    let dir = fixture_dir("nolibs");
    let path = write_src(&dir, "main.rnx", MATH);
    let objects = build_objects(&path, true, 1);
    let mut inputs: Vec<LinkInput> =
        objects.iter().map(|b| LinkInput::ObjectBytes(b.clone())).collect();
    inputs.push(LinkInput::ArchiveBytes(runtime::archive::BYTES.to_vec()));
    let out = dir.join("nolibs_native_static");
    native::native_link_static(&inputs, &out, &native::StaticOpts::default()).unwrap();
    let (code, stdout) = run_bin(&out);
    assert_eq!((code, stdout.as_str()), (42, ""));
}

#[test]
fn native_output_is_deterministic() {
    let dir = fixture_dir("deterministic");
    let path = write_src(&dir, "main.rnx", STRINGS);
    let objects = build_objects(&path, true, 1);
    let mut inputs: Vec<LinkInput> =
        objects.iter().map(|b| LinkInput::ObjectBytes(b.clone())).collect();
    inputs.push(LinkInput::ArchiveBytes(runtime::archive::BYTES.to_vec()));
    for lib in ["pthread", "dl", "m", "c"] {
        inputs.push(LinkInput::Lib(lib.to_string()));
    }
    let first = dir.join("det_first");
    let second = dir.join("det_second");
    native::native_link_static(&inputs, &first, &native::StaticOpts::default()).unwrap();
    native::native_link_static(&inputs, &second, &native::StaticOpts::default()).unwrap();
    assert_eq!(std::fs::read(&first).unwrap(), std::fs::read(&second).unwrap());
}

#[test]
fn binutils_accept_native_output() {
    let dir = fixture_dir("binutils");
    let path = write_src(&dir, "main.rnx", MATH);
    let objects = build_objects(&path, true, 1);
    let so_dir = runtime_so_dir();
    let native_st = dir.join("bin_native_static");
    let native_dy = dir.join("bin_native_dynamic");
    link_native_static(&objects, &native_st);
    link_native_dynamic(&objects, &native_dy, &so_dir);
    for bin in [&native_st, &native_dy] {
        for args in [["-d"], ["-h"]] {
            let out = Command::new("objdump").args(args).arg(bin).output().unwrap();
            assert!(
                out.status.success(),
                "objdump {args:?} rejected {bin:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        for args in [["-SW"], ["-hW"], ["-lW"]] {
            let out = Command::new("readelf").args(args).arg(bin).output().unwrap();
            assert!(out.status.success(), "readelf {args:?} failed on {bin:?}");
        }
        let out = Command::new("nm").arg(bin).output().unwrap();
        assert!(
            out.status.success(),
            "nm rejected {bin:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let dis = Command::new("objdump").args(["-d"]).arg(&native_st).output().unwrap();
    assert!(dis.status.success());
    assert!(
        String::from_utf8_lossy(&dis.stdout).contains("Disassembly of section .text"),
        "objdump -d shows no .text disassembly"
    );
    let syms = Command::new("nm").arg(&native_st).output().unwrap();
    assert!(syms.status.success());
    assert!(!String::from_utf8_lossy(&syms.stdout).is_empty(), "nm shows no symbols");
}

#[test]
fn merged_sections_are_compact() {
    let dir = fixture_dir("compact");
    let path = write_src(&dir, "main.rnx", STRINGS);
    let objects = build_objects(&path, true, 1);
    let out = dir.join("compact_native_static");
    link_native_static(&objects, &out);
    let secs = read_sections(&out);
    assert!(secs.len() < 150, "expected merged sections, got {}", secs.len());
    let count = |want: &str| secs.iter().filter(|(name, _, _, _)| name == want).count();
    assert_eq!(count(".eh_frame"), 1, ".eh_frame was not merged");
    assert_eq!(count(".data.rel.ro"), 1, ".data.rel.ro was not merged");
    assert_eq!(count(".interp"), 1, "expected one .interp");
    assert!(count(".text") <= 6, ".text was not merged: {}", count(".text"));
    assert!(count(".rodata") <= 16, ".rodata was not merged: {}", count(".rodata"));
    assert!(count(".bss") <= 8, ".bss was not merged: {}", count(".bss"));
    assert!(count(".data") <= 8, ".data was not merged: {}", count(".data"));
    let raw = Command::new("readelf").args(["-SW", &out.to_string_lossy()]).output().unwrap();
    assert!(raw.status.success());
    for line in String::from_utf8_lossy(&raw.stdout).lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 8 && parts[2] == "VERSYM" {
            assert_eq!(parts[6], "02", ".gnu.version sh_entsize must be 2, got {}", parts[6]);
        }
    }
}

fn tls_markers(bytes: &[u8], label: &str) -> (usize, usize, usize) {
    let obj = linker::core::obj::Object::parse(bytes, label).unwrap();
    let secs = obj
        .sections
        .iter()
        .filter(|s| s.flags & linker::core::obj::SHF_TLS != 0)
        .count();
    let syms = obj.symbols.iter().filter(|s| s.kind == linker::core::obj::STT_TLS).count();
    let relas = obj
        .relas
        .iter()
        .filter(|r| {
            matches!(
                r.kind,
                linker::target::x86_64::R_TLSGD
                    | linker::target::x86_64::R_TLSLD
                    | linker::target::x86_64::R_DTPOFF32
                    | linker::target::x86_64::R_GOTTPOFF
                    | linker::target::x86_64::R_TPOFF32
            )
        })
        .count();
    (secs, syms, relas)
}

fn dev_objects(dir: &Path, tag: &str, src: &str) -> Vec<Vec<u8>> {
    let path = write_src(dir, &format!("{tag}.rnx"), src);
    let mut cfg = cli::CompileConfig::default();
    cfg.jobs = 1;
    cfg.memory_cap = u64::MAX;
    cli::build_files_staged_cfg(path.to_str().unwrap(), "Main", false, 1, None, &cfg)
        .unwrap_or_else(|e| panic!("build {}: {e:?}", path.display()))
        .objects
}

#[test]
fn dynamic_inputs_carry_no_tls() {
    let dir = fixture_dir("notls");
    let mut objects = Vec::new();
    objects.append(&mut dev_objects(&dir, "math", MATH));
    objects.append(&mut dev_objects(&dir, "strings", STRINGS));
    objects.append(&mut dev_objects(&dir, "multi", &multi_src()));
    assert!(!objects.is_empty());
    for (i, bytes) in objects.iter().enumerate() {
        let (secs, syms, relas) = tls_markers(bytes, &format!("user object {i}"));
        assert_eq!((secs, syms, relas), (0, 0, 0), "user object {i} carries TLS");
    }
    for name in ["Scrt1.o", "crti.o", "crtbeginS.o", "crtendS.o", "crtn.o"] {
        let path = native::system_file(name).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let (secs, syms, relas) = tls_markers(&bytes, name);
        assert_eq!((secs, syms, relas), (0, 0, 0), "{name} carries TLS");
    }
}

#[test]
fn dynamic_tls_still_rejected() {
    let dir = fixture_dir("tlserr");
    let mut objects = dev_objects(&dir, "math", MATH);
    assert!(!objects.is_empty());
    let bytes = &mut objects[0];
    let shoff = u64::from_le_bytes(bytes[40..48].try_into().unwrap()) as usize;
    let shentsize = u16::from_le_bytes(bytes[58..60].try_into().unwrap()) as usize;
    let shnum = u16::from_le_bytes(bytes[60..62].try_into().unwrap()) as usize;
    let obj = linker::core::obj::Object::parse(bytes, "probe").unwrap();
    let text = (0..shnum)
        .find(|&i| obj.section_name(i).unwrap_or("") == ".text")
        .expect("user object has no .text");
    let flags_off = shoff + text * shentsize + 8;
    let mut flags = u64::from_le_bytes(bytes[flags_off..flags_off + 8].try_into().unwrap());
    flags |= linker::core::obj::SHF_TLS;
    bytes[flags_off..flags_off + 8].copy_from_slice(&flags.to_le_bytes());
    let (secs, _, _) = tls_markers(bytes, "patched");
    assert!(secs > 0);
    let so_dir = runtime_so_dir();
    let inputs: Vec<LinkInput> = objects.iter().map(|b| LinkInput::ObjectBytes(b.clone())).collect();
    let opts = native::DynamicOpts {
        entry: "_start".to_string(),
        icf: true,
        target: "x86_64-unknown-linux-gnu".to_string(),
        lib_dirs: vec![so_dir.clone()],
        libs: vec!["runtime_native".to_string()],
        runpath: Some(so_dir.to_string_lossy().into_owned()),
        strip: false,
    };
    let err = native::native_link_dynamic(&inputs, &dir.join("tls_out"), &opts).unwrap_err();
    assert!(err.to_string().contains("TLS"), "unexpected error: {err}");
}

#[test]
fn system_file_miss_lists_search_dirs() {
    let err = native::system_file("rnx-definitely-missing-file.o").unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("rnx-definitely-missing-file.o"), "{msg}");
    assert!(msg.contains("/usr/lib"), "{msg}");
}

#[test]
fn native_rejects_cross_target() {
    let dir = fixture_dir("crosstarget");
    let path = write_src(&dir, "main.rnx", MATH);
    let objects = build_objects(&path, true, 1);
    let inputs: Vec<LinkInput> = objects.iter().map(|b| LinkInput::ObjectBytes(b.clone())).collect();
    let mut opts = native::StaticOpts::default();
    opts.target = "aarch64-unknown-linux-gnu".to_string();
    let err = native::native_link_static(&inputs, &dir.join("out"), &opts).unwrap_err();
    assert!(err.to_string().contains("native backend not yet implemented"), "{err}");
    let dopts = native::DynamicOpts {
        entry: "_start".to_string(),
        icf: true,
        target: "x86_64-pc-windows-msvc".to_string(),
        lib_dirs: vec![dir.clone()],
        libs: vec![],
        runpath: None,
        strip: false,
    };
    let err = native::native_link_dynamic(&inputs, &dir.join("out2"), &dopts).unwrap_err();
    assert!(err.to_string().contains("native backend not yet implemented"), "{err}");
    let _ = dir;
}

fn link_native_static_stripped(objects: &[Vec<u8>], out: &Path) {
    let mut inputs: Vec<LinkInput> = objects.iter().map(|b| LinkInput::ObjectBytes(b.clone())).collect();
    inputs.push(LinkInput::ArchiveBytes(runtime::archive::BYTES.to_vec()));
    for lib in ["pthread", "dl", "m", "c"] {
        inputs.push(LinkInput::Lib(lib.to_string()));
    }
    let mut opts = native::StaticOpts::default();
    opts.strip = true;
    native::native_link_static(&inputs, out, &opts).unwrap();
}

#[test]
fn stripped_static_drops_symtab_keeps_build_id_and_runs() {
    let dir = fixture_dir("strip");
    let path = write_src(&dir, "main.rnx", MATH);
    let objects = build_objects(&path, true, 1);
    let stripped = dir.join("strip_native");
    link_native_static_stripped(&objects, &stripped);
    let (code, stdout) = run_bin(&stripped);
    assert_eq!((code, stdout.as_str()), (42, ""));
    let secs = read_sections(&stripped);
    let names: Vec<&str> = secs.iter().map(|(name, _, _, _)| name.as_str()).collect();
    assert!(!names.contains(&".symtab"), "stripped output kept .symtab: {names:?}");
    assert!(!names.contains(&".strtab"), "stripped output kept .strtab: {names:?}");
    assert!(names.contains(&".note.gnu.build-id"), "stripped output lacks .note.gnu.build-id: {names:?}");
    assert!(names.contains(&".shstrtab"), "stripped output lacks .shstrtab: {names:?}");
    let notes = Command::new("readelf").args(["-n", &stripped.to_string_lossy()]).output().unwrap();
    assert!(notes.status.success());
    let notes = String::from_utf8_lossy(&notes.stdout).into_owned();
    assert!(notes.contains("NT_GNU_BUILD_ID"), "readelf -n shows no build ID: {notes}");
    assert!(notes.contains("Build ID:"), "readelf -n shows no Build ID value: {notes}");
    let file_out = Command::new("file").arg(&stripped).output().unwrap();
    assert!(file_out.status.success());
    let classification = String::from_utf8_lossy(&file_out.stdout).into_owned();
    assert!(
        classification.contains("stripped") && !classification.contains("not stripped"),
        "unexpected file classification: {classification}"
    );
    assert!(classification.contains("BuildID"), "file shows no BuildID: {classification}");
    let again = dir.join("strip_native_again");
    link_native_static_stripped(&objects, &again);
    assert_eq!(
        std::fs::read(&stripped).unwrap(),
        std::fs::read(&again).unwrap(),
        "stripped output (including BuildID) is not deterministic"
    );
}
