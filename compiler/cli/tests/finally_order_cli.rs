#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

use std::process::Command;

fn run(backend: &str, dir: &std::path::Path) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_rnx"))
        .arg("run")
        .arg("--backend")
        .arg(backend)
        .arg(dir.join("main.rnx"))
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{backend}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

fn run_aot(dir: &std::path::Path) -> String {
    let build = Command::new(env!("CARGO_BIN_EXE_rnx"))
        .arg("build")
        .arg(dir.join("main.rnx"))
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "aot build: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let artifact = String::from_utf8_lossy(&build.stdout)
        .lines()
        .rev()
        .find_map(|l| l.strip_prefix("artifact: "))
        .map(|s| s.trim().trim_end_matches(" (fresh)").to_string())
        .expect("build prints the artifact path");
    let out = Command::new(artifact).output().unwrap();
    assert!(
        out.status.success(),
        "aot run: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

fn check_all_backends(src: &str, want: &str, tag: &str) {
    let dir = crate::scratch::dir(&format!("finally-order-{tag}"));
    std::fs::write(dir.join("main.rnx"), src).unwrap();
    for backend in ["interpreter", "cranelift", "llvm"] {
        let got = run(backend, &dir);
        assert_eq!(got, want, "{backend} {tag}");
    }
    let aot = run_aot(&dir);
    assert_eq!(aot, want, "aot {tag}");
}

#[test]
fn finally_runs_after_catch_and_before_following_code() {
    check_all_backends(
        "fn boom(n: Int): Int throws {\n\
        if n > 0 { throw \"bad\" }\n\
        return 1\n\
        }\n\
        fn Main(): Int {\n\
        try {\n\
        let x = boom(1)\n\
        println!(\"no throw {x}\")\n\
        } catch (e) {\n\
        println!(\"caught\")\n\
        } finally {\n\
        println!(\"finally\")\n\
        }\n\
        println!(\"after try\")\n\
        let y = boom(0)\n\
        println!(\"y={y}\")\n\
        return 0\n\
        }\n",
        "caught\nfinally\nafter try\ny=1\n",
        "catch",
    );
}

#[test]
fn finally_runs_before_a_return_inside_try() {
    check_all_backends(
        "fn pick(): Int {\n\
        try {\n\
        println!(\"try\")\n\
        return 7\n\
        } finally {\n\
        println!(\"finally\")\n\
        }\n\
        }\n\
        fn Main(): Int {\n\
        let v = pick()\n\
        println!(\"v={v}\")\n\
        return 0\n\
        }\n",
        "try\nfinally\nv=7\n",
        "return",
    );
}

#[test]
fn finally_runs_before_a_rethrow_leaves_the_catch() {
    check_all_backends(
        "fn nested(): Int throws {\n\
        try {\n\
        try {\n\
        throw \"inner\"\n\
        } catch (e) {\n\
        println!(\"rethrow\")\n\
        throw\n\
        } finally {\n\
        println!(\"finally\")\n\
        }\n\
        return 1\n\
        } catch (e) {\n\
        println!(\"outer caught\")\n\
        return 2\n\
        }\n\
        }\n\
        fn Main(): Int {\n\
        let r = nested()\n\
        println!(\"r={r}\")\n\
        return 0\n\
        }\n",
        "rethrow\nfinally\nouter caught\nr=2\n",
        "rethrow",
    );
}

#[test]
fn finally_runs_before_continue_leaves_the_try() {
    check_all_backends(
        "fn Main(): Int {\n\
        for i in 0..3 {\n\
        try {\n\
        if i == 1 { continue }\n\
        println!(\"body {i}\")\n\
        } finally {\n\
        println!(\"finally {i}\")\n\
        }\n\
        }\n\
        println!(\"done\")\n\
        return 0\n\
        }\n",
        "body 0\nfinally 0\nfinally 1\nbody 2\nfinally 2\ndone\n",
        "continue",
    );
}

#[test]
fn finally_runs_before_question_mark_propagates() {
    check_all_backends(
        "fn risky(fail: Bool): Result<Int, String> {\n\
        if fail { return Err(\"boom\") }\n\
        return Ok(41)\n\
        }\n\
        fn work(fail: Bool): Result<Int, String> {\n\
        try {\n\
        let x = risky(fail)?\n\
        println!(\"try ok\")\n\
        return Ok(x + 1)\n\
        } finally {\n\
        println!(\"cleanup\")\n\
        }\n\
        }\n\
        fn Main(): Int {\n\
        let ok = work(false)\n\
        println!(ok.unwrapOr(-1))\n\
        let bad = work(true)\n\
        switch bad {\n\
        case .Ok(v): println!(v); pass\n\
        case .Err(e): println!(\"caught {e}\"); pass\n\
        }\n\
        return 0\n\
        }\n",
        "try ok\ncleanup\n42\ncleanup\ncaught boom\n",
        "question",
    );
}

#[test]
fn finally_runs_around_question_mark_in_nested_try() {
    check_all_backends(
        "fn risky(fail: Bool): Result<Int, String> {\n\
        if fail { return Err(\"boom\") }\n\
        return Ok(41)\n\
        }\n\
        fn inner(fail: Bool): Result<Int, String> {\n\
        try {\n\
        try {\n\
        let x = risky(fail)?\n\
        println!(\"inner body\")\n\
        return Ok(x)\n\
        } finally {\n\
        println!(\"inner cleanup\")\n\
        }\n\
        } finally {\n\
        println!(\"outer cleanup\")\n\
        }\n\
        }\n\
        fn Main(): Int {\n\
        let ok = inner(false)\n\
        println!(ok.unwrapOr(-1))\n\
        let bad = inner(true)\n\
        switch bad {\n\
        case .Ok(v): println!(v); pass\n\
        case .Err(e): println!(\"caught {e}\"); pass\n\
        }\n\
        return 0\n\
        }\n",
        "inner body\ninner cleanup\nouter cleanup\n41\ninner cleanup\nouter cleanup\ncaught boom\n",
        "nested",
    );
}

#[test]
fn finally_runs_before_question_mark_leaves_the_catch() {
    check_all_backends(
        "fn risky(fail: Bool): Result<Int, String> {\n\
        if fail { return Err(\"boom\") }\n\
        return Ok(41)\n\
        }\n\
        fn work(): Result<Int, String> {\n\
        try {\n\
        throw \"first\"\n\
        } catch (e) {\n\
        println!(\"caught {e}\")\n\
        let x = risky(true)?\n\
        return Ok(x)\n\
        } finally {\n\
        println!(\"cleanup\")\n\
        }\n\
        }\n\
        fn Main(): Int {\n\
        let r = work()\n\
        switch r {\n\
        case .Ok(v): println!(v); pass\n\
        case .Err(e): println!(\"caught {e}\"); pass\n\
        }\n\
        return 0\n\
        }\n",
        "caught first\ncleanup\ncaught boom\n",
        "catch-question",
    );
}

#[test]
fn finally_does_not_evaluate_the_return_value_twice() {
    check_all_backends(
        "fn risky(fail: Bool): Result<Int, String> {\n\
        if fail { return Err(\"boom\") }\n\
        return Ok(41)\n\
        }\n\
        fn bump(log: Array<Int>): Int {\n\
        log.push(log.length())\n\
        return log.length()\n\
        }\n\
        fn work(log: Array<Int>): Result<Int, String> {\n\
        try {\n\
        let x = risky(false)?\n\
        return Ok(bump(log) + x)\n\
        } finally {\n\
        println!(\"cleanup\")\n\
        }\n\
        }\n\
        fn Main(): Int {\n\
        let log: Array<Int> = []\n\
        let r = work(log)\n\
        println!(r.unwrapOr(-1))\n\
        println!(log.length())\n\
        return 0\n\
        }\n",
        "cleanup\n42\n1\n",
        "once",
    );
}

#[test]
fn finally_keeps_the_question_mark_payload_type() {
    check_all_backends(
        "fn text(fail: Bool): Result<String, String> {\n\
        if fail { return Err(\"boom\") }\n\
        return Ok(\"hi\")\n\
        }\n\
        fn work(fail: Bool): Result<String, String> {\n\
        try {\n\
        let s = text(fail)?\n\
        println!(s.length())\n\
        return Ok(s)\n\
        } finally {\n\
        println!(\"cleanup\")\n\
        }\n\
        }\n\
        fn Main(): Int {\n\
        println!(work(false).unwrapOr(\"bad\"))\n\
        println!(work(true).unwrapOr(\"bad\"))\n\
        return 0\n\
        }\n",
        "2\ncleanup\nhi\ncleanup\nbad\n",
        "payload-type",
    );
}
