use std::path::PathBuf;

const FIB_SRC: &str = "fn Fib(n: Int): Int {\n    if n < 2 {\n        return n;\n    }\n    return Fib(n - 1) + Fib(n - 2);\n}\n\nfn Main(): Int {\n    return Fib(10);\n}\n";

fn write_src(tag: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rnx-profdbg-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("main.rnx");
    std::fs::write(&src, FIB_SRC).unwrap();
    (dir, src)
}

fn rnx() -> String {
    env!("CARGO_BIN_EXE_rnx").to_string()
}

fn readelf_sections(path: &std::path::Path) -> String {
    let out = std::process::Command::new("readelf")
        .arg("-S")
        .arg(path)
        .output()
        .expect("readelf runs");
    assert!(out.status.success(), "readelf failed");
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn test_time_passes_output() {
    let (dir, src) = write_src("time");
    let out_path = dir.join("diag_test");
    let out = std::process::Command::new(rnx())
        .arg("build")
        .arg("--time-passes")
        .arg(&src)
        .arg("-o")
        .arg(&out_path)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let stderr = String::from_utf8(out.stderr).unwrap();
    for pass in ["lex_parse", "typecheck", "opt_", "codegen"] {
        assert!(stderr.contains(pass), "missing {pass}:\n{stderr}");
    }
    assert!(stderr.contains("lines/sec"), "missing throughput:\n{stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_perfetto_trace_json_validity() {
    let (dir, src) = write_src("trace");
    let trace_path = dir.join("trace.json");
    let out_path = dir.join("trace_test");
    let out = std::process::Command::new(rnx())
        .arg("build")
        .arg("--trace")
        .arg(&trace_path)
        .arg(&src)
        .arg("-o")
        .arg(&out_path)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(trace_path.exists(), "trace file missing");
    let text = std::fs::read_to_string(&trace_path).unwrap();
    assert!(text.contains("\"traceEvents\""), "no traceEvents:\n{text}");
    assert!(text.contains("\"ph\": \"X\""), "no complete events:\n{text}");
    for pass in ["lex_parse", "typecheck", "opt_pipeline", "codegen"] {
        assert!(text.contains(pass), "missing {pass}:\n{text}");
    }
    let mut events = 0;
    for line in text.lines() {
        if line.contains("\"ph\": \"X\"") {
            assert!(line.contains("\"ts\":"), "no ts:\n{line}");
            assert!(line.contains("\"dur\":"), "no dur:\n{line}");
            let ts: u64 = line
                .split("\"ts\":")
                .nth(1)
                .unwrap()
                .split(|c: char| !c.is_ascii_digit())
                .find(|s| !s.is_empty())
                .unwrap()
                .parse()
                .unwrap();
            let dur: u64 = line
                .split("\"dur\":")
                .nth(1)
                .unwrap()
                .split(|c: char| !c.is_ascii_digit())
                .find(|s| !s.is_empty())
                .unwrap()
                .parse()
                .unwrap();
            let _ = (ts, dur);
            events += 1;
        }
    }
    assert!(events >= 4, "too few events: {events}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_perf_map_generation() {
    let (dir, src) = write_src("perfmap");
    let out = std::process::Command::new(rnx())
        .arg("run")
        .arg("--backend")
        .arg("cranelift")
        .arg("--perf-map")
        .arg(&src)
        .output()
        .unwrap();
    assert_eq!(out.status.code().unwrap(), 55, "{}", String::from_utf8_lossy(&out.stderr));
    let stderr = String::from_utf8(out.stderr).unwrap();
    let line = stderr
        .lines()
        .find(|l| l.starts_with("perf map: "))
        .expect(format!("no perf map line:\n{stderr}").as_str());
    let map_path = PathBuf::from(line["perf map: ".len()..].trim());
    assert!(map_path.exists(), "map file missing");
    let text = std::fs::read_to_string(&map_path).unwrap();
    assert!(!text.is_empty(), "map file empty");
    let mut saw_main = false;
    for entry in text.lines() {
        let mut parts = entry.split_whitespace();
        let addr = parts.next().expect("addr");
        let size = parts.next().expect("size");
        let name = parts.next().expect("name");
        assert!(
            addr.chars().all(|c| c.is_ascii_hexdigit()),
            "bad addr `{addr}`"
        );
        assert!(
            size.chars().all(|c| c.is_ascii_hexdigit()),
            "bad size `{size}`"
        );
        assert!(!usize::from_str_radix(addr, 16).is_ok_and(|a| a == 0), "zero addr");
        if name == "Main" {
            saw_main = true;
        }
    }
    assert!(saw_main, "no Main entry:\n{text}");
    let _ = std::fs::remove_file(&map_path);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_dwarf_debug_symbols_present() {
    let (dir, src) = write_src("dwarf");
    let obj_path = dir.join("debug_app.o");
    let out = std::process::Command::new(rnx())
        .arg("build")
        .arg("--emit-obj")
        .arg("-g")
        .arg(&src)
        .arg("-o")
        .arg(&obj_path)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let sections = readelf_sections(&obj_path);
    assert!(sections.contains(".debug_info"), "no .debug_info:\n{sections}");
    assert!(sections.contains(".debug_line"), "no .debug_line:\n{sections}");
    let app_path = dir.join("debug_app");
    let out = std::process::Command::new(rnx())
        .arg("build")
        .arg("-g")
        .arg(&src)
        .arg("-o")
        .arg(&app_path)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let sections = readelf_sections(&app_path);
    assert!(sections.contains(".debug_info"), "no .debug_info:\n{sections}");
    assert!(sections.contains(".debug_line"), "no .debug_line:\n{sections}");
    let run = std::process::Command::new(&app_path).output().unwrap();
    assert_eq!(run.status.code().unwrap(), 55);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_release_omits_debug_info() {
    let (dir, src) = write_src("release");
    let obj_path = dir.join("release_app.o");
    let out = std::process::Command::new(rnx())
        .arg("build")
        .arg("--emit-obj")
        .arg("--release")
        .arg("-g")
        .arg(&src)
        .arg("-o")
        .arg(&obj_path)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let sections = readelf_sections(&obj_path);
    assert!(!sections.contains(".debug_info"), "leaked .debug_info:\n{sections}");
    assert!(!sections.contains(".debug_line"), "leaked .debug_line:\n{sections}");
    let app_path = dir.join("release_app");
    let out = std::process::Command::new(rnx())
        .arg("build")
        .arg("--release")
        .arg(&src)
        .arg("-o")
        .arg(&app_path)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let sections = readelf_sections(&app_path);
    assert!(!sections.contains(".debug_info"), "leaked .debug_info:\n{sections}");
    assert!(!sections.contains(".debug_line"), "leaked .debug_line:\n{sections}");
    let _ = std::fs::remove_dir_all(&dir);
}
