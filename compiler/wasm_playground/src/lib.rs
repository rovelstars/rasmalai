use diagnostics::theme::AuraTheme;
use diagnostics::tree_hook::{TreeHook, render};
use diagnostics::{line_col, Diagnostic};
use frontend::ast as A;
use frontend::token::TokenKind;
use runtime::machine::{ExecError, Machine};
use wasm_bindgen::prelude::*;

fn prepare(source: &str) -> Result<A::Module, Diagnostic> {
    frontend::modules::ModuleGraph::from_source(source)
}
fn check_merged(source: &str) -> Result<(Vec<Diagnostic>, Vec<Diagnostic>), Diagnostic> {
    let mut module = prepare(source)?;
    let mut diags = frontend::desugar::desugar(&mut module);
    diags.extend(frontend::semantic::check(&module));
    let mut warnings = Vec::new();
    let mut errors = Vec::new();
    for d in diags {
        if d.code.is_warning() {
            warnings.push(d);
        } else {
            errors.push(d);
        }
    }
    Ok((warnings, errors))
}

fn role_of(kind: &TokenKind) -> &'static str {
    match kind {
        TokenKind::Fn
        | TokenKind::Let
        | TokenKind::Const
        | TokenKind::Return
        | TokenKind::Public
        | TokenKind::Private
        | TokenKind::Defer
        | TokenKind::Unsafe
        | TokenKind::Static
        | TokenKind::If
        | TokenKind::Else
        | TokenKind::For
        | TokenKind::While
        | TokenKind::In
        | TokenKind::Break
        | TokenKind::Continue
        | TokenKind::Switch
        | TokenKind::Case
        | TokenKind::Default
        | TokenKind::Throw
        | TokenKind::Throws
        | TokenKind::Try
        | TokenKind::Catch
        | TokenKind::Finally
        | TokenKind::Guard
        | TokenKind::Do
        | TokenKind::Fallthrough
        | TokenKind::Is
        | TokenKind::Import
        | TokenKind::From
        | TokenKind::As
        | TokenKind::Async
        | TokenKind::Await => "keyword",
        TokenKind::Class
        | TokenKind::Struct
        | TokenKind::Record
        | TokenKind::Trait
        | TokenKind::Enum
        | TokenKind::Init
        | TokenKind::Deinit
        | TokenKind::OnReload
        | TokenKind::Extends
        | TokenKind::With
        | TokenKind::Comptime
        | TokenKind::Native => "keyword",
        TokenKind::Int(_)
        | TokenKind::Float(_)
        | TokenKind::StrText(_)
        | TokenKind::StrOpen
        | TokenKind::StrClose
        | TokenKind::True
        | TokenKind::False
        | TokenKind::Null => "literal",
        TokenKind::Ident(name) if is_type_name(name) => "type",
        TokenKind::Ident(_) | TokenKind::This => "text",
        _ => "plain",
    }
}

fn is_type_name(word: &str) -> bool {
    matches!(
        word,
        "Int"
            | "Float"
            | "FastFloat"
            | "Bool"
            | "Void"
            | "String"
            | "Any"
            | "Array"
            | "Map"
            | "Set"
            | "GenRef"
            | "Vec4f"
            | "Vec4i"
            | "Vec2"
            | "Option"
            | "Result"
    )
}

fn token_text<'a>(src: &'a str, start: u32, end: u32) -> &'a str {
    let start = (start as usize).min(src.len());
    let end = (end as usize).min(src.len()).max(start);
    src.get(start..end).unwrap_or("")
}

fn diag_hook(src: &str, diag: &Diagnostic) -> String {
    let theme = AuraTheme::plain();
    let kind = if diag.code.is_warning() { "warning" } else { "error" };
    let (line, col) = diag.span.map(|s| line_col(src, s.start)).unwrap_or((1, 1));
    let lines: Vec<&str> = src.lines().collect();
    let mut context = Vec::new();
    if !lines.is_empty() {
        let lo = line.saturating_sub(2).max(1);
        let hi = (line + 1).min(lines.len());
        for n in lo..=hi {
            context.push((n, lines[n - 1].to_string()));
        }
    }
    render(
        &theme,
        &TreeHook {
            title: format!("{kind}[{}]: {}", diag.code, diag.message),
            location: Some(format!("line {line}, column {col}")),
            context,
            pointer_line: line,
            pointer_col: col.saturating_sub(1),
            got: None,
            expected: None,
            footer: diag.hint.clone(),
            footer2: None,
        },
    )
}

#[wasm_bindgen]
pub fn tokenize(source: &str) -> String {
    let toks = match frontend::lexer::lex(source) {
        Ok(toks) => toks,
        Err(e) => {
            return format!(
                "{{\"error\":\"{}\"}}",
                diagnostics::escape_json(&e.to_string())
            );
        }
    };
    let mut out = String::from("{\"tokens\":[");
    for (i, tok) in toks.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&format!(
            "{{\"role\":\"{}\",\"text\":\"{}\",\"start\":{},\"end\":{}}}",
            role_of(&tok.kind),
            diagnostics::escape_json(token_text(source, tok.span.start, tok.span.end)),
            tok.span.start,
            tok.span.end,
        ));
    }
    out.push_str("]}");
    out
}

#[wasm_bindgen]
pub fn check(source: &str) -> String {
    let (warnings, errors) = match check_merged(source) {
        Ok(pair) => pair,
        Err(e) => return diag_hook(source, &e),
    };
    if warnings.is_empty() && errors.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for diag in errors.iter().chain(warnings.iter()) {
        out.push_str(&diag_hook(source, diag));
    }
    out
}

#[wasm_bindgen]
pub fn run(source: &str) -> String {
    let (warnings, errors) = match check_merged(source) {
        Ok(pair) => pair,
        Err(e) => return diag_hook(source, &e),
    };
    if !errors.is_empty() {
        let mut out = String::new();
        for diag in errors.iter().chain(warnings.iter()) {
            out.push_str(&diag_hook(source, diag));
        }
        return out;
    }
    let mut module = match prepare(source) {
        Ok(m) => m,
        Err(e) => return diag_hook(source, &e),
    };
    frontend::desugar::desugar(&mut module);
    let mut lowered = match lir::lower::lower(&module) {
        Ok(l) => l,
        Err(e) => return diag_hook(source, &e),
    };
    let entry = if lowered.functions.iter().any(|f| f.name == "Main") {
        "Main"
    } else {
        "main"
    };
    lir::opt::optimize_lir(&mut lowered, 1, entry);
    let leaked: &'static lir::instr::Module = Box::leak(Box::new(lowered));
    let mut machine = Machine::new(leaked);
    let mut out = String::new();
    match machine.call(entry, Vec::new()) {
        Ok(v) => {
            for line in &machine.output {
                out.push_str(line);
                out.push('\n');
            }
            out.push_str(&format!("=> {}", v.display()));
        }
        Err(ExecError::Throw(v)) => {
            out.push_str(&format!("thrown: {}", v.display()));
        }
        Err(ExecError::Fatal(m)) => {
            out.push_str(&format!("fatal: {m}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenize_emits_aura_roles() {
        let json = tokenize("fn main(): Int { return 42; }");
        assert!(json.contains("\"role\":\"keyword\",\"text\":\"fn\""), "{json}");
        assert!(json.contains("\"role\":\"type\",\"text\":\"Int\""), "{json}");
        assert!(json.contains("\"role\":\"literal\",\"text\":\"42\""), "{json}");
    }

    #[test]
    fn check_empty_for_valid_source() {
        assert_eq!(check("fn main(): Int { return 1; }"), String::new());
    }

    #[test]
    fn check_renders_tree_hook_for_errors() {
        let out = check("fn main(: Int { return 1; }");
        assert!(out.contains("┌─"), "{out}");
    }

    #[test]
    fn run_executes_main_and_returns_value() {
        let out = run("fn main(): Int { return 42; }");
        assert!(out.contains("=> 42"), "{out}");
    }

    #[test]
    fn run_aborts_on_assignment_type_mismatch_without_executing() {
        let out = run("fn Main(): Int { let a = 2; a = \"hello\"; return 0; }");
        assert!(out.contains("E205"), "{out}");
        assert!(!out.contains("=>"), "{out}");
    }

    #[test]
    fn run_executes_unannotated_binding_as_null() {
        let out = run("fn Main(): Int { let a; return 0; }");
        assert!(!out.contains("E206"), "{out}");
        assert!(out.contains("=> 0"), "{out}");
    }

    #[test]
    fn run_executes_recursive_class_tree() {
        let out = run("class TreeNode { let value: Int; let left: TreeNode?; let right: TreeNode?; fn count(): Int { let total = 1; if (this.left != null) { total = total + this.left.count(); } if (this.right != null) { total = total + this.right.count(); } return total; } } fn Main(): Int { let root = new TreeNode(); root.value = 10; let l = new TreeNode(); l.value = 5; root.left = l; assert(root.count() == 2, \"count\"); print(\"wasm-tree-ok\"); return 0; }");
        assert!(out.contains("wasm-tree-ok"), "{out}");
        assert!(out.contains("=> 0"), "{out}");
    }

    #[test]
    fn run_rejects_cyclic_default_construction() {
        let out = run("class Node { let next: Node = new Node(); } fn Main(): Int { return 0; }");
        assert!(out.contains("E108"), "{out}");
        assert!(out.contains("cyclic default construction"), "{out}");
        assert!(!out.contains("=>"), "{out}");
    }

    #[test]
    fn run_executes_enum_payload_match() {
        let out = run("enum Expr { Num(Int), Add(Expr, Expr) } fn eval(e: Expr): Int { switch (e) { case .Num(n): return n; case .Add(l, r): return eval(l) + eval(r); } } fn Main(): Int { let s = Expr.Add(Expr.Num(1), Expr.Num(2)); assert(eval(s) == 3, \"eval\"); print(\"wasm-enum-ok\"); return 0; }");
        assert!(out.contains("wasm-enum-ok"), "{out}");
        assert!(out.contains("=> 0"), "{out}");
    }

    #[test]
    fn run_executes_qualified_variant_pattern() {
        let out = run("enum E { A(Int), B } fn f(e: E): Int { switch (e) { case E.A(x): return x; default: return -1; } } fn Main(): Int { print(f(E.A(5))); return 0; }");
        assert!(out.contains("5"), "{out}");
        assert!(out.contains("=> 0"), "{out}");
    }

    #[test]
    fn run_executes_pointer_read_write() {
        let out = run("import { ByteBuffer } from \"@std/bytes\"; fn Main(): Int { let buf = ByteBuffer.allocate(16); unsafe { let ptr: Pointer<Int> = Pointer.fromAddress<Int>(buf.address()); ptr.write(42); assert(ptr.read() == 42, \"rw\"); *ptr = 7; assert(*ptr == 7, \"sugar\"); print(\"wasm-ptr-ok\"); } return 0; }");
        assert!(out.contains("wasm-ptr-ok"), "{out}");
        assert!(out.contains("=> 0"), "{out}");
    }

    #[test]
    fn run_rejects_pointer_read_outside_unsafe() {
        let out = run("fn Main(): Int { let p: Pointer<Int>; unsafe { p = Pointer.fromAddress<Int>(8); } let v = p.read(); return 0; }");
        assert!(out.contains("E202"), "{out}");
        assert!(!out.contains("=>"), "{out}");
    }
}
