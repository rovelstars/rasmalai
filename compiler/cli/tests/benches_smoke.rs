use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;

fn rnx() -> String {
    env!("CARGO_BIN_EXE_rnx").to_string()
}

fn benches_dir() -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    root.parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("benches")
}

fn fresh_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rnx-benchsmoke-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run_bench(bench: &str, backend: Option<&str>) -> std::process::Output {
    let dir = fresh_dir(&format!("{bench}-{}", backend.unwrap_or("interp")));
    let prog = dir.join("main.rnx");
    let src = std::fs::read(benches_dir().join(format!("{bench}.rnx"))).unwrap();
    std::fs::write(&prog, src).unwrap();
    let mut cmd = Command::new(rnx());
    cmd.arg("run").env("NO_COLOR", "1").env("RNX_BENCH_SCALE", "tiny");
    if let Some(b) = backend {
        cmd.arg("--backend").arg(b);
    }
    cmd.arg(&prog);
    let out = cmd.output().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    out
}

fn result_lines(stdout: &str) -> BTreeMap<String, Vec<String>> {
    let mut vals: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for line in stdout.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("RESULT ") {
            let mut parts = rest.splitn(2, ' ');
            let key = parts.next().unwrap_or("").to_string();
            let val = parts.next().unwrap_or("").to_string();
            if key.ends_with("ms") || key == "live_delta" {
                continue;
            }
            if key == "workers" {
                let fields: Vec<&str> = val.split_whitespace().collect();
                if fields.len() == 3 {
                    vals.entry(key).or_default().push(format!("{} {}", fields[0], fields[2]));
                    continue;
                }
            }
            vals.entry(key).or_default().push(val);
        }
    }
    vals
}

fn check_bench(bench: &str) {
    let mut reference: Option<BTreeMap<String, Vec<String>>> = None;
    for backend in [None, Some("cranelift"), Some("llvm")] {
        let out = run_bench(bench, backend);
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "{bench} {:?} failed: {stderr}", backend);
        let vals = result_lines(&stdout);
        assert!(!vals.is_empty(), "{bench} {:?} printed no RESULT lines", backend);
        match &reference {
            None => reference = Some(vals),
            Some(want) => assert_eq!(&vals, want, "{bench} {:?} checksum mismatch", backend),
        }
    }
}

#[test]
fn benches_run_tiny_on_all_backends() {
    for bench in ["binary_trees", "parallel_workers", "net_throughput", "json_stress"] {
        check_bench(bench);
    }
}
