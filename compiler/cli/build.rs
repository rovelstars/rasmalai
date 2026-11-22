use std::process::Command;

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn format_unix_utc(secs: u64) -> String {
    let (y, m, d) = civil_from_days((secs / 86_400) as i64);
    let rem = secs % 86_400;
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=RNX_INSTALLATION_MANAGED");
    println!("cargo:rerun-if-env-changed=RNX_INSTALLATION_NO_TIMESTAMP");
    println!("cargo:rerun-if-env-changed=RNX_CHECKOUT_STDLIB");
    println!("cargo:rerun-if-env-changed=RNX_RELEASE_BUILD");
    println!("cargo:rerun-if-env-changed=CI");
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
    // The baked commit and timestamp must follow new commits, not just changes
    // to this script. Watch HEAD and the branch refs it points at.
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/refs/heads");
    let commit = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=RNX_COMMIT={commit}");
    let stamped = std::env::var("RNX_INSTALLATION_NO_TIMESTAMP")
        .unwrap_or_default()
        .is_empty();
    let secs = if stamped {
        std::env::var("SOURCE_DATE_EPOCH")
            .ok()
            .and_then(|s| s.trim().parse::<u64>().ok())
            .or_else(|| {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .ok()
                    .map(|d| d.as_secs())
            })
    } else {
        None
    };
    let built = secs.map(format_unix_utc).unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=RNX_BUILD_TIMESTAMP={built}");
    let reason = std::env::var("RNX_INSTALLATION_MANAGED").unwrap_or_default();
    println!("cargo:rustc-env=RNX_INSTALLATION_MANAGED_REASON={reason}");
    let checkout = match std::env::var("RNX_CHECKOUT_STDLIB") {
        Ok(value) => value,
        Err(_)
            if std::env::var_os("CI").is_some()
                || std::env::var_os("RNX_RELEASE_BUILD").is_some() =>
        {
            "0".to_string()
        }
        Err(_) => "1".to_string(),
    };
    println!("cargo:rustc-env=RNX_CHECKOUT_STDLIB={checkout}");
    let target = std::env::var("TARGET").unwrap_or_else(|_| "unknown".to_string());
    println!("cargo:rustc-env=RNX_TARGET_TRIPLE={target}");
}
