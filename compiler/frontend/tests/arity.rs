use frontend::desugar::desugar;
use frontend::parser::Parser;
use frontend::semantic::check;

fn parse(src: &str) -> frontend::ast::Module {
    Parser::parse_module(src).unwrap_or_else(|e| panic!("{e}"))
}

fn codes(src: &str) -> Vec<String> {
    let mut m = parse(src);
    let mut out: Vec<String> = desugar(&mut m)
        .iter()
        .map(|d| d.code.as_str().to_string())
        .collect();
    out.extend(check(&m).iter().map(|d| d.code.as_str().to_string()));
    out
}

fn messages(src: &str) -> Vec<(String, String)> {
    let mut m = parse(src);
    let _ = desugar(&mut m);
    check(&m)
        .iter()
        .map(|d| (d.code.as_str().to_string(), d.message.clone()))
        .collect()
}

#[test]
fn extra_arg_on_fn_is_e108() {
    let got = messages(
        "fn add(a: Int, b: Int): Int { return a + b; } fn Main(): Int { print(add(1, 2, 3)); return 0; }",
    );
    assert!(
        got.iter()
            .any(|(c, m)| c == "E108" && m.contains("`add` takes 2 args")),
        "{got:?}"
    );
}

#[test]
fn missing_arg_on_fn_is_e108() {
    let got = messages(
        "fn add(a: Int, b: Int): Int { return a + b; } fn Main(): Int { print(add(1)); return 0; }",
    );
    assert!(
        got.iter()
            .any(|(c, m)| c == "E108" && m.contains("`add` takes 2 args")),
        "{got:?}"
    );
}

#[test]
fn exact_arity_is_clean() {
    let got = codes(
        "fn add(a: Int, b: Int): Int { return a + b; } fn Main(): Int { print(add(1, 2)); return 0; }",
    );
    assert!(got.is_empty(), "{got:?}");
}

#[test]
fn omitted_trailing_default_is_clean() {
    let got = codes(
        "fn configure(host: String, port: Int = 8080): String { return host; } fn Main(): Int { print(configure(\"api.dev\")); return 0; }",
    );
    assert!(got.is_empty(), "{got:?}");
}

#[test]
fn extra_arg_beyond_defaults_is_e108() {
    let got = messages(
        "fn configure(host: String, port: Int = 8080): String { return host; } fn Main(): Int { print(configure(\"a\", 1, 2)); return 0; }",
    );
    assert!(
        got.iter()
            .any(|(c, m)| c == "E108" && m.contains("`configure` takes 1..2 args")),
        "{got:?}"
    );
}

#[test]
fn extra_arg_on_init_is_e108() {
    let got = messages(
        "class Box { let x: Int = 0; init(x: Int) { this.x = x; } } fn Main(): Int { let b = new Box(1, 2, 3); return 0; }",
    );
    assert!(
        got.iter()
            .any(|(c, m)| c == "E108" && m.contains("`Box.init` takes 1 args")),
        "{got:?}"
    );
}

#[test]
fn missing_arg_on_init_is_e108() {
    let got = messages(
        "class Box { let x: Int = 0; init(x: Int) { this.x = x; } } fn Main(): Int { let b = new Box(); return 0; }",
    );
    assert!(
        got.iter()
            .any(|(c, m)| c == "E108" && m.contains("`Box.init` takes 1 args")),
        "{got:?}"
    );
}

#[test]
fn exact_init_arity_is_clean() {
    let got = codes(
        "class Box { let x: Int = 0; init(x: Int) { this.x = x; } } fn Main(): Int { let b = new Box(1); return 0; }",
    );
    assert!(got.is_empty(), "{got:?}");
}

#[test]
fn arity_error_points_at_call_site() {
    let src = "fn add(a: Int, b: Int): Int { return a + b; } fn Main(): Int { print(add(1)); return 0; }";
    let mut m = parse(src);
    let _ = desugar(&mut m);
    let diags = check(&m);
    let d = diags
        .iter()
        .find(|d| d.code.as_str() == "E108")
        .expect("E108");
    let span = d.span.expect("span");
    let call_at = src.find("add(1)").expect("call") as u32;
    assert!(span.start <= call_at && call_at < span.end, "{span:?}");
}

const CLOCK: &str = "class Clock { let seconds: Int = 0; init(seconds: Int) { this.seconds = seconds; } static fn base(): Int { return 60; } fn virtual(seconds: Int): Clock { return new Clock(seconds + Clock.base()); } }";

#[test]
fn instance_method_on_type_is_e108() {
    let got = messages(&format!("{CLOCK} fn Main(): Int {{ let c = Clock.virtual(0); return 0; }}"));
    assert!(
        got.iter().any(|(c, m)| c == "E108"
            && m.contains("`Clock.virtual`")
            && m.contains("instance method")),
        "{got:?}"
    );
}

#[test]
fn instance_method_on_type_points_at_call() {
    let src = format!("{CLOCK} fn Main(): Int {{ let c = Clock.virtual(0); return 0; }}");
    let mut m = parse(&src);
    let _ = desugar(&mut m);
    let diags = check(&m);
    let d = diags
        .iter()
        .find(|d| d.code.as_str() == "E108" && d.message.contains("instance method"))
        .expect("E108");
    let span = d.span.expect("span");
    let call_at = src.find("Clock.virtual(0)").expect("call") as u32;
    assert!(span.start <= call_at && call_at < span.end, "{span:?}");
}

#[test]
fn static_call_on_type_is_clean() {
    let got = codes(&format!("{CLOCK} fn Main(): Int {{ return Clock.base(); }}"));
    assert!(got.is_empty(), "{got:?}");
}

#[test]
fn instance_call_on_value_is_clean() {
    let got = codes(&format!(
        "{CLOCK} fn Main(): Int {{ let c = new Clock(0); let d = c.virtual(1); return d.seconds; }}"
    ));
    assert!(got.is_empty(), "{got:?}");
}

#[test]
fn struct_instance_method_on_type_is_e108() {
    let got = messages(
        "struct Point { let x: Int = 0; fn get(): Int { return this.x; } } fn Main(): Int { return Point.get(); }",
    );
    assert!(
        got.iter().any(|(c, m)| c == "E108"
            && m.contains("`Point.get`")
            && m.contains("instance method")),
        "{got:?}"
    );
}

const TASKS: &str = "class Task { let id: Int = 0; init(id: Int) { this.id = id; } fn run(): Int { return this.id; } fn runWith(x: Int): Int { return this.id + x; } }";

#[test]
fn index_receiver_arity_checked() {
    let got = messages(&format!(
        "{TASKS} fn Main(): Int {{ let tasks: Array<Task> = []; return tasks[0].runWith(); }}"
    ));
    assert!(
        got.iter()
            .any(|(c, m)| c == "E108" && m.contains("`Task.runWith`")),
        "{got:?}"
    );
}

#[test]
fn index_receiver_unknown_method_is_e108() {
    let got = messages(&format!(
        "{TASKS} fn Main(): Int {{ let tasks: Array<Task> = []; return tasks[0].nope(); }}"
    ));
    assert!(
        got.iter()
            .any(|(c, m)| c == "E108" && m.contains("unknown method `nope`")),
        "{got:?}"
    );
}

#[test]
fn index_receiver_correct_arity_is_clean() {
    let got = codes(&format!(
        "{TASKS} fn Main(): Int {{ let tasks: Array<Task> = []; return tasks[0].runWith(1); }}"
    ));
    assert!(got.is_empty(), "{got:?}");
}

#[test]
fn index_receiver_unannotated_stays_clean() {
    let got = codes(
        "fn Main(): Int { let xs = []; xs.push(1); return xs[0].whatever(1, 2); }",
    );
    assert!(got.is_empty(), "{got:?}");
}

const GALLERY: &str = "enum Color { Red, Green, Blue } class Canvas { let mode: Int = 0; fn setMode(c: Color): Int { return this.mode; } }";

#[test]
fn index_receiver_enum_hint_is_clean() {
    let got = codes(&format!(
        "{GALLERY} fn Main(): Int {{ let cs: Array<Canvas> = []; return cs[0].setMode(.Blue); }}"
    ));
    assert!(got.is_empty(), "{got:?}");
}

#[test]
fn index_receiver_bad_enum_variant_is_e108() {
    let got = messages(&format!(
        "{GALLERY} fn Main(): Int {{ let cs: Array<Canvas> = []; return cs[0].setMode(.Nope); }}"
    ));
    assert!(
        got.iter()
            .any(|(c, m)| c == "E108" && m.contains("unknown variant")),
        "{got:?}"
    );
}
