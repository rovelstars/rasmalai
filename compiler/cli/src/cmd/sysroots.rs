use diagnostics::{Code, Diagnostic};
use std::path::Path;
use sysroot::pack::Pack;
use sysroot::store::Store;

#[derive(serde::Serialize)]
struct PackRow {
    triple: String,
    digest: String,
    tier: String,
    current: bool,
}

pub(super) fn run_sysroot(action: cli::args::SysrootAction) {
    match action {
        cli::args::SysrootAction::Add {
            spec,
            local,
            digest,
        } => run_add(spec, local, digest),
        cli::args::SysrootAction::List { json } => run_list(json),
        cli::args::SysrootAction::Path { triple } => run_path(&triple),
        cli::args::SysrootAction::Verify { triple } => run_verify(&triple),
        cli::args::SysrootAction::Remove { triple } => run_remove(&triple),
        cli::args::SysrootAction::Prune { keep } => run_prune(keep),
    }
}

fn fail(err: Diagnostic) -> ! {
    eprintln!("{err}");
    std::process::exit(1);
}

fn parse_spec(spec: &str) -> Result<(String, Option<String>), Diagnostic> {
    let (triple, version) = match spec.rsplit_once('@') {
        Some((triple, version)) if !triple.is_empty() && !version.is_empty() => {
            (triple, Some(version.to_string()))
        }
        _ => (spec, None),
    };
    let target = sysroot::target::Target::parse(triple)
        .map_err(|e| Diagnostic::new(Code::E108, e.to_string()))?;
    if let Some(version) = &version
        && (version.is_empty()
            || !version
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-' || b == b'+'))
    {
        return Err(Diagnostic::new(
            Code::E108,
            format!(
                "`@{version}` is not a pack version (expected letters, digits, `.`, `-`, or `+`)"
            ),
        ));
    }
    Ok((target.triple, version))
}

fn run_add(spec: Option<String>, local: Option<std::path::PathBuf>, digest: Option<String>) {
    if let Some(digest) = digest.as_deref()
        && !sysroot::pack::is_digest(digest)
    {
        fail(Diagnostic::new(
            Code::E108,
            format!("`--digest {digest}` must be `sha256:<64 lowercase hex chars>`"),
        ));
    }
    let store = Store::default_store();
    match local {
        Some(dir) => add_local(&store, spec.as_deref(), &dir, digest.as_deref()),
        None => add_remote(&store, spec.as_deref(), digest.as_deref()),
    }
}

fn add_local(store: &Store, spec: Option<&str>, dir: &Path, digest: Option<&str>) {
    let pack = match Pack::load(dir) {
        Ok(pack) => pack,
        Err(e) => fail(e.diagnostic()),
    };
    if let Some(spec) = spec {
        let (triple, version) = match parse_spec(spec) {
            Ok(parsed) => parsed,
            Err(e) => fail(e),
        };
        if version.is_some() {
            fail(Diagnostic::new(
                Code::E124,
                format!(
                    "`--local` installs the pack in `{}`; drop the `@version` suffix",
                    dir.display()
                ),
            ));
        }
        if triple != pack.target.triple {
            fail(Diagnostic::new(
                Code::E124,
                format!(
                    "pack `{}` carries target `{}`, not `{triple}`",
                    dir.display(),
                    pack.target.triple
                ),
            ));
        }
    }
    match store.install_from_dir(dir, digest) {
        Ok(installed) => println!("installed {} {}", installed.triple, installed.digest),
        Err(e) => fail(e.diagnostic()),
    }
}

fn add_remote(store: &Store, spec: Option<&str>, digest: Option<&str>) {
    let Some(spec) = spec else {
        fail(Diagnostic::new(
            Code::E108,
            "`rnx sysroot add` needs a target triple, or `--local <dir>` for a hand-built pack",
        ));
    };
    let (triple, version) = match parse_spec(spec) {
        Ok(parsed) => parsed,
        Err(e) => fail(e),
    };
    if sysroot::store::offline_flag() {
        fail(
            Diagnostic::new(
                Code::E124,
                format!("cannot install target pack `{triple}` while offline"),
            )
            .with_hint(format!(
                "re-run without `--offline`, or install a local pack with `rnx sysroot add --local <dir> {triple}`"
            )),
        );
    }
    let bare = frontend::oci::sysroot_oci();
    let tag = frontend::oci::sysroot_tag(
        version.as_deref().unwrap_or(env!("CARGO_PKG_VERSION")),
        &triple,
    );
    let download = match frontend::oci::download_sysroot(&bare, &tag, &triple, digest) {
        Ok(download) => download,
        Err(e) => fail(e),
    };
    let installed = store.install_from_dir(&download.dir, Some(&download.digest));
    let _ = std::fs::remove_dir_all(&download.dir);
    match installed {
        Ok(installed) => println!("installed {} {}", installed.triple, installed.digest),
        Err(e) => fail(e.diagnostic()),
    }
}

fn run_list(json: bool) {
    let rows: Vec<PackRow> = Store::default_store()
        .list()
        .into_iter()
        .map(|pack| PackRow {
            triple: pack.triple,
            digest: pack.digest,
            tier: pack.tier,
            current: pack.current,
        })
        .collect();
    if json {
        match serde_json::to_string_pretty(&rows) {
            Ok(text) => println!("{text}"),
            Err(e) => fail(Diagnostic::new(
                Code::E124,
                format!("cannot encode the target pack list: {e}"),
            )),
        }
        return;
    }
    if rows.is_empty() {
        println!("no target packs installed");
        return;
    }
    let triple_width = rows
        .iter()
        .map(|row| row.triple.len())
        .max()
        .unwrap_or(0)
        .max("TRIPLE".len());
    let digest_width = rows
        .iter()
        .map(|row| row.digest.len())
        .max()
        .unwrap_or(0)
        .max("DIGEST".len());
    let tier_width = rows
        .iter()
        .map(|row| row.tier.len())
        .max()
        .unwrap_or(0)
        .max("TIER".len());
    println!(
        "{:<triple_width$}  {:<digest_width$}  {:<tier_width$}  CURRENT",
        "TRIPLE", "DIGEST", "TIER"
    );
    for row in &rows {
        println!(
            "{:<triple_width$}  {:<digest_width$}  {:<tier_width$}  {}",
            row.triple,
            row.digest,
            row.tier,
            if row.current { "*" } else { "" }
        );
    }
}

fn run_path(triple: &str) {
    let store = Store::default_store();
    match store.current(triple) {
        Some(pack) => println!("{}", pack.path.display()),
        None => fail(
            Diagnostic::new(
                Code::E124,
                format!("no target pack installed for `{triple}`"),
            )
            .with_hint(format!("run `rnx sysroot add {triple}`")),
        ),
    }
}

fn run_verify(triple: &str) {
    match Store::default_store().verify(triple) {
        Ok(pack) => println!("verified {} {}", pack.triple, pack.digest),
        Err(e) => fail(e.with_hint(format!(
            "run `rnx sysroot add {triple}` to reinstall the pack"
        ))),
    }
}

fn run_remove(triple: &str) {
    match Store::default_store().remove(triple) {
        Ok(count) => println!("removed {count} pack(s) for {triple}"),
        Err(e) => fail(e.diagnostic()),
    }
}

fn run_prune(keep: Option<usize>) {
    match Store::default_store().prune(keep) {
        Ok(count) => println!("pruned {count} pack(s)"),
        Err(e) => fail(e.diagnostic()),
    }
}
