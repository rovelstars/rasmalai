#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

use std::path::{Path, PathBuf};
use std::process::Command;

const PRELUDE: &str = r#"class Node {
    let label: String
    let kids: Array<Node> = []

    init(label: String) {
        this.label = label
    }

    fn addChild(c: Node) {
        this.kids.push(c)
    }

    fn count(): Int {
        let total = 0
        let i = 0
        while i < this.kids.length() {
            total += 1
            i += 1
        }
        return total
    }

    fn at(i: Int): Node {
        return this.kids[i]
    }

    fn text(): String {
        return this.label
    }
}

fn leaf(n: String): Node {
    return new Node(n)
}

fn column(): Node {
    return new Node("column")
}

fn Count(): Int {
    return 3
}
"#;

fn rnx() -> String {
    env!("CARGO_BIN_EXE_rnx").to_string()
}

fn vendor() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../stdlib/src")
}

fn project(tag: &str, src: &str) -> crate::scratch::Scratch {
    let dir = crate::scratch::dir(tag);
    std::fs::write(
        dir.join("Project.config"),
        "export default {\n    project: { name: \"ub\", version: \"0.1.0\" },\n    entries: { main: \"main.rnx\" },\n}\n",
    )
    .unwrap();
    std::fs::write(dir.join("main.rnx"), src).unwrap();
    dir
}

fn run(dir: &Path, backend: &str) -> String {
    let out = Command::new(rnx())
        .args(["run", "--backend", backend])
        .current_dir(dir)
        .env("NO_COLOR", "1")
        .env("RNX_VENDOR_DIR", vendor())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(out.status.success(), "{backend}: {stderr}{stdout}");
    assert!(
        !stderr.contains("panicked") && !stderr.contains("rnx panic"),
        "{backend}: {stderr}"
    );
    stdout
}

fn program(body: &str) -> String {
    format!("{PRELUDE}\nfn Main(): Int {{\n{body}\n    return 0\n}}\n")
}

fn check_all(tag: &str, body: &str, want: &str) {
    let dir = project(tag, &program(body));
    for backend in ["interpreter", "cranelift", "llvm"] {
        assert_eq!(run(&dir, backend), want, "{backend} {tag}");
    }
}

fn reject(src: &str, tag: &str) -> String {
    let dir = project(tag, src);
    let out = Command::new(rnx())
        .args(["check", "main.rnx"])
        .current_dir(&dir)
        .env("NO_COLOR", "1")
        .env("RNX_VENDOR_DIR", vendor())
        .output()
        .unwrap();
    assert!(!out.status.success(), "expected a rejection for {tag}");
    String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr)
}

#[test]
fn block_appends_children_in_source_order() {
    check_all(
        "order",
        r#"
    let c = column() {
        leaf("a")
        leaf("b")
    }
    println!(c.count())
    println!(c.at(0).text())
    println!(c.at(1).text())
"#,
        "2\na\nb\n",
    );
}

#[test]
fn nested_blocks_attach_to_their_own_result() {
    check_all(
        "nested",
        r#"
    let outer = column() {
        column() {
            leaf("x")
        }
        leaf("y")
    }
    println!(outer.count())
    println!(outer.at(0).count())
    println!(outer.at(1).text())
    println!(outer.at(0).at(0).text())
"#,
        "2\n1\ny\nx\n",
    );
}

#[test]
fn if_flattens_only_the_taken_branch() {
    check_all(
        "if",
        r#"
    let flag = true
    let c = column() {
        if flag {
            leaf("taken")
        } else {
            leaf("skipped")
        }
        if !flag {
            leaf("also-skipped")
        }
    }
    println!(c.count())
    println!(c.at(0).text())
"#,
        "1\ntaken\n",
    );
}

#[test]
fn for_flattens_every_body_statement() {
    check_all(
        "for",
        r#"
    let c = column() {
        for i in 0..3 {
            leaf("n${i}")
        }
    }
    println!(c.count())
    println!(c.at(0).text())
    println!(c.at(2).text())
"#,
        "3\nn0\nn2\n",
    );
}

#[test]
fn while_flattens_each_iteration() {
    check_all(
        "while",
        r#"
    let c = column() {
        let i = 0
        while i < 2 {
            leaf("w${i}")
            i += 1
        }
    }
    println!(c.count())
    println!(c.at(0).text())
    println!(c.at(1).text())
"#,
        "2\nw0\nw1\n",
    );
}

#[test]
fn switch_flattens_only_the_taken_case() {
    check_all(
        "switch",
        r#"
    let k = 2
    let c = column() {
        switch k {
            case 1: leaf("A"); pass;
            case 2: leaf("B"); pass;
            default: leaf("D"); pass;
        }
    }
    println!(c.count())
    println!(c.at(0).text())
"#,
        "1\nB\n",
    );
}

#[test]
fn let_inside_a_block_stays_inside() {
    check_all(
        "scope",
        r#"
    let c = column() {
        let t = "in"
        leaf(t)
        leaf(t + "!")
    }
    println!(c.count())
    println!(c.at(1).text())
"#,
        "2\nin!\n",
    );
}

#[test]
fn block_yields_the_callee_value() {
    check_all(
        "result",
        r#"
    let c = column() { leaf("a") }
    println!(c.count())
"#,
        "1\n",
    );
}

#[test]
fn a_receiver_without_add_child_is_e108() {
    let src = program(
        r#"
    let n = Count() { println!("x") }
    println!(n)
"#,
    );
    let text = reject(&src, "e108");
    assert!(text.contains("E108"), "{text}");
    assert!(text.contains("addChild"), "{text}");
    assert!(text.contains("Int"), "{text}");
}
