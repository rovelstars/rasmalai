use std::path::PathBuf;
use std::process::Command;

fn resolve_src(src: &str, tag: &str) -> (lir::instr::Module, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rnx-dinan-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let main = dir.join("main.rnx");
    std::fs::write(&main, src).unwrap();
    let g = frontend::modules::ModuleGraph::build(&main).unwrap_or_else(|e| panic!("{e}"));
    let mut m = g.resolve().unwrap_or_else(|e| panic!("{e}"));
    let d = frontend::desugar::desugar(&mut m);
    assert!(d.is_empty(), "{d:?}");
    let c = frontend::semantic::check(&m);
    assert!(c.iter().all(|x| x.code.is_warning()), "{c:?}");
    let mut out = lir::lower::lower(&m).unwrap_or_else(|e| panic!("{e}"));
    lir::opt::optimize_lir(&mut out, 1, "Main");
    let v = lir::verify::verify(&out);
    assert!(v.is_empty(), "{v:?}");
    (out, dir)
}

fn run_backend(rnx: &str, dir: &PathBuf, backend: &str) -> String {
    let out = Command::new(rnx)
        .arg("run")
        .arg("--backend")
        .arg(backend)
        .arg(dir.join("main.rnx"))
        .output()
        .unwrap();
    assert!(out.status.success(), "{backend} run failed: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap()
}

fn check_stdout_everywhere(src: &str, want: &str, tag: &str) {
    let (module, dir) = resolve_src(src, tag);
    let leaked: &'static lir::instr::Module = Box::leak(Box::new(module));
    let mut machine = runtime::machine::Machine::new(leaked);
    let r = machine.call("Main", vec![]).unwrap_or_else(|e| panic!("interpreter {tag}: {e:?}"));
    match r {
        runtime::value::Value::Int(0) => {}
        other => panic!("interpreter {tag}: {other:?}"),
    }
    assert_eq!(machine.output.join("\n") + "\n", want, "interpreter {tag} stdout");

    let rnx = env!("CARGO_BIN_EXE_rnx");
    for backend in ["cranelift", "llvm"] {
        let got = run_backend(rnx, &dir, backend);
        assert_eq!(got, want, "{backend} {tag} stdout");
    }

    let build = std::process::Command::new(rnx)
        .arg("build")
        .arg(dir.join("main.rnx"))
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let bin = String::from_utf8_lossy(&build.stdout).lines().rev().find_map(|l| l.strip_prefix("artifact: ")).map(|s| s.trim().trim_end_matches(" (fresh)").to_string()).expect("build prints artifact path");
    let run = std::process::Command::new(&bin).output().unwrap();
    assert_eq!(run.status.code().unwrap(), 0, "aot {tag} exit");
    assert_eq!(String::from_utf8(run.stdout).unwrap(), want, "aot {tag} stdout");
    let _ = std::fs::remove_dir_all(&dir);
}

fn check_fails_with(src: &str, want: &[&str], tag: &str) {
    let dir = std::env::temp_dir().join(format!("rnx-dinan-err-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.rnx"), src).unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_rnx"))
        .arg("build")
        .env("NO_COLOR", "1")
        .current_dir(&dir)
        .arg("main.rnx")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "expected failure");
    let mut text = String::from_utf8_lossy(&out.stderr).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stdout));
    for w in want {
        assert!(text.contains(w), "missing `{w}` in: {text}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

const PRELUDE: &str = "interface Summarizable {\n\
    fn summary(): String;\n\
}\n\
class User : Summarizable {\n\
    let name: String;\n\
    let age: Int;\n\
    init(name: String, age: Int) {\n\
        this.name = name;\n\
        this.age = age;\n\
    }\n\
    fn summary(): String {\n\
        return \"${this.name} (${this.age})\";\n\
    }\n\
}\n\
class SystemConfig {\n\
    let id: Int;\n\
    init(id: Int) { this.id = id; }\n\
}\n";

#[test]
fn iface_dispatch_and_rtti() {
    check_stdout_everywhere(
        &format!("{PRELUDE}\n\
        fn Main(): Int {{\n\
            let u = new User(\"Alice\", 28);\n\
            let s: Summarizable = u;\n\
            print(s.summary());\n\
            print(typeOf(s));\n\
            let any_u: Any = u;\n\
            print(any_u is Summarizable);\n\
            print(any_u is User);\n\
            let cfg: Any = new SystemConfig(1);\n\
            print(cfg is Summarizable);\n\
            print(s is Summarizable);\n\
            print(s is User);\n\
            print(s is SystemConfig);\n\
            return 0;\n\
        }}\n"),
        "Alice (28)\nUser\ntrue\ntrue\nfalse\ntrue\ntrue\nfalse\n",
        "iface_dispatch",
    );
}

#[test]
fn iface_switch_and_narrowing() {
    check_stdout_everywhere(
        &format!("{PRELUDE}\n\
        fn label(v: Any): String {{\n\
            switch v {{\n\
            case is Summarizable: return \"sum\";\n\
            default: return \"other\";\n\
            }}\n\
        }}\n\
        fn show(s: Summarizable): String {{\n\
            return s.summary();\n\
        }}\n\
        fn Main(): Int {{\n\
            let u = new User(\"Bo\", 7);\n\
            print(label(u));\n\
            print(label(42));\n\
            print(show(u));\n\
            let a: Any = u;\n\
            if (a is Summarizable) {{\n\
                print(show(a));\n\
            }}\n\
            if (u is Summarizable) {{\n\
                print(show(u));\n\
            }}\n\
            return 0;\n\
        }}\n"),
        "sum\nother\nBo (7)\nBo (7)\nBo (7)\n",
        "iface_switch",
    );
}

#[test]
fn named_and_default_args() {
    check_stdout_everywhere(
        "fn configure(host: String, port: Int = 8080, secure: Bool = false): String {\n\
        let sec = \"http\";\n\
        if (secure) {\n\
            sec = \"https\";\n\
        }\n\
        return \"${sec}://${host}:${port}\";\n\
        }\n\
        fn Main(): Int {\n\
            print(configure(\"api.dev\"));\n\
            print(configure(\"api.dev\", secure: true));\n\
            print(configure(\"api.dev\", port: 9000, secure: true));\n\
            print(configure(\"api.dev\", 9000));\n\
            return 0;\n\
        }\n",
        "http://api.dev:8080\nhttps://api.dev:8080\nhttps://api.dev:9000\nhttp://api.dev:9000\n",
        "named_args",
    );
}

#[test]
fn defer_throw_catch_unwind() {
    check_stdout_everywhere(
        "fn fallible(fail: Bool, log: Array<String>): Int {\n\
        defer log.push(\"defer_outer\");\n\
        try {\n\
            defer log.push(\"defer_inner_1\");\n\
            defer log.push(\"defer_inner_2\");\n\
            if (fail) {\n\
                throw \"database_down\";\n\
            }\n\
            defer log.push(\"defer_never_reached\");\n\
            return 200;\n\
        } catch (err) {\n\
            log.push(\"caught: \" + err);\n\
            return 500;\n\
        }\n\
        }\n\
        fn Main(): Int {\n\
            let log: Array<String> = [];\n\
            print(fallible(true, log));\n\
            print(log.length());\n\
            print(log[0]);\n\
            print(log[1]);\n\
            print(log[2]);\n\
            print(log[3]);\n\
            let log2: Array<String> = [];\n\
            print(fallible(false, log2));\n\
            print(log2.length());\n\
            print(log2[0]);\n\
            print(log2[1]);\n\
            return 0;\n\
        }\n",
        "500\n4\ndefer_inner_2\ndefer_inner_1\ncaught: database_down\ndefer_outer\n200\n4\ndefer_never_reached\ndefer_inner_2\n",
        "unwind",
    );
}

#[test]
fn task_program_assert_style() {
    check_stdout_everywhere(
        &format!("{PRELUDE}\n\
        fn configure(host: String, port: Int = 8080, secure: Bool = false): String {{\n\
            let sec = \"http\";\n\
            if (secure) {{\n\
                sec = \"https\";\n\
            }}\n\
            return \"${{sec}}://${{host}}:${{port}}\";\n\
        }}\n\
        fn fallible_service(fail: Bool, log: Array<String>): Int {{\n\
            defer log.push(\"defer_outer\");\n\
            try {{\n\
                defer log.push(\"defer_inner_1\");\n\
                defer log.push(\"defer_inner_2\");\n\
                if (fail) {{\n\
                    throw \"database_down\";\n\
                }}\n\
                defer log.push(\"defer_never_reached\");\n\
                return 200;\n\
            }} catch (err) {{\n\
                log.push(\"caught: \" + err);\n\
                return 500;\n\
            }}\n\
        }}\n\
        fn Main(): Int {{\n\
            let u = new User(\"Alice\", 28);\n\
            let s: Summarizable = u;\n\
            assert(s.summary() == \"Alice (28)\", \"dispatch\");\n\
            let any_u: Any = u;\n\
            assert(any_u is Summarizable, \"rtti true\");\n\
            let cfg: Any = new SystemConfig(1);\n\
            assert(!(cfg is Summarizable), \"rtti false\");\n\
            assert(configure(\"api.dev\") == \"http://api.dev:8080\", \"defaults\");\n\
            assert(configure(\"api.dev\", secure: true) == \"https://api.dev:8080\", \"named\");\n\
            assert(configure(\"api.dev\", port: 9000, secure: true) == \"https://api.dev:9000\", \"named all\");\n\
            let log: Array<String> = [];\n\
            let status = fallible_service(true, log);\n\
            assert(status == 500, \"status\");\n\
            assert(log.length() == 4, \"len\");\n\
            assert(log[0] == \"defer_inner_2\", \"lifo2\");\n\
            assert(log[1] == \"defer_inner_1\", \"lifo1\");\n\
            assert(log[2] == \"caught: database_down\", \"caught\");\n\
            assert(log[3] == \"defer_outer\", \"outer\");\n\
            print(\"ok\");\n\
            return 0;\n\
        }}\n"),
        "ok\n",
        "task_program",
    );
}

#[test]
fn err_unknown_named_param() {
    check_fails_with(
        "fn f(a: Int, b: Int = 1): Int { return a + b; }\nfn Main(): Int { print(f(1, c: 2)); return 0; }\n",
        &["unknown parameter `c`"],
        "unknown_param",
    );
}

#[test]
fn err_duplicate_named_param() {
    check_fails_with(
        "fn f(a: Int, b: Int = 1): Int { return a + b; }\nfn Main(): Int { print(f(a: 1, a: 2)); return 0; }\n",
        &["duplicate argument"],
        "dup_param",
    );
}

#[test]
fn err_mandatory_after_default() {
    check_fails_with(
        "fn f(a: Int = 1, b: Int): Int { return a + b; }\nfn Main(): Int { return 0; }\n",
        &["follows a defaulted parameter"],
        "param_order",
    );
}

#[test]
fn err_construct_interface() {
    check_fails_with(
        &format!("{PRELUDE}\nfn Main(): Int {{\n    let s = Summarizable();\n    return 0;\n}}\n"),
        &["cannot construct interface"],
        "construct_iface",
    );
}

#[test]
fn err_missing_conformance() {
    check_fails_with(
        "interface P { fn ping(): Int; }\nclass C : P {\n    let x: Int;\n    init(x: Int) { this.x = x; }\n}\nfn Main(): Int { return 0; }\n",
        &["does not implement `ping`"],
        "conformance",
    );
}

#[test]
fn err_unknown_interface() {
    check_fails_with(
        "class C : Ghost {\n    let x: Int;\n    init(x: Int) { this.x = x; }\n}\nfn Main(): Int { return 0; }\n",
        &["unknown interface `Ghost`"],
        "unknown_iface",
    );
}
