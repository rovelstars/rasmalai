use diagnostics::{Code, Diagnostic};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LinkPreference {
    #[default]
    Auto,
    Static,
    Dynamic,
}

impl LinkPreference {
    pub fn parse(raw: &str) -> Result<LinkPreference, String> {
        match raw {
            "auto" => Ok(LinkPreference::Auto),
            "static" => Ok(LinkPreference::Static),
            "dynamic" => Ok(LinkPreference::Dynamic),
            _ => Err(format!(
                "invalid `link` value `{raw}` (expected `static`, `dynamic`, or `auto`)"
            )),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            LinkPreference::Auto => "auto",
            LinkPreference::Static => "static",
            LinkPreference::Dynamic => "dynamic",
        }
    }

    pub fn effective(&self, has_shared_object: bool) -> &'static str {
        match self {
            LinkPreference::Static => "static",
            LinkPreference::Dynamic => "dynamic",
            LinkPreference::Auto => {
                if has_shared_object {
                    "dynamic"
                } else {
                    "static"
                }
            }
        }
    }
}

pub const KNOWN_SPDX: &[&str] = &[
    "0BSD",
    "Apache-2.0",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "CC0-1.0",
    "GPL-2.0-only",
    "GPL-2.0-or-later",
    "GPL-3.0-only",
    "GPL-3.0-or-later",
    "AGPL-3.0-only",
    "ISC",
    "LGPL-2.1-only",
    "LGPL-2.1-or-later",
    "LGPL-3.0-only",
    "LGPL-3.0-or-later",
    "MIT",
    "MPL-2.0",
    "Unlicense",
];

pub fn is_known_spdx(value: &str) -> bool {
    KNOWN_SPDX.contains(&value)
}

pub fn obligation_for(spdx: &str) -> &'static str {
    match spdx {
        "Apache-2.0" => "notice",
        "MPL-2.0" => "file-copyleft",
        "LGPL-2.1-only" | "LGPL-2.1-or-later" => "relink-object-files",
        "LGPL-3.0-only" | "LGPL-3.0-or-later" => "relink-corresponding-source",
        "GPL-2.0-only" | "GPL-2.0-or-later" | "GPL-3.0-only" | "GPL-3.0-or-later" | "AGPL-3.0-only" => {
            "whole-work-copyleft"
        }
        _ => "none",
    }
}

pub fn is_whole_work_copyleft(spdx: &str) -> bool {
    obligation_for(spdx) == "whole-work-copyleft"
}

fn marker_for(spdx: &str) -> &'static str {
    match spdx {
        "MIT" => "mit license",
        "Apache-2.0" => "apache license",
        "BSD-2-Clause" | "BSD-3-Clause" => "bsd",
        "ISC" => "isc license",
        "MPL-2.0" => "mozilla public",
        "LGPL-2.1-only" | "LGPL-2.1-or-later" => "lesser general public",
        "GPL-2.0-only"
        | "GPL-2.0-or-later"
        | "GPL-3.0-only"
        | "GPL-3.0-or-later" => "general public",
        "AGPL-3.0-only" => "affero",
        "Unlicense" => "unlicense",
        "CC0-1.0" => "cc0",
        "0BSD" => "0bsd",
        _ => "",
    }
}

pub fn infer_spdx_from_text(text: &str) -> Option<String> {
    let lower = text.to_lowercase();
    if lower.contains("mit license") {
        return Some("MIT".to_string());
    }
    if lower.contains("apache license") {
        return Some("Apache-2.0".to_string());
    }
    if lower.contains("mozilla public license") {
        return Some("MPL-2.0".to_string());
    }
    if lower.contains("lesser general public") {
        if lower.contains("version 3") {
            return Some("LGPL-3.0-only".to_string());
        }
        return Some("LGPL-2.1-only".to_string());
    }
    if lower.contains("affero general public") {
        return Some("AGPL-3.0-only".to_string());
    }
    if lower.contains("general public license") {
        if lower.contains("version 3") {
            return Some("GPL-3.0-only".to_string());
        }
        return Some("GPL-2.0-only".to_string());
    }
    if lower.contains("isc license") {
        return Some("ISC".to_string());
    }
    if lower.contains("unlicense") {
        return Some("Unlicense".to_string());
    }
    if lower.contains("cc0") {
        return Some("CC0-1.0".to_string());
    }
    None
}

pub fn license_text_matches(text: &str, spdx: &str) -> bool {
    let marker = marker_for(spdx);
    if marker.is_empty() {
        return false;
    }
    text.to_lowercase().contains(marker)
}

pub fn package_license_text(dir: &std::path::Path) -> Option<String> {
    for name in ["LICENSE", "LICENSE.txt"] {
        if let Ok(bytes) = std::fs::read(dir.join(name)) {
            if let Ok(text) = String::from_utf8(bytes) {
                if !text.trim().is_empty() {
                    return Some(text);
                }
            }
        }
    }
    None
}

pub fn resolve_package_license(dir: &std::path::Path, declared: Option<&str>) -> String {
    if let Some(spdx) = declared {
        return spdx.to_string();
    }
    if let Some(text) = package_license_text(dir) {
        if let Some(inferred) = infer_spdx_from_text(&text) {
            return inferred;
        }
    }
    "unknown".to_string()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LicenseEntry {
    pub name: String,
    pub version: String,
    pub license: String,
    pub link: String,
    pub obligation: String,
}

pub fn license_conflicts(entries: &[LicenseEntry]) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for e in entries {
        seen.insert(e.license.clone());
    }
    if seen.contains("Apache-2.0")
        && (seen.contains("GPL-2.0-only") || seen.contains("GPL-2.0-or-later"))
    {
        out.push(
            "Apache-2.0 is incompatible with GPL-2.0 in the same closure (choose GPL-3.0-or-later or remove one side)"
                .to_string(),
        );
    }
    let static_copyleft: Vec<&LicenseEntry> = entries
        .iter()
        .filter(|e| e.obligation == "whole-work-copyleft" && e.link == "static")
        .collect();
    if !static_copyleft.is_empty() {
        let names: Vec<String> = static_copyleft
            .iter()
            .map(|e| format!("{}@{} ({})", e.name, e.version, e.license))
            .collect();
        out.push(format!(
            "static linkage pulls whole-work copyleft into the binary: {} (ship source or switch to dynamic where the license permits it)",
            names.join(", ")
        ));
    }
    let relink: Vec<&LicenseEntry> = entries
        .iter()
        .filter(|e| {
            (e.obligation == "relink-object-files"
                || e.obligation == "relink-corresponding-source")
                && e.link == "static"
        })
        .collect();
    if !relink.is_empty() {
        let names: Vec<String> = relink
            .iter()
            .map(|e| format!("{}@{} ({}, {})", e.name, e.version, e.license, e.obligation))
            .collect();
        out.push(format!(
            "static linkage carries a relink obligation: {} (ship relinkable objects or corresponding source, or refuse the bundle)",
            names.join(", ")
        ));
    }
    out.sort();
    out.dedup();
    out
}

pub fn validate_spdx_field(spdx: &str) -> Result<(), Diagnostic> {
    if is_known_spdx(spdx) {
        return Ok(());
    }
    Err(Diagnostic::new(
        Code::E108,
        format!(
            "unknown license `{spdx}` (expected one of {})",
            KNOWN_SPDX.join(", ")
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_values_parse_and_default_auto() {
        assert_eq!(LinkPreference::parse("static").unwrap(), LinkPreference::Static);
        assert_eq!(LinkPreference::parse("dynamic").unwrap(), LinkPreference::Dynamic);
        assert_eq!(LinkPreference::parse("auto").unwrap(), LinkPreference::Auto);
        assert!(LinkPreference::parse("shared").is_err());
        assert_eq!(LinkPreference::default(), LinkPreference::Auto);
    }

    #[test]
    fn auto_resolves_by_shared_object_presence() {
        assert_eq!(LinkPreference::Auto.effective(true), "dynamic");
        assert_eq!(LinkPreference::Auto.effective(false), "static");
        assert_eq!(LinkPreference::Static.effective(true), "static");
        assert_eq!(LinkPreference::Dynamic.effective(false), "dynamic");
    }

    #[test]
    fn spdx_validation_covers_known_set() {
        for spdx in KNOWN_SPDX {
            validate_spdx_field(spdx).unwrap();
            assert!(!obligation_for(spdx).is_empty());
        }
        assert!(validate_spdx_field("GPL-2.0").is_err());
        assert!(validate_spdx_field("").is_err());
    }

    #[test]
    fn obligations_follow_license_families() {
        assert_eq!(obligation_for("MIT"), "none");
        assert_eq!(obligation_for("Apache-2.0"), "notice");
        assert_eq!(obligation_for("MPL-2.0"), "file-copyleft");
        assert_eq!(obligation_for("LGPL-2.1-only"), "relink-object-files");
        assert_eq!(obligation_for("LGPL-3.0-only"), "relink-corresponding-source");
        assert_eq!(obligation_for("GPL-3.0-only"), "whole-work-copyleft");
    }

    #[test]
    fn conflicts_flag_apache_gpl2_and_static_copyleft() {
        let entries = vec![
            LicenseEntry {
                name: "a".to_string(),
                version: "1.0.0".to_string(),
                license: "Apache-2.0".to_string(),
                link: "static".to_string(),
                obligation: obligation_for("Apache-2.0").to_string(),
            },
            LicenseEntry {
                name: "b".to_string(),
                version: "2.0.0".to_string(),
                license: "GPL-2.0-only".to_string(),
                link: "static".to_string(),
                obligation: obligation_for("GPL-2.0-only").to_string(),
            },
        ];
        let conflicts = license_conflicts(&entries);
        assert_eq!(conflicts.len(), 2);
        assert!(conflicts.iter().any(|c| c.contains("incompatible")));
        assert!(conflicts.iter().any(|c| c.contains("whole-work copyleft")));
    }

    #[test]
    fn relink_obligation_surfaces_for_static_lgpl() {
        let entries = vec![LicenseEntry {
            name: "sqlite3".to_string(),
            version: "3.45.0".to_string(),
            license: "LGPL-3.0-only".to_string(),
            link: "static".to_string(),
            obligation: obligation_for("LGPL-3.0-only").to_string(),
        }];
        let conflicts = license_conflicts(&entries);
        assert_eq!(conflicts.len(), 1);
        assert!(conflicts[0].contains("relink obligation"));
        let dynamic = vec![LicenseEntry {
            link: "dynamic".to_string(),
            ..entries[0].clone()
        }];
        assert!(license_conflicts(&dynamic).is_empty());
    }

    #[test]
    fn inference_reads_common_headers() {
        assert_eq!(
            infer_spdx_from_text("MIT License\n\nPermission is hereby granted"),
            Some("MIT".to_string())
        );
        assert_eq!(
            infer_spdx_from_text("Licensed under the Apache License, Version 2.0"),
            Some("Apache-2.0".to_string())
        );
        assert_eq!(infer_spdx_from_text("nothing recognizable"), None);
    }
}
