#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

const FLOAT_TO_INT: &str = "fn Main(): Int { let i = 2.5 as Int println!(\"i={i}\") return 0 }\n";
const INT_TO_FLOAT: &str = "fn Main(): Int { let i = 1 as Float println!(\"i={i}\") return 0 }\n";
const BOOL_TO_INT: &str = "fn Main(): Int { let i = true as Int println!(\"i={i}\") return 0 }\n";
const BYTE_PRINT: &str = "fn Main(): Int {\n    let b = 65 as Byte\n    println!(b)\n    let s = \"b={b}\"\n    println!(s)\n    if s == \"b=65\" { return 42 }\n    return 0\n}\n";

fn write_src(tag: &str, src: &str) -> crate::scratch::Scratch {
    let dir = crate::scratch::dir(tag);
    std::fs::write(dir.join("main.rnx"), src).unwrap();
    dir
}

fn rnx(dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let mut cmd = std::process::Command::new(rnx);
    cmd.current_dir(dir);
    cmd.env("NO_COLOR", "1");
    for a in args {
        cmd.arg(a);
    }
    cmd.output().unwrap()
}

fn combined(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr)
}

fn assert_rejected_everywhere(tag: &str, src: &str, message: &str) {
    let dir = write_src(tag, src);
    let main = dir.join("main.rnx");
    let main = main.to_str().unwrap();
    for backend in ["interpreter", "cranelift", "llvm"] {
        let out = rnx(&dir, &["run", main, "--backend", backend]);
        assert!(!out.status.success(), "{backend} must reject the cast");
        let text = combined(&out);
        assert!(text.contains("error[E108]"), "{backend}: {text}");
        assert!(text.contains(message), "{backend} diagnostic mismatch: {text}");
    }
    let checked = rnx(&dir, &["check", main]);
    assert!(!checked.status.success(), "check must reject the cast");
    assert!(combined(&checked).contains(message), "{}", combined(&checked));
}

#[test]
fn float_to_int_as_rejected_on_every_backend() {
    assert_rejected_everywhere(
        "castf2i",
        FLOAT_TO_INT,
        "`as` does not convert `Float` to `Int`",
    );
}

#[test]
fn int_to_float_as_rejected_on_every_backend() {
    assert_rejected_everywhere(
        "casti2f",
        INT_TO_FLOAT,
        "`as` does not convert `Int` to `Float`",
    );
}

#[test]
fn bool_to_int_as_rejected_on_every_backend() {
    assert_rejected_everywhere(
        "castb2i",
        BOOL_TO_INT,
        "`as` does not convert `Bool` to `Int`",
    );
}

#[test]
fn byte_print_matches_on_every_backend() {
    let dir = write_src("castbyte", BYTE_PRINT);
    let main = dir.join("main.rnx");
    let main = main.to_str().unwrap();
    for backend in ["interpreter", "cranelift", "llvm"] {
        let out = rnx(&dir, &["run", main, "--backend", backend]);
        assert_eq!(
            out.status.code(),
            Some(42),
            "{backend}: {}",
            combined(&out)
        );
        assert_eq!(
            String::from_utf8(out.stdout).unwrap(),
            "65\nb=65\n",
            "{backend} stdout"
        );
    }
}
