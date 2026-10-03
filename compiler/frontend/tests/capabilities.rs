use frontend::capabilities::{
    analyze, analyze_with_fuel, Capability, CapabilityTier, SecurityDiagnostic,
};
use frontend::parser::Parser;
use std::str::FromStr;

fn report(src: &str) -> frontend::capabilities::CapabilityAnalysisReport {
    let module = Parser::parse_module(src).unwrap_or_else(|e| panic!("parse failed: {e}"));
    analyze(&module, src, "pkg.rnx").unwrap_or_else(|e| panic!("analysis failed: {e}"))
}

fn caps(src: &str) -> Vec<String> {
    let mut out: Vec<String> = report(src)
        .capabilities
        .iter()
        .map(|c| c.to_string())
        .collect();
    out.sort();
    out
}

#[test]
fn pure_library_yields_no_capabilities() {
    let src = "fn add(a: Int, b: Int): Int {\n    return a + b;\n}\nfn Main(): Int {\n    return add(1, 2);\n}\n";
    let r = report(src);
    assert_eq!(r.tier, CapabilityTier::Pure);
    assert!(r.capabilities.is_empty(), "{:?}", r.capabilities);
    assert!(r.traces.is_empty(), "{:?}", r.traces);
}

#[test]
fn delegated_file_write_through_param() {
    let src = "fn writeData(path: String, data: String): Int {\n    File.write(path, data);\n    return 0;\n}\n";
    let r = report(src);
    assert_eq!(r.tier, CapabilityTier::Delegated);
    assert_eq!(r.capabilities, vec![Capability::FsDelegated]);
    assert_eq!(caps(src), vec!["fs:delegated"]);
    assert_eq!(r.traces.len(), 1);
    let chain = &r.traces[0];
    assert!(chain.is_delegated);
    assert_eq!(chain.nodes.len(), 2);
    assert_eq!(chain.nodes[0].symbol, "writeData");
    assert_eq!(chain.nodes[0].file, "pkg.rnx");
    assert_eq!(chain.nodes[0].line, 1);
    let sink = chain.nodes.last().unwrap();
    assert_eq!(sink.symbol, "File.write");
    assert_eq!(sink.line, 2);
    assert_eq!(sink.col, 5);
    assert!(sink.expression_snippet.contains("File.write"));
}

#[test]
fn mutated_path_reports_s201() {
    let src = "fn writeData(path: String): Int {\n    File.write(path + \"/hack\", \"\");\n    return 0;\n}\n";
    let module = Parser::parse_module(src).unwrap();
    match analyze(&module, src, "pkg.rnx") {
        Err(SecurityDiagnostic::S201 { message, span }) => {
            assert!(message.contains("untrusted path mutation"), "{message}");
            assert!(message.contains("fs:write"), "{message}");
            assert!(message.contains("File.write"), "{message}");
            assert!(span.start > 0, "S201 carries the sink span");
        }
        other => panic!("expected S201, got {other:?}"),
    }
}

#[test]
fn discord_fetch_pattern_yields_scoped_net() {
    let src = "fn postMsg(token: String): Int {\n    fetch(\"https://discord.com/api/v10/channels\");\n    return 0;\n}\n";
    let r = report(src);
    assert_eq!(r.tier, CapabilityTier::Ambient);
    assert_eq!(caps(src), vec!["net:http:https://discord.com/api/v10/*"]);
}

#[test]
fn process_spawn_yields_hazard() {
    let src = "fn shrink(png: String): Int {\n    Process.spawn(\"oxipng\", []);\n    return 0;\n}\n";
    let r = report(src);
    assert_eq!(r.tier, CapabilityTier::Hazard);
    assert_eq!(caps(src), vec!["sys:exec:oxipng"]);
}

#[test]
fn native_and_unsafe_yield_hazard() {
    let src = "import { fn puts(s: String): Int } from native \"c\"\nfn Main(): Int {\n    unsafe {\n        Pointer.read(0);\n    }\n    return 0;\n}\n";
    let r = report(src);
    assert_eq!(r.tier, CapabilityTier::Hazard);
    assert_eq!(caps(src), vec!["unsafe:ffi", "unsafe:raw_memory"]);
}

#[test]
fn circular_calls_terminate() {
    let src = "fn a(): Int {\n    return b();\n}\nfn b(): Int {\n    return a();\n}\nfn Main(): Int {\n    return a();\n}\n";
    let r = report(src);
    assert_eq!(r.tier, CapabilityTier::Pure);
}

#[test]
fn fuel_exhaustion_returns_s501() {
    let src = "fn add(a: Int, b: Int): Int {\n    return a + b;\n}\nfn Main(): Int {\n    return add(1, 2);\n}\n";
    let module = Parser::parse_module(src).unwrap();
    match analyze_with_fuel(&module, src, "pkg.rnx", 5) {
        Err(SecurityDiagnostic::S501 { fuel_limit }) => assert_eq!(fuel_limit, 5),
        other => panic!("expected S501, got {other:?}"),
    }
}

#[test]
fn capability_display_and_parse_round_trip() {
    let cases = [
        "fs:read:/tmp/**",
        "fs:write:/var/data/**",
        "fs:delegated",
        "net:http:https://discord.com/api/v10/*",
        "net:ws:wss://gateway.discord.gg/*",
        "net:delegated",
        "sys:exec:ffmpeg",
        "env:read:DISCORD_TOKEN",
        "env:read:*",
        "unsafe:ffi",
        "unsafe:raw_memory",
    ];
    for s in cases {
        let parsed = Capability::from_str(s).unwrap_or_else(|e| panic!("parse {s}: {e}"));
        assert_eq!(parsed.to_string(), s, "round trip {s}");
    }
    assert_eq!(
        Capability::from_str("env:dump").unwrap(),
        Capability::EnvDump
    );
    assert!(Capability::from_str("bogus:cap").is_err());
}

#[test]
fn origin_snippet_is_signature_only() {
    let src = "fn loadData(path: String): Int {\n    File.open(path, FileMode.Read);\n    return 0;\n}\n";
    let r = report(src);
    assert_eq!(r.traces.len(), 1);
    let origin = &r.traces[0].nodes[0];
    assert_eq!(origin.symbol, "loadData");
    assert_eq!(origin.expression_snippet, "fn loadData(path: String): Int");
    assert!(!origin.expression_snippet.contains('{'));
    assert!(!origin.expression_snippet.contains("return"));
}

#[test]
fn stdio_from_handle_needs_term_read_and_write() {
    let src = "fn Main(): Int {\n    File.fromHandle(1, \"stdout\", .Write);\n    return 0;\n}\n";
    let r = report(src);
    assert_eq!(r.tier, CapabilityTier::Ambient);
    assert_eq!(caps(src), vec!["term:read", "term:write"]);
}

#[test]
fn file_instance_text_io_needs_term_caps() {
    let src = "fn Main(): Int {\n    out.writeText(\"hi\");\n    inn.readText();\n    return 0;\n}\n";
    let r = report(src);
    assert_eq!(r.tier, CapabilityTier::Ambient);
    assert_eq!(caps(src), vec!["term:read", "term:write"]);
}
