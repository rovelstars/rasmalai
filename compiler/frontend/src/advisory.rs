use crate::deplock::ProjectDepLock;
use crate::fetch::{engine_satisfied, is_valid_range, parse_json, registry_get, JsonValue};
use diagnostics::{Code, Diagnostic};

pub const FEED_PATH: &str = "/api/v0/advisories";
pub const SUPPORTED_SPEC: u32 = 0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Advisory {
    pub id: String,
    pub package: String,
    pub affected: String,
    pub artifacts: Vec<String>,
    pub severity: String,
    pub title: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedFeed {
    pub advisories: Vec<Advisory>,
    pub key_id: String,
    pub public_key: String,
    pub signature: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub advisory_id: String,
    pub package: String,
    pub version: String,
    pub severity: String,
    pub title: String,
    pub detail: String,
    pub pinned_artifact: bool,
}

fn canonical_json_string(out: &mut String, value: &str) {
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{9}' => out.push_str("\\t"),
            '\u{a}' => out.push_str("\\n"),
            '\u{c}' => out.push_str("\\f"),
            '\u{d}' => out.push_str("\\r"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u00{:02x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn canonical_advisory(out: &mut String, a: &Advisory) {
    let mut pins = a.artifacts.clone();
    pins.sort();
    out.push_str("{\"affected\":");
    canonical_json_string(out, &a.affected);
    out.push_str(",\"artifacts\":[");
    for (i, pin) in pins.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        canonical_json_string(out, pin);
    }
    out.push_str("],\"detail\":");
    canonical_json_string(out, &a.detail);
    out.push_str(",\"id\":");
    canonical_json_string(out, &a.id);
    out.push_str(",\"package\":");
    canonical_json_string(out, &a.package);
    out.push_str(",\"severity\":");
    canonical_json_string(out, &a.severity);
    out.push_str(",\"title\":");
    canonical_json_string(out, &a.title);
    out.push('}');
}

pub fn feed_canonical_bytes(advisories: &[Advisory]) -> Vec<u8> {
    let mut sorted: Vec<&Advisory> = advisories.iter().collect();
    sorted.sort_by(|a, b| a.id.cmp(&b.id));
    let mut out = String::from("{\"advisories\":[");
    for (i, a) in sorted.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        canonical_advisory(&mut out, a);
    }
    out.push_str("]}");
    out.into_bytes()
}

pub fn decode_hex(text: &str) -> Option<Vec<u8>> {
    if text.len() % 2 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(text.len() / 2);
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let hi = (bytes[i] as char).to_digit(16)?;
        let lo = (bytes[i + 1] as char).to_digit(16)?;
        out.push(((hi << 4) | lo) as u8);
        i += 2;
    }
    Some(out)
}

pub fn key_id_for_public_key(raw: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(raw);
    let digest = hasher.finalize();
    digest.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

fn json_string(value: &JsonValue) -> Option<String> {
    value.as_str().map(|s| s.to_string())
}

fn parse_advisory(value: &JsonValue) -> Option<Advisory> {
    let obj = value;
    let artifacts = match obj.get("artifacts") {
        None => Vec::new(),
        Some(list) => {
            let items = list.as_array()?;
            let mut pins = Vec::new();
            for item in items {
                pins.push(json_string(item)?);
            }
            pins
        }
    };
    let severity = json_string(obj.get("severity")?)?;
    if severity != "critical" && severity != "warning" && severity != "info" {
        return None;
    }
    Some(Advisory {
        id: json_string(obj.get("id")?)?,
        package: json_string(obj.get("package")?)?,
        affected: obj.get("affected").and_then(json_string).unwrap_or_else(|| "*".to_string()),
        artifacts,
        severity,
        title: obj.get("title").and_then(json_string).unwrap_or_default(),
        detail: obj.get("detail").and_then(json_string).unwrap_or_default(),
    })
}

pub fn parse_feed(body: &[u8]) -> Result<SignedFeed, Diagnostic> {
    let bad = |msg: &str| Diagnostic::new(Code::E108, format!("advisory feed: {msg}"));
    let text = std::str::from_utf8(body).map_err(|_| bad("feed is not valid UTF-8"))?;
    let root = parse_json(text.as_bytes()).map_err(|_| bad("feed is not valid JSON"))?;
    let advisories = match root.get("advisories") {
        Some(list) => {
            let items = list.as_array().ok_or_else(|| bad("`advisories` must be an array"))?;
            let mut out = Vec::new();
            for item in items {
                out.push(parse_advisory(item).ok_or_else(|| bad("advisory entry is malformed"))?);
            }
            out
        }
        None => return Err(bad("feed is missing `advisories`")),
    };
    Ok(SignedFeed {
        advisories,
        key_id: root.get("keyId").and_then(json_string).ok_or_else(|| bad("feed is missing `keyId`"))?,
        public_key: root
            .get("publicKey")
            .and_then(json_string)
            .ok_or_else(|| bad("feed is missing `publicKey`"))?,
        signature: root
            .get("signature")
            .and_then(json_string)
            .ok_or_else(|| bad("feed is missing `signature`"))?,
    })
}

pub fn verify_feed(feed: &SignedFeed) -> Result<(), Diagnostic> {
    let bad = |msg: &str| Diagnostic::new(Code::E108, format!("advisory feed: {msg}"));
    let public_key = decode_hex(&feed.public_key).ok_or_else(|| bad("public key is not hex"))?;
    if public_key.len() != 32 {
        return Err(bad("public key is not 32 bytes"));
    }
    if key_id_for_public_key(&public_key) != feed.key_id {
        return Err(bad("key id does not match the feed public key"));
    }
    let signature = decode_hex(&feed.signature).ok_or_else(|| bad("signature is not hex"))?;
    if signature.len() != 64 {
        return Err(bad("signature is not 64 bytes"));
    }
    let message = feed_canonical_bytes(&feed.advisories);
    verify_signature(&public_key, &message, &signature)?;
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn verify_signature(public_key: &[u8], message: &[u8], signature: &[u8]) -> Result<(), Diagnostic> {
    let peer = ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, public_key);
    peer.verify(message, signature)
        .map_err(|_| Diagnostic::new(Code::E108, "advisory feed: signature verification failed"))
}

#[cfg(target_arch = "wasm32")]
fn verify_signature(_public_key: &[u8], _message: &[u8], _signature: &[u8]) -> Result<(), Diagnostic> {
    Err(Diagnostic::new(Code::E108, "advisory feed: verification is not available on this target"))
}

pub enum FeedError {
    Unavailable(String),
    Rejected(String),
}

impl FeedError {
    pub fn message(&self) -> &str {
        match self {
            FeedError::Unavailable(message) => message,
            FeedError::Rejected(message) => message,
        }
    }
}

pub fn fetch_signed_feed(registry_base: &str) -> Result<SignedFeed, FeedError> {
    let url = format!("{}{FEED_PATH}", registry_base.trim_end_matches('/'));
    let reply = registry_get(&url, "advisory feed")
        .map_err(|e| FeedError::Unavailable(format!("request failed: {}", e.message)))?;
    if reply.status != 200 {
        return Err(FeedError::Unavailable(format!("registry returned HTTP {}", reply.status)));
    }
    match reply.spec_header.as_deref() {
        Some(spec) if spec.trim() == SUPPORTED_SPEC.to_string() => {}
        Some(spec) => {
            return Err(FeedError::Rejected(format!(
                "registry speaks spec {spec}, this CLI supports {SUPPORTED_SPEC}"
            )));
        }
        None => {
            return Err(FeedError::Rejected(
                "feed is missing the rnx-registry-spec header".to_string(),
            ));
        }
    }
    let feed = parse_feed(&reply.body).map_err(|e| FeedError::Rejected(e.message))?;
    verify_feed(&feed).map_err(|e| FeedError::Rejected(e.message))?;
    if let Some(pinned) = std::env::var("RNX_ADVISORY_KEY").ok().filter(|s| !s.trim().is_empty()) {
        let want = pinned.trim().to_lowercase();
        if feed.public_key.to_lowercase() != want && feed.key_id != want {
            return Err(FeedError::Rejected(
                "feed key does not match RNX_ADVISORY_KEY".to_string(),
            ));
        }
    }
    Ok(feed)
}

pub fn is_registry_source(source: &str) -> bool {
    source.starts_with("registry:")
}

pub fn advisory_matches(advisory: &Advisory, version: &str, artifact_digest: Option<&str>) -> bool {
    if !is_valid_range(&advisory.affected) {
        return false;
    }
    if !engine_satisfied(&advisory.affected, version) {
        return false;
    }
    if advisory.artifacts.is_empty() {
        return true;
    }
    match artifact_digest {
        Some(pin) => advisory.artifacts.iter().any(|p| p == pin),
        None => false,
    }
}

pub fn match_lock(lock: &ProjectDepLock, advisories: &[Advisory]) -> Vec<Finding> {
    let mut findings: Vec<Finding> = Vec::new();
    for package in &lock.packages {
        if !is_registry_source(&package.source) {
            continue;
        }
        for advisory in advisories {
            if advisory.package != package.name {
                continue;
            }
            if !advisory_matches(advisory, &package.version, package.artifact_digest.as_deref()) {
                continue;
            }
            findings.push(Finding {
                advisory_id: advisory.id.clone(),
                package: package.name.clone(),
                version: package.version.clone(),
                severity: advisory.severity.clone(),
                title: advisory.title.clone(),
                detail: advisory.detail.clone(),
                pinned_artifact: !advisory.artifacts.is_empty(),
            });
        }
    }
    findings.sort_by(|a, b| {
        (a.severity.clone(), a.package.clone(), a.advisory_id.clone())
            .cmp(&(b.severity.clone(), b.package.clone(), b.advisory_id.clone()))
    });
    findings
}

pub fn has_hold(findings: &[Finding]) -> bool {
    findings.iter().any(|f| f.severity == "critical")
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::signature::KeyPair;

    fn make_advisory(id: &str, affected: &str, artifacts: &[&str], severity: &str) -> Advisory {
        Advisory {
            id: id.to_string(),
            package: "@acme/widget".to_string(),
            affected: affected.to_string(),
            artifacts: artifacts.iter().map(|s| s.to_string()).collect(),
            severity: severity.to_string(),
            title: format!("title {id}"),
            detail: String::new(),
        }
    }

    fn lock_with(version: &str, artifact: Option<&str>) -> ProjectDepLock {
        ProjectDepLock {
            version: crate::deplock::LOCK_VERSION,
            packages: vec![crate::deplock::LockedPackage {
                name: "@acme/widget".to_string(),
                version: version.to_string(),
                source: format!("registry:@acme/widget@{version}"),
                checksum: "x".to_string(),
                artifact_digest: artifact.map(|s| s.to_string()),
                license: Some("MIT".to_string()),
                link: Some("static".to_string()),
                dependencies: Vec::new(),
                tier: "pure".to_string(),
                capabilities: Vec::new(),
            }],
        }
    }

    #[test]
    fn golden_canonical_vector_matches_registry() {
        let advisories = vec![Advisory {
            id: "ADV-1".to_string(),
            package: "@acme/widget".to_string(),
            affected: "^1.0.0".to_string(),
            artifacts: Vec::new(),
            severity: "warning".to_string(),
            title: "Bad \"quotes\" \\ test".to_string(),
            detail: "line1\nline2\té".to_string(),
        }];
        assert_eq!(
            String::from_utf8(feed_canonical_bytes(&advisories)).unwrap(),
            "{\"advisories\":[{\"affected\":\"^1.0.0\",\"artifacts\":[],\"detail\":\"line1\\nline2\\té\",\"id\":\"ADV-1\",\"package\":\"@acme/widget\",\"severity\":\"warning\",\"title\":\"Bad \\\"quotes\\\" \\\\ test\"}]}"
        );
    }

    #[test]
    fn sign_verify_roundtrip_with_ring() {
        let private_key = ring::signature::Ed25519KeyPair::generate_pkcs8(
            &ring::rand::SystemRandom::new(),
        )
        .unwrap();
        let key = ring::signature::Ed25519KeyPair::from_pkcs8(private_key.as_ref()).unwrap();
        let advisories = vec![make_advisory("ADV-1", "^1.0.0", &[], "warning")];
        let message = feed_canonical_bytes(&advisories);
        let signature = key.sign(&message);
        let public_key = key.public_key().as_ref().to_vec();
        let feed = SignedFeed {
            advisories,
            key_id: key_id_for_public_key(&public_key),
            public_key: public_key.iter().map(|b| format!("{b:02x}")).collect(),
            signature: signature.as_ref().iter().map(|b| format!("{b:02x}")).collect(),
        };
        verify_feed(&feed).unwrap();
        let mut tampered = feed.clone();
        tampered.advisories.push(make_advisory("ADV-2", "*", &[], "critical"));
        assert!(verify_feed(&tampered).is_err());
        let mut wrong_key = feed.clone();
        wrong_key.key_id = "deadbeefdeadbeef".to_string();
        assert!(verify_feed(&wrong_key).is_err());
    }

    #[test]
    fn version_range_matching_follows_registry_grammar() {
        let advisory = make_advisory("ADV-1", "^1.0.0", &[], "warning");
        assert!(advisory_matches(&advisory, "1.4.3", None));
        assert!(!advisory_matches(&advisory, "2.0.0", None));
        assert!(!advisory_matches(&advisory, "1.0.0-rc.1", None));
        let exact = make_advisory("ADV-2", "1.2.3", &[], "info");
        assert!(advisory_matches(&exact, "1.2.3", None));
        let broken = make_advisory("ADV-3", "<1.0.0", &[], "warning");
        assert!(!advisory_matches(&broken, "1.2.3", None));
    }

    #[test]
    fn artifact_pins_match_exactly() {
        let pin = format!("sha256:{}", "ab".repeat(32));
        let other = format!("sha256:{}", "cd".repeat(32));
        let advisory = make_advisory("ADV-1", "*", &[&pin], "critical");
        assert!(advisory_matches(&advisory, "1.0.0", Some(&pin)));
        assert!(!advisory_matches(&advisory, "1.0.0", Some(&other)));
        assert!(!advisory_matches(&advisory, "1.0.0", None));
        let unpinned = make_advisory("ADV-2", "*", &[], "warning");
        assert!(advisory_matches(&unpinned, "1.0.0", None));
        assert!(advisory_matches(&unpinned, "1.0.0", Some(&other)));
    }

    #[test]
    fn lock_matching_reports_tiered_findings() {
        let pin = format!("sha256:{}", "ab".repeat(32));
        let lock = lock_with("1.4.3", Some(&pin));
        let advisories = vec![
            make_advisory("ADV-1", "^1.0.0", &[], "warning"),
            make_advisory("ADV-2", "*", &[&pin], "critical"),
            make_advisory("ADV-3", "^2.0.0", &[], "warning"),
        ];
        let findings = match_lock(&lock, &advisories);
        assert_eq!(findings.len(), 2);
        assert!(has_hold(&findings));
        let critical: Vec<&Finding> = findings.iter().filter(|f| f.severity == "critical").collect();
        assert_eq!(critical.len(), 1);
        assert!(critical[0].pinned_artifact);
        let clean = match_lock(&lock_with("3.0.0", None), &advisories);
        assert!(clean.is_empty());
        assert!(!has_hold(&clean));
    }

    #[test]
    fn lock_matching_skips_non_registry_sources() {
        let lock = ProjectDepLock {
            version: crate::deplock::LOCK_VERSION,
            packages: vec![crate::deplock::LockedPackage {
                name: "@acme/widget".to_string(),
                version: "1.0.0".to_string(),
                source: "path:../widget".to_string(),
                checksum: "x".to_string(),
                artifact_digest: None,
                license: Some("MIT".to_string()),
                link: Some("static".to_string()),
                dependencies: Vec::new(),
                tier: "pure".to_string(),
                capabilities: Vec::new(),
            }],
        };
        let findings = match_lock(&lock, &[make_advisory("ADV-1", "*", &[], "critical")]);
        assert!(findings.is_empty());
    }
}
