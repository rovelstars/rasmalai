use diagnostics::{Code, Diagnostic};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NativePrefer {
    #[default]
    None,
    System,
    Bundled,
}

impl NativePrefer {
    pub fn from_env() -> NativePrefer {
        match std::env::var("RNX_PREFER_NATIVE").as_deref() {
            Ok("system") => NativePrefer::System,
            Ok("bundled") => NativePrefer::Bundled,
            _ => NativePrefer::None,
        }
    }

    pub fn resolve(system_flag: bool, bundled_flag: bool) -> NativePrefer {
        if system_flag {
            return NativePrefer::System;
        }
        if bundled_flag {
            return NativePrefer::Bundled;
        }
        NativePrefer::from_env()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveNative {
    pub system: bool,
    pub version: Option<String>,
    pub from: Option<String>,
}

pub fn effective_choice(
    declared_system: bool,
    declared_version: Option<&str>,
    declared_from: Option<&str>,
    pin: Option<&crate::project::NativePin>,
    prefer: NativePrefer,
) -> EffectiveNative {
    let pin_system = pin.map(|p| p.system);
    let pin_version = pin.and_then(|p| p.version.as_deref());
    let pin_from = pin.and_then(|p| p.from.as_deref());
    let version = pin_version.or(declared_version);
    let from = pin_from.or(declared_from);
    match (pin_system, prefer) {
        (Some(system), _) => EffectiveNative { system, version: version.map(str::to_string), from: if system { None } else { from.map(str::to_string) } },
        (None, NativePrefer::System) => EffectiveNative { system: true, version: version.map(str::to_string), from: None },
        (None, NativePrefer::Bundled) => EffectiveNative { system: false, version: version.map(str::to_string), from: from.map(str::to_string) },
        (None, NativePrefer::None) => EffectiveNative {
            system: declared_system,
            version: version.map(str::to_string),
            from: from.map(str::to_string),
        },
    }
}

fn valid_probe_name(lib: &str) -> bool {
    !lib.is_empty()
        && !lib.starts_with('-')
        && lib.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_' || b == b'+' || b == b'-')
}

fn pkg_config_version(lib: &str) -> Option<String> {
    let out = std::process::Command::new("pkg-config")
        .arg("--modversion")
        .arg(lib)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let version = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if version.is_empty() { None } else { Some(version) }
}

fn ldconfig_soname_version(lib: &str) -> Option<Option<String>> {
    let out = std::process::Command::new("ldconfig").arg("-p").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let want = format!("lib{lib}.so");
    let mut found = false;
    let mut version: Option<String> = None;
    for line in text.lines() {
        let entry = line.trim();
        let Some(name) = entry.split_whitespace().next() else {
            continue;
        };
        if name == want || name.starts_with(&format!("{want}.")) {
            found = true;
            if let Some(suffix) = name.strip_prefix(&format!("{want}.")) {
                let candidate = suffix.split_whitespace().next().unwrap_or_default();
                if !candidate.is_empty()
                    && candidate.bytes().all(|b| b.is_ascii_digit() || b == b'.')
                    && version.is_none()
                {
                    version = Some(candidate.to_string());
                }
            }
        }
    }
    if found { Some(version) } else { None }
}

pub fn verify_system_lib(lib: &str, constraint: Option<&str>) -> Result<String, Diagnostic> {
    if !valid_probe_name(lib) {
        return Err(Diagnostic::new(
            Code::E108,
            format!("system library `{lib}` is not a valid library name (expected `[A-Za-z0-9._+-]+`, never starting with `-`)"),
        ));
    }
    if let Some(installed) = pkg_config_version(lib) {
        if let Some(range) = constraint {
            if !crate::fetch::engine_satisfied(range, &installed) {
                return Err(Diagnostic::new(
                    Code::E108,
                    format!("system library `{lib}` reports version `{installed}` which does not satisfy `{range}` (via `pkg-config --modversion {lib}`)"),
                ));
            }
        }
        return Ok(installed);
    }
    match ldconfig_soname_version(lib) {
        Some(maybe_version) => {
            if let Some(range) = constraint {
                match maybe_version {
                    Some(installed) if crate::fetch::engine_satisfied(range, &installed) => Ok(installed),
                    Some(installed) => Err(Diagnostic::new(
                        Code::E108,
                        format!("system library `{lib}` reports version `{installed}` which does not satisfy `{range}` (via `ldconfig -p`)"),
                    )),
                    None => Err(Diagnostic::new(
                        Code::E108,
                        format!("system library `{lib}` is present but its version cannot be verified without `pkg-config`, so constraint `{range}` fails closed"),
                    )),
                }
            } else {
                Ok(maybe_version.unwrap_or_default())
            }
        }
        None => Err(Diagnostic::new(
            Code::E108,
            format!("system library `{lib}` not found (`pkg-config --modversion {lib}` failed and `ldconfig -p` lists no `lib{lib}.so`)"),
        )),
    }
}

pub fn check_owner_version(dep: &str, owner_version: &str, constraint: &str) -> Result<(), Diagnostic> {
    if crate::fetch::engine_satisfied(constraint, owner_version) {
        return Ok(());
    }
    Err(Diagnostic::new(
        Code::E108,
        format!("native dependency `{dep}` pins bundled version `{constraint}` but the declaring package is at `{owner_version}`"),
    ))
}

pub fn triple_target(triple: &str) -> Option<crate::project::eval::ConfigTarget> {
    let parts: Vec<&str> = triple.split('-').collect();
    if parts.len() < 3 || parts.len() > 4 {
        return None;
    }
    if parts.iter().any(|p| p.is_empty()) {
        return None;
    }
    Some(crate::project::eval::ConfigTarget {
        os: parts[2].to_string(),
        arch: parts[0].to_string(),
        env: parts.get(3).unwrap_or(&"").to_string(),
    })
}

pub fn host_triple() -> String {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let env = std::env::var("RNX_TARGET_ENV").unwrap_or_else(|_| {
        if os == "windows" {
            "msvc".to_string()
        } else if os == "linux" {
            "gnu".to_string()
        } else {
            String::new()
        }
    });
    let vendor = "unknown";
    if env.is_empty() {
        format!("{arch}-{vendor}-{os}")
    } else {
        format!("{arch}-{vendor}-{os}-{env}")
    }
}

#[cfg(test)]
pub(crate) fn test_path_guard() -> std::sync::MutexGuard<'static, ()> {
    static GUARD: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    GUARD.get_or_init(|| std::sync::Mutex::new(())).lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct PathOverride {
        prev: Option<String>,
    }

    impl PathOverride {
        fn prepend(dir: &std::path::Path) -> PathOverride {
            let prev = std::env::var("PATH").ok();
            let next = match &prev {
                Some(p) if !p.is_empty() => format!("{}:{}", dir.display(), p),
                _ => dir.display().to_string(),
            };
            unsafe {
                std::env::set_var("PATH", next);
            }
            PathOverride { prev }
        }
    }

    impl Drop for PathOverride {
        fn drop(&mut self) {
            unsafe {
                match &self.prev {
                    Some(p) => std::env::set_var("PATH", p),
                    None => std::env::remove_var("PATH"),
                }
            }
        }
    }

    fn stub_bin(tag: &str, name: &str, body: &str) -> crate::scratch::Scratch {
        let dir = crate::scratch::dir(&format!("native-probe-{tag}"));
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&path).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&path, perms).unwrap();
        }
        dir
    }

    #[test]
    fn prefer_flags_win_over_env() {
        assert_eq!(NativePrefer::resolve(true, false), NativePrefer::System);
        assert_eq!(NativePrefer::resolve(false, true), NativePrefer::Bundled);
        assert_eq!(NativePrefer::resolve(false, false), NativePrefer::from_env());
    }

    #[test]
    fn effective_choice_prefers_pin_then_flag_then_declared() {
        let declared = (true, Some(">=1.0"), None);
        let pin = crate::project::NativePin { system: false, version: None, from: Some("libz.so".to_string()) };
        let got = effective_choice(declared.0, declared.1, declared.2, Some(&pin), NativePrefer::System);
        assert_eq!(got, EffectiveNative { system: false, version: Some(">=1.0".to_string()), from: Some("libz.so".to_string()) });
        let got = effective_choice(false, None, Some("libz.so"), None, NativePrefer::System);
        assert_eq!(got, EffectiveNative { system: true, version: None, from: None });
        let got = effective_choice(true, Some(">=2.0.0"), None, None, NativePrefer::Bundled);
        assert_eq!(got, EffectiveNative { system: false, version: Some(">=2.0.0".to_string()), from: None });
        let got = effective_choice(true, None, None, None, NativePrefer::None);
        assert_eq!(got, EffectiveNative { system: true, version: None, from: None });
    }

    #[test]
    fn probe_rejects_bad_names_without_probing() {
        let _guard = test_path_guard();
        for bad in ["-lfoo", "a/b", "has space", "semi;colon", ""] {
            let err = verify_system_lib(bad, None).unwrap_err();
            assert_eq!(err.code, diagnostics::Code::E108);
            assert!(err.message.contains("not a valid library name"), "{}", err.message);
        }
    }

    #[test]
    fn probe_reads_pkg_config_and_checks_constraints() {
        let _guard = test_path_guard();
        let dir = stub_bin(
            "ok",
            "pkg-config",
            "#!/bin/sh\nif [ \"$1\" = \"--modversion\" ]; then echo \"1.2.13\"; exit 0; fi\nexit 1\n",
        );
        let _path = PathOverride::prepend(&dir);
        assert_eq!(verify_system_lib("z", Some(">=1.2.0")).unwrap(), "1.2.13");
        assert_eq!(verify_system_lib("z", None).unwrap(), "1.2.13");
        let err = verify_system_lib("z", Some(">=2.0.0")).unwrap_err();
        assert_eq!(err.code, diagnostics::Code::E108);
        assert!(err.message.contains("1.2.13"), "{}", err.message);
        assert!(err.message.contains(">=2.0.0"), "{}", err.message);
    }

    #[test]
    fn probe_fails_closed_when_library_is_absent() {
        let _guard = test_path_guard();
        let dir = stub_bin("missing", "pkg-config", "#!/bin/sh\nexit 1\n");
        std::fs::write(dir.join("ldconfig"), "#!/bin/sh\nexit 1\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let path = dir.join("ldconfig");
            let mut perms = std::fs::metadata(&path).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&path, perms).unwrap();
        }
        let _path = PathOverride::prepend(&dir);
        let err = verify_system_lib("nosuchlib_xyz", None).unwrap_err();
        assert_eq!(err.code, diagnostics::Code::E108);
        assert!(err.message.contains("not found"), "{}", err.message);
    }

    #[test]
    fn triple_target_splits_arch_vendor_os_env() {
        let t = triple_target("x86_64-unknown-linux-gnu").unwrap();
        assert_eq!((t.arch.as_str(), t.os.as_str(), t.env.as_str()), ("x86_64", "linux", "gnu"));
        let t = triple_target("aarch64-apple-darwin").unwrap();
        assert_eq!((t.arch.as_str(), t.os.as_str(), t.env.as_str()), ("aarch64", "darwin", ""));
        assert!(triple_target("nope").is_none());
        assert!(triple_target("a-b-c-d-e").is_none());
    }

    #[test]
    fn owner_version_check_uses_range_grammar() {
        check_owner_version("z", "1.2.0", "^1.0.0").unwrap();
        let err = check_owner_version("z", "2.0.0", "^1.0.0").unwrap_err();
        assert_eq!(err.code, diagnostics::Code::E108);
        assert!(err.message.contains("2.0.0"), "{}", err.message);
    }
}
