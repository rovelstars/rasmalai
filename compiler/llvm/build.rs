fn inkwell_feature_tag(text: &str) -> Option<String> {
    for line in text.lines() {
        if !line.contains("inkwell") {
            continue;
        }
        let mut rest = line;
        while let Some(at) = rest.find("llvm") {
            rest = &rest[at + 4..];
            let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            if !digits.is_empty() && rest[digits.len()..].starts_with('-') {
                return Some(format!("llvm{digits}"));
            }
        }
    }
    None
}

fn llvm_sys_tag(text: &str) -> Option<String> {
    for line in text.lines() {
        if !line.contains("llvm-sys") {
            continue;
        }
        let tail = line.split("llvm-sys").nth(1).unwrap_or("");
        let tail = tail.trim_start_matches(|c: char| !c.is_ascii_digit());
        let digits: String = tail
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if let Ok(raw) = digits.parse::<u32>() {
            if raw >= 100 {
                return Some(format!("llvm{}", raw / 10));
            }
        }
    }
    None
}

fn main() {
    let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    let path = std::path::Path::new(&dir).join("Cargo.toml");
    println!("cargo:rerun-if-changed={}", path.display());
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    if let Some(tag) = inkwell_feature_tag(&text).or_else(|| llvm_sys_tag(&text)) {
        println!("cargo:rustc-env=RNX_LLVM_VERSION={tag}");
        return;
    }
    panic!("llvm/build.rs: cannot derive LLVM version from {}", path.display());
}
