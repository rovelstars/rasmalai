use diagnostics::{Code, Diagnostic};
use std::path::{Path, PathBuf};
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
        cli::args::SysrootAction::Build {
            triple,
            from,
            runtime,
            builtins,
            out,
            abi,
            loader,
            kernel_min,
            glibc_baseline,
            prelude,
            force,
        } => run_build(
            triple,
            from,
            runtime,
            builtins,
            out,
            abi,
            loader,
            kernel_min,
            glibc_baseline,
            prelude,
            force,
        ),
        cli::args::SysrootAction::Publish {
            dir,
            repo,
            version,
            tag,
            push_url,
            token,
            username,
        } => run_publish(dir, repo, version, tag, push_url, token, username),
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

const SYSROOT_SOURCE: &str = "https://github.com/rovelstars/rasmalai";

#[allow(clippy::too_many_arguments)]
fn run_build(
    triple: String,
    from: Vec<PathBuf>,
    runtime: Option<PathBuf>,
    builtins: Option<PathBuf>,
    out: PathBuf,
    abi: Option<u32>,
    loader: Option<String>,
    kernel_min: Option<String>,
    glibc_baseline: Option<String>,
    prelude: Option<String>,
    force: bool,
) {
    let target = match sysroot::target::Target::parse(&triple) {
        Ok(target) => target,
        Err(e) => fail(Diagnostic::new(Code::E108, e.to_string())),
    };
    let options = sysroot::build::BuildOptions {
        from,
        runtime,
        builtins,
        loader,
        kernel_min,
        glibc_baseline,
        prelude,
        abi,
        force,
    };
    match sysroot::build::build(&target, &out, &options) {
        Ok(built) => println!("built {} {} {}", built.target, built.digest, built.dir.display()),
        Err(e) => fail(e),
    }
}

fn run_publish(
    dir: Option<PathBuf>,
    repo: Option<String>,
    version: Option<String>,
    tag: Option<String>,
    push_url: Option<String>,
    token: Option<String>,
    username: Option<String>,
) {
    let dir = dir.unwrap_or_else(|| PathBuf::from("."));
    let pack = match Pack::load(&dir) {
        Ok(pack) => pack,
        Err(e) => fail(e.diagnostic()),
    };
    let repo = repo.unwrap_or_else(frontend::oci::sysroot_oci);
    let tag = match tag {
        Some(tag) => tag,
        None => frontend::oci::sysroot_tag(
            version.as_deref().unwrap_or(env!("CARGO_PKG_VERSION")),
            &pack.target.triple,
        ),
    };
    let token = match token.or_else(|| std::env::var(frontend::oci::OCI_TOKEN_ENV).ok()) {
        Some(token) if !token.trim().is_empty() => token,
        _ => fail(
            Diagnostic::new(Code::E108, "target pack push needs a registry token")
                .with_hint(format!("set {} or pass --token", frontend::oci::OCI_TOKEN_ENV)),
        ),
    };
    let username = username
        .or_else(|| std::env::var(frontend::oci::OCI_USERNAME_ENV).ok())
        .filter(|name| !name.trim().is_empty());
    let (registry, repo_path) = match frontend::oci::split_bare(&repo) {
        Ok(parts) => parts,
        Err(e) => fail(e),
    };
    let base = push_url
        .or_else(|| std::env::var(frontend::oci::PUSH_URL_ENV).ok())
        .unwrap_or_else(|| {
            format!(
                "{}://{registry}",
                frontend::oci::oci_scheme(&registry)
            )
        });
    let bytes = match frontend::oci::archive_pack_dir(&dir) {
        Ok(bytes) => bytes,
        Err(e) => fail(e),
    };
    let upstream_file = pack
        .manifest
        .link
        .libs
        .get("c")
        .and_then(|value| value.as_deref())
        .or(Some(pack.manifest.runtime.file.as_str()));
    let upstream_digest = match upstream_file {
        Some(name) => match pack.resolve_file(name) {
            Ok(path) => match sysroot::pack::file_sha256(&path) {
                Ok(digest) => digest,
                Err(e) => fail(e.diagnostic()),
            },
            Err(e) => fail(e.diagnostic()),
        },
        None => fail(Diagnostic::new(
            Code::E124,
            "target pack names no libc or runtime archive for the upstream annotation",
        )),
    };
    let input = frontend::oci::AssembleInput {
        role: "sysroot".to_string(),
        title: "sysroot.tar.gz".to_string(),
        kind: "sysroot".to_string(),
        target: pack.target.triple.clone(),
        platform: frontend::oci::platform_for_triple(&pack.target.triple),
        generator: format!("rnx-sysroot {}", env!("CARGO_PKG_VERSION")),
        upstream: format!("{}@{upstream_digest}", pack.manifest.libc),
        source: Some(SYSROOT_SOURCE.to_string()),
        bytes,
    };
    let assembly = match frontend::oci::assemble_index(&[input]) {
        Ok(assembly) => assembly,
        Err(e) => fail(e),
    };
    let auth = cli::publish::PushAuth {
        username: username.as_deref(),
        token: &token,
    };
    match cli::publish::push_assembly_as(&base, &repo_path, &tag, Some(auth), &assembly) {
        Ok(()) => println!("pushed {repo}:{tag} {}", assembly.digest),
        Err(e) => fail(e),
    }
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
