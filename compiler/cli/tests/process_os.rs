use std::path::PathBuf;

fn resolve_src(src: &str, tag: &str) -> (lir::instr::Module, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rnx-procos-{tag}-{}", std::process::id()));
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

fn check_all_backends(src: &str, want: i64, want_out: &[String], tag: &str) {
    let (module, dir) = resolve_src(src, tag);
    let leaked: &'static lir::instr::Module = Box::leak(Box::new(module));
    let mut machine = runtime::machine::Machine::new(leaked);
    let r = machine.call("Main", vec![]).unwrap_or_else(|e| panic!("interpreter {tag}: {e:?}"));
    match r {
        runtime::value::Value::Int(v) if v == want => {}
        other => panic!("interpreter {tag}: {other:?}"),
    }
    let got = machine.output.clone();
    let want_out: Vec<String> = want_out.to_vec();
    assert_eq!(got, want_out, "interpreter {tag} stdout");

    let mut jit = cranelift::jit::Jit::compile(leaked).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("Main", &[]).unwrap(), want, "cranelift {tag}");
    assert_eq!(llvm::codegen::execute(leaked, "Main").unwrap(), want, "llvm {tag}");

    let rnx = env!("CARGO_BIN_EXE_rnx");
    let build = std::process::Command::new(rnx)
        .arg("build")
        .arg(dir.join("main.rnx"))
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let bin = String::from_utf8_lossy(&build.stdout).lines().rev().find_map(|l| l.strip_prefix("artifact: ")).map(|s| s.trim().trim_end_matches(" (fresh)").to_string()).expect("build prints artifact path");
    let run = std::process::Command::new(&bin).output().unwrap();
    assert_eq!(run.status.code().unwrap(), want as i32, "aot {tag} exit");
    let suffix = if want_out.is_empty() { "" } else { "\n" };
    assert_eq!(
        String::from_utf8(run.stdout).unwrap(),
        want_out.join("\n") + suffix,
        "aot {tag} stdout"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

fn check_aot_only(src: &str, want: i64, tag: &str) {
    let dir = std::env::temp_dir().join(format!("rnx-procos-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.rnx"), src).unwrap();
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let build = std::process::Command::new(rnx)
        .arg("build")
        .arg(dir.join("main.rnx"))
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let bin = String::from_utf8_lossy(&build.stdout).lines().rev().find_map(|l| l.strip_prefix("artifact: ")).map(|s| s.trim().trim_end_matches(" (fresh)").to_string()).expect("build prints artifact path");
    let run = std::process::Command::new(&bin).output().unwrap();
    assert_eq!(run.status.code().unwrap(), want as i32, "aot {tag} exit: {}", String::from_utf8_lossy(&run.stderr));
    let _ = std::fs::remove_dir_all(&dir);
}

const SCRIPT: &str = "import { Process, Stdio, SpawnOptions } from \"@std/process\";\n\
import { OS } from \"@std/os\";\n\
import { ByteBuffer } from \"@std/bytes\";\n\
import { Map } from \"@std/collections\";\n\
fn Main(): Int {\n\
    let platform = OS.platform();\n\
    assert(platform == \"linux\" || platform == \"macos\" || platform == \"windows\", \"valid os platform\");\n\
    let arch = OS.arch();\n\
    assert(arch == \"x86_64\" || arch == \"aarch64\" || arch == \"arm\", \"valid cpu architecture\");\n\
    assert(OS.cpuCount() >= 1, \"cpu count positive\");\n\
    assert(OS.uptime() >= 0.0, \"uptime non-negative\");\n\
    assert(OS.tmpdir().length() > 0, \"tmpdir not empty\");\n\
    assert(OS.hostname().length() > 0, \"hostname not empty\");\n\
    if (platform == \"windows\") {\n\
        assert(OS.eol().length() == 2, \"eol crlf\");\n\
    } else {\n\
        assert(OS.eol() == \"\\n\", \"eol lf\");\n\
    }\n\
    let home = OS.homedir();\n\
    let home_env = Process.allEnv().get(\"HOME\");\n\
    if (home_env != null) {\n\
        assert(home == (home_env ?? \"\"), \"homedir matches HOME\");\n\
    }\n\
    let pid = Process.pid();\n\
    assert(pid > 0, \"process pid positive\");\n\
    let cwd = Process.cwd();\n\
    assert(cwd.length() > 0, \"cwd not empty\");\n\
    assert(Process.args().length() >= 1, \"args non-empty\");\n\
    let test_key = \"RNX_TEST_PROCESS_ENV_VAR\";\n\
    Process.setEnv(test_key, \"active_123\");\n\
    let fetched = Process.env(test_key);\n\
    assert(fetched != null && (fetched ?? \"\") == \"active_123\", \"env set and get\");\n\
    assert((Process.allEnv().get(test_key) ?? \"\") == \"active_123\", \"allEnv sees var\");\n\
    Process.removeEnv(test_key);\n\
    assert(Process.env(test_key) == null, \"env remove\");\n\
    let echo_cmd = platform == \"windows\" ? \"cmd\" : \"echo\";\n\
    let echo_args = platform == \"windows\" ? [\"/c\", \"echo\", \"hello_rasmalai\"] : [\"hello_rasmalai\"];\n\
    let output = Process.run(echo_cmd, echo_args);\n\
    assert(output.exitCode == 0, \"process run exit code 0\");\n\
    assert(output.stdoutText().contains(\"hello_rasmalai\"), \"process run stdout match\");\n\
    if (platform != \"windows\") {\n\
        let child = Process.spawn(\"cat\", [], new SpawnOptions(\n\
            null,\n\
            null,\n\
            Stdio.Piped,\n\
            Stdio.Piped,\n\
            Stdio.Inherit\n\
        ));\n\
        assert(child.pid > 0, \"child process pid positive\");\n\
        let write_buf = ByteBuffer.fromString(\"streaming_piped_data\\n\");\n\
        let bytes_written = child.writeStdin(write_buf);\n\
        assert(bytes_written == 21, \"bytes written to stdin\");\n\
        child.closeStdin();\n\
        let read_buf = ByteBuffer.allocate(64);\n\
        let bytes_read = child.readStdout(read_buf);\n\
        assert(bytes_read == 21, \"bytes read from stdout\");\n\
        assert(read_buf.readString(0, bytes_read).contains(\"streaming_piped_data\"), \"pipe echo match\");\n\
        let exit_code = child.wait();\n\
        assert(exit_code == 0, \"child process exited cleanly with code 0\");\n\
        let texter = Process.spawn(\"cat\", []);\n\
        assert(texter.writeStdinText(\"text_api\\n\") == 9, \"writeStdinText\");\n\
        texter.closeStdin();\n\
        assert(texter.readStdoutText().contains(\"text_api\"), \"readStdoutText\");\n\
        assert(texter.wait() == 0, \"texter wait\");\n\
        let sleeper = Process.spawn(\"sleep\", [\"30\"]);\n\
        assert(sleeper.tryWait() == null, \"sleeper running\");\n\
        assert(sleeper.kill(9), \"sleeper killed\");\n\
        assert(sleeper.wait() == 137, \"sigkill code\");\n\
        let env_map = new Map<String, String>();\n\
        env_map.set(\"RNX_KID_VAR\", \"kid_42\");\n\
        let env_opts = new SpawnOptions(null, env_map, Stdio.Piped, Stdio.Piped, Stdio.Piped);\n\
        let env_out = Process.run(\"sh\", [\"-c\", \"echo $RNX_KID_VAR\"], env_opts);\n\
        assert(env_out.exitCode == 0, \"env run code\");\n\
        assert(env_out.stdoutText().contains(\"kid_42\"), \"env override reaches child\");\n\
        let err_out = Process.run(\"sh\", [\"-c\", \"echo oops >&2\"]);\n\
        assert(err_out.exitCode == 0, \"stderr run code\");\n\
        assert(err_out.stderrText().contains(\"oops\"), \"stderr captured\");\n\
        let fail_out = Process.run(\"sh\", [\"-c\", \"exit 3\"]);\n\
        assert(fail_out.exitCode == 3, \"nonzero exit code\");\n\
        let parter = Process.spawn(\"cat\", []);\n\
        let pwb = ByteBuffer.fromString(\"hello world\");\n\
        assert(parter.writeStdin(pwb, 0, 5) == 5, \"partial stdin write\");\n\
        parter.closeStdin();\n\
        let prb = ByteBuffer.allocate(16);\n\
        assert(parter.readStdout(prb) == 5, \"partial stdout read\");\n\
        assert(prb.readString(0, 5) == \"hello\", \"partial echo\");\n\
        assert(parter.wait() == 0, \"partial wait\");\n\
        let termer = Process.spawn(\"sleep\", [\"30\"]);\n\
        assert(termer.kill(), \"sigterm default\");\n\
        assert(termer.wait() == 143, \"sigterm code\");\n\
        let cwd_opts = new SpawnOptions(\"/tmp\", null, Stdio.Piped, Stdio.Piped, Stdio.Piped);\n\
        let pwd_out = Process.run(\"sh\", [\"-c\", \"pwd\"], cwd_opts);\n\
        assert(pwd_out.exitCode == 0, \"pwd code\");\n\
        assert(pwd_out.stdoutText().contains(\"tmp\"), \"spawn cwd\");\n\
    }\n\
    return 0;\n\
}\n";

const EXIT_SRC: &str = "import { Process } from \"@std/process\";\n\
fn Main(): Int {\n\
    Process.exit(7);\n\
    return 0;\n\
}\n";

const CHDIR_SRC: &str = "import { Process } from \"@std/process\";\n\
import { OS } from \"@std/os\";\n\
fn Main(): Int {\n\
    let home = Process.cwd();\n\
    assert(home.length() > 0, \"start cwd\");\n\
    assert(Process.chdir(OS.tmpdir()), \"chdir tmp\");\n\
    assert(Process.cwd().length() > 0, \"tmp cwd\");\n\
    assert(!Process.chdir(\"/nonexistent_dir_rnx_zzz\"), \"chdir missing fails\");\n\
    assert(Process.chdir(home), \"chdir back\");\n\
    assert(Process.cwd() == home, \"cwd restored\");\n\
    return 0;\n\
}\n";

#[test]
fn process_os_all_backends() {
    check_all_backends(SCRIPT, 0, &[], "script");
}

#[test]
fn process_exit_aot_only() {
    check_aot_only(EXIT_SRC, 7, "exit");
}

#[test]
fn process_chdir_aot_only() {
    check_aot_only(CHDIR_SRC, 0, "chdir");
}
