fn expected_tag(text: &str) -> Option<String> {
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

#[test]
fn llvm_version_matches_inkwell_pin() {
    let manifest =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let text = std::fs::read_to_string(&manifest).unwrap();
    let expected = expected_tag(&text).unwrap();
    assert_eq!(llvm::LLVM_VERSION, expected);
    assert!(manifest.is_file());
    let sys_line = text
        .lines()
        .find(|l| l.contains("llvm-sys"))
        .unwrap_or("");
    let tail = sys_line.split("llvm-sys").nth(1).unwrap_or("");
    let digits: String = tail
        .trim_start_matches(|c: char| !c.is_ascii_digit())
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    let raw: u32 = digits.parse().unwrap();
    assert_eq!(format!("llvm{}", raw / 10), expected);
}
