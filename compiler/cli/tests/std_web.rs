use std::path::PathBuf;

fn resolve_src(src: &str, tag: &str) -> (lir::instr::Module, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rnx-web-{tag}-{}", std::process::id()));
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

const PRELUDE: &str = "import { URL, Headers, URLSearchParams, HttpStatus, statusCode, statusReason, statusFromCode, encodeComponent, decodeComponent, isSuccess, isClientError, isServerError } from \"@std/web\";\n";

#[test]
fn status_codes_round_trip() {
    check_all_backends(
        &format!("{PRELUDE}fn Main(): Int {{\n\
        assert(statusCode(HttpStatus.Ok) == 200, \"ok\");\n\
        assert(statusCode(HttpStatus.NotFound) == 404, \"not found\");\n\
        assert(statusCode(HttpStatus.GatewayTimeout) == 504, \"timeout\");\n\
        assert(statusReason(HttpStatus.Ok) == \"OK\", \"reason ok\");\n\
        assert(statusReason(HttpStatus.NotFound) == \"Not Found\", \"reason 404\");\n\
        assert((statusCode(statusFromCode(201) ?? HttpStatus.Ok)) == 201, \"from 201\");\n\
        assert(statusFromCode(999) == null, \"unknown none\");\n\
        assert(isSuccess(200) && isSuccess(204) && !isSuccess(404), \"success\");\n\
        assert(isClientError(404) && !isClientError(200), \"client\");\n\
        assert(isServerError(503) && !isServerError(302), \"server\");\n\
        return 0;\n\
        }}\n"),
        0,
        &[],
        "status",
    );
}

#[test]
fn headers_case_insensitive_crud() {
    check_all_backends(
        &format!("{PRELUDE}fn Main(): Int {{\n\
        let h = new Headers();\n\
        assert(h.len() == 0, \"empty\");\n\
        h.set(\"Content-Type\", \"text/plain\");\n\
        assert((h.get(\"content-type\") ?? \"\") == \"text/plain\", \"lower\");\n\
        assert((h.get(\"CONTENT-TYPE\") ?? \"\") == \"text/plain\", \"upper\");\n\
        assert(h.has(\"Content-Type\"), \"has\");\n\
        h.set(\"Content-Type\", \"text/html\");\n\
        assert(h.len() == 1, \"overwrite keeps one\");\n\
        assert((h.get(\"content-type\") ?? \"\") == \"text/html\", \"overwritten\");\n\
        assert(h.delete(\"CONTENT-type\"), \"delete\");\n\
        assert(!h.has(\"content-type\"), \"gone\");\n\
        assert(!h.delete(\"content-type\"), \"delete missing\");\n\
        let m = Headers.fromPairs([[\"X-A\", \"1\"], [\"X-B\", \"2\"], [\"short\"]]);\n\
        assert(m.len() == 2, \"pairs skip short\");\n\
        assert(m.keys().length() == 2, \"keys\");\n\
        assert(m.values()[1] == \"2\", \"values\");\n\
        return 0;\n\
        }}\n"),
        0,
        &[],
        "headers",
    );
}

#[test]
fn url_absolute_parse() {
    check_all_backends(
        &format!("{PRELUDE}fn Main(): Int {{\n\
        let u = new URL(\"https://example.com:8080/a/b?x=1#frag\");\n\
        assert(u.href() == \"https://example.com:8080/a/b?x=1#frag\", \"href\");\n\
        assert(u.protocol() == \"https:\", \"protocol\");\n\
        assert(u.hostname() == \"example.com\", \"host\");\n\
        assert(u.host() == \"example.com:8080\", \"hostport\");\n\
        assert(u.port() == \"8080\", \"port\");\n\
        assert(u.pathname() == \"/a/b\", \"path\");\n\
        assert(u.search() == \"?x=1\", \"search\");\n\
        assert(u.hash() == \"#frag\", \"hash\");\n\
        assert(u.origin() == \"https://example.com:8080\", \"origin\");\n\
        let d = new URL(\"http://example.com/a\");\n\
        assert(d.href() == \"http://example.com/a\", \"no port\");\n\
        let p = new URL(\"https://example.com:443/a\");\n\
        assert(p.href() == \"https://example.com/a\", \"default port dropped\");\n\
        assert(p.host() == \"example.com\", \"host no port\");\n\
        let q = new URL(\"https://EXAMPLE.com/A\");\n\
        assert(q.hostname() == \"example.com\", \"host lowered\");\n\
        assert(q.pathname() == \"/A\", \"path case kept\");\n\
        return 0;\n\
        }}\n"),
        0,
        &[],
        "absolute",
    );
}

#[test]
fn url_relative_resolution() {
    check_all_backends(
        &format!("{PRELUDE}fn Main(): Int {{\n\
        let b = \"https://example.com/a/b?x=1#frag\";\n\
        assert(new URL(\"../c?y=2\", b).href() == \"https://example.com/c?y=2\", \"dotdot\");\n\
        assert(new URL(\"/root\", b).href() == \"https://example.com/root\", \"root\");\n\
        assert(new URL(\"?z=3\", b).href() == \"https://example.com/a/b?z=3\", \"query only\");\n\
        assert(new URL(\"#n2\", b).href() == \"https://example.com/a/b?x=1#n2\", \"frag only\");\n\
        assert(new URL(\"//other.com/p\", b).href() == \"https://other.com/p\", \"protocol relative\");\n\
        assert(new URL(\"./sib\", b).href() == \"https://example.com/a/sib\", \"dot\");\n\
        assert(new URL(\"https://a.com/x\", b).href() == \"https://a.com/x\", \"absolute wins\");\n\
        let sp = new URL(\"https://h.com/p?a=1&b=2\").searchParams();\n\
        assert((sp.get(\"b\") ?? \"\") == \"2\", \"search params\");\n\
        return 0;\n\
        }}\n"),
        0,
        &[],
        "relative",
    );
}

#[test]
fn url_mutation() {
    check_all_backends(
        &format!("{PRELUDE}fn Main(): Int {{\n\
        let u = new URL(\"https://example.com/a?x=1#f\");\n\
        u.setPathname(\"/b/c\");\n\
        assert(u.pathname() == \"/b/c\", \"path set\");\n\
        u.setSearch(\"?y=2\");\n\
        assert(u.search() == \"?y=2\", \"search set\");\n\
        u.setHash(\"n\");\n\
        assert(u.hash() == \"#n\", \"hash set\");\n\
        assert(u.href() == \"https://example.com/b/c?y=2#n\", \"href rebuilt\");\n\
        u.setSearch(\"\");\n\
        assert(u.search() == \"\", \"search cleared\");\n\
        return 0;\n\
        }}\n"),
        0,
        &[],
        "mutate",
    );
}

#[test]
fn url_non_ascii_native_literals() {
    check_all_backends(
        &format!("{PRELUDE}fn Main(): Int {{\n\
        let u = new URL(\"http://127.0.0.1:8080/café?tag=🚀\");\n\
        assert(u.hostname() == \"127.0.0.1\", \"host\");\n\
        assert(u.port() == \"8080\", \"port\");\n\
        assert(u.pathname() == \"/café\", \"accent path\");\n\
        assert((u.searchParams().get(\"tag\") ?? \"\") == \"🚀\", \"emoji param\");\n\
        assert(u.href() == \"http://127.0.0.1:8080/café?tag=🚀\", \"href round trip\");\n\
        let e = encodeComponent(\"café 🚀\", false);\n\
        assert(e == \"caf%C3%A9%20%F0%9F%9A%80\", \"encode\");\n\
        assert(decodeComponent(e, false) == \"café 🚀\", \"decode round trip\");\n\
        return 0;\n\
        }}\n"),
        0,
        &[],
        "unicode",
    );
}

#[test]
fn params_crud_and_encoding() {
    check_all_backends(
        &format!("{PRELUDE}fn Main(): Int {{\n\
        let p = new URLSearchParams(\"a=1&a=2&b=3\");\n\
        assert(p.size() == 3, \"size\");\n\
        assert((p.get(\"a\") ?? \"\") == \"1\", \"first\");\n\
        assert(p.getAll(\"a\").length() == 2, \"all\");\n\
        assert(p.get(\"missing\") == null, \"none\");\n\
        assert(p.has(\"b\") && !p.has(\"z\"), \"has\");\n\
        p.set(\"a\", \"9\");\n\
        assert(p.getAll(\"a\").length() == 1 && (p.get(\"a\") ?? \"\") == \"9\", \"set\");\n\
        p.append(\"a\", \"10\");\n\
        assert(p.getAll(\"a\").length() == 2, \"append\");\n\
        assert(p.delete(\"b\"), \"delete\");\n\
        assert(!p.has(\"b\"), \"gone\");\n\
        let q = new URLSearchParams(\"b=2&a=1&b=1\");\n\
        q.sort();\n\
        assert(q.toString() == \"a=1&b=2&b=1\", \"sorted\");\n\
        let e = new URLSearchParams();\n\
        e.append(\"q\", \"a b&c\");\n\
        assert(e.toString() == \"q=a+b%26c\", \"form encoded\");\n\
        let d = new URLSearchParams(e.toString());\n\
        assert((d.get(\"q\") ?? \"\") == \"a b&c\", \"round trip\");\n\
        let u = new URLSearchParams(\"n=%C3%A9\");\n\
        assert((u.get(\"n\") ?? \"\") == \"é\", \"utf8 decode\");\n\
        return 0;\n\
        }}\n"),
        0,
        &[],
        "params",
    );
}

#[test]
fn params_iteration() {
    check_all_backends(
        &format!("{PRELUDE}fn Main(): Int {{\n\
        let p = new URLSearchParams(\"x=1&y=2\");\n\
        let names: Array<String> = [];\n\
        let vals: Array<String> = [];\n\
        for pair in p {{\n\
        names.push(pair[0]);\n\
        vals.push(pair[1]);\n\
        }}\n\
        assert(names.length() == 2, \"iter count\");\n\
        assert(names[0] == \"x\" && vals[1] == \"2\", \"iter order\");\n\
        let e = p.entries();\n\
        assert(e.length() == 2 && e[1][0] == \"y\", \"entries\");\n\
        assert(p.keys()[0] == \"x\" && p.values()[1] == \"2\", \"keys values\");\n\
        return 0;\n\
        }}\n"),
        0,
        &[],
        "iterate",
    );
}

#[test]
fn percent_codec_edges() {
    check_all_backends(
        &format!("{PRELUDE}fn Main(): Int {{\n\
        assert(encodeComponent(\"abc-_.~123\", false) == \"abc-_.~123\", \"unreserved\");\n\
        assert(encodeComponent(\"a b\", true) == \"a+b\", \"form space\");\n\
        assert(encodeComponent(\"a b\", false) == \"a%20b\", \"url space\");\n\
        assert(decodeComponent(\"a+b\", true) == \"a b\", \"plus\");\n\
        assert(decodeComponent(\"a+b\", false) == \"a+b\", \"plus kept\");\n\
        assert(decodeComponent(\"%41%42\", false) == \"AB\", \"hex\");\n\
        assert(decodeComponent(\"%zz\", false) == \"%zz\", \"bad passthrough\");\n\
        assert(decodeComponent(\"100%\", false) == \"100%\", \"lone percent\");\n\
        assert(decodeComponent(\"%C3%A9\", false) == \"é\", \"multibyte\");\n\
        assert(decodeComponent(\"%E2%82%AC\", false) == \"€\", \"euro\");\n\
        return 0;\n\
        }}\n"),
        0,
        &[],
        "codec",
    );
}
