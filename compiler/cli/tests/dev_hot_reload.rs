use cli::dev::{DevEvent, DevRunner};
use std::path::PathBuf;
use std::time::{Duration, Instant};

static HARNESS_CTR: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn fresh_file(tag: &str, src: &str) -> PathBuf {
    let n = HARNESS_CTR.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("rnx-devtest-{tag}-{n}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("main.rnx");
    std::fs::write(&f, src).unwrap();
    f
}

fn wait_event(runner: &mut DevRunner, deadline: Duration) -> Option<DevEvent> {
    let end = Instant::now() + deadline;
    while Instant::now() < end {
        if let Some(ev) = runner.poll(Duration::from_millis(100)) {
            return Some(ev);
        }
    }
    None
}

const V1: &str = "fn getValue(): Int { return 1; }\nfn Main(): Int { return getValue(); }\n";
const V2: &str = "fn getValue(): Int { return 2; }\nfn Main(): Int { return getValue(); }\n";
const V3: &str = "fn getValue(): Int { return 3; }\nfn Main(): Int { return getValue(); }\n";

#[test]
fn test_body_swap_under_watcher() {
    let f = fresh_file("swap", V1);
    let mut runner = DevRunner::new(&f, "Main", true).unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(runner.call("Main", &[]).unwrap(), 1);
    std::fs::write(&f, V2).unwrap();
    match wait_event(&mut runner, Duration::from_secs(10)) {
        Some(DevEvent::Swapped { names, .. }) => assert!(names.contains(&"getValue".to_string()), "{names:?}"),
        other => panic!("expected swap, got {}", other.is_some()),
    }
    assert_eq!(runner.call("getValue", &[]).unwrap(), 2);
    assert_eq!(runner.call("Main", &[]).unwrap(), 2);
}

#[test]
fn test_diagnostic_tolerance() {
    let f = fresh_file("diag", V2);
    let mut runner = DevRunner::new(&f, "Main", true).unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(runner.call("Main", &[]).unwrap(), 2);
    std::fs::write(&f, "fn getValue(): Int { return ").unwrap();
    match wait_event(&mut runner, Duration::from_secs(10)) {
        Some(DevEvent::Broken { count }) => assert!(count >= 1),
        _ => panic!("expected broken"),
    }
    assert_eq!(runner.call("getValue", &[]).unwrap(), 2);
    assert_eq!(runner.call("Main", &[]).unwrap(), 2);
    std::fs::write(&f, V3).unwrap();
    match wait_event(&mut runner, Duration::from_secs(10)) {
        Some(DevEvent::Swapped { .. }) => {}
        _ => panic!("expected recovery swap"),
    }
    assert_eq!(runner.call("Main", &[]).unwrap(), 3);
}

#[test]
fn test_structural_change_restarts() {
    let f = fresh_file("struct", V1);
    let mut runner = DevRunner::new(&f, "Main", true).unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(runner.call("Main", &[]).unwrap(), 1);
    std::fs::write(&f, "fn getValue(x: Int): Int { return x; }\nfn Main(): Int { return getValue(7); }\n").unwrap();
    match wait_event(&mut runner, Duration::from_secs(10)) {
        Some(DevEvent::Restarted { reason }) => assert!(reason.contains("getValue"), "{reason}"),
        _ => panic!("expected restart"),
    }
    assert_eq!(runner.call("Main", &[]).unwrap(), 7);
}

#[test]
fn test_delta_classify_unit() {
    let old = module_of_src(V1);
    let same = module_of_src("fn getValue(): Int { return 1; }\nfn Main(): Int { return getValue(); }\n");
    match cli::delta::classify(&old, &same) {
        cli::delta::Delta::Pure { swapped } => assert!(swapped.is_empty(), "{swapped:?}"),
        _ => panic!("identical modules must be pure+empty"),
    }
    let new = module_of_src(V2);
    match cli::delta::classify(&old, &new) {
        cli::delta::Delta::Pure { swapped } => assert_eq!(swapped, vec!["getValue".to_string()]),
        _ => panic!("body change must be pure"),
    }
    let sig = module_of_src("fn getValue(x: Int): Int { return x; }\nfn Main(): Int { return getValue(1); }\n");
    match cli::delta::classify(&old, &sig) {
        cli::delta::Delta::Structural { .. } => {}
        _ => panic!("arity change must be structural"),
    }
    let cls = module_of_src("fn getValue(): Int { return 1; }\nfn Main(): Int { return getValue(); }\nclass P { let x: Int; }\n");
    match cli::delta::classify(&old, &cls) {
        cli::delta::Delta::Structural { .. } => {}
        _ => panic!("added class must be structural"),
    }
}

fn module_of_src(src: &str) -> frontend::ast::Module {
    let n = HARNESS_CTR.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("rnx-devunit-{n}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.rnx"), src).unwrap();
    let g = frontend::modules::ModuleGraph::build(&dir.join("main.rnx")).unwrap_or_else(|e| panic!("{e}"));
    let mut m = g.resolve().unwrap_or_else(|e| panic!("{e}"));
    frontend::harness::strip_tests(&mut m);
    frontend::harness::strip_benches(&mut m);
    let d = frontend::desugar::desugar(&mut m);
    assert!(d.is_empty(), "{d:?}");
    let _ = std::fs::remove_dir_all(&dir);
    m
}
