use std::path::PathBuf;

pub(super) fn run_bindgen(
    header: Vec<PathBuf>,
    native_lib: String,
    pkg_name: String,
    out: PathBuf,
    target: String,
    prefix: Vec<String>,
    skip: Vec<String>,
    include: Vec<PathBuf>,
    upstream_name: Option<String>,
    check: bool,
) {
    let opts = frontend::bindgen::BindgenOptions {
        headers: header,
        include_dirs: include,
        native_lib,
        pkg_name,
        target,
        prefixes: prefix,
        skips: skip,
        upstream_name,
        title: None,
    };
    let output = match frontend::bindgen::generate(&opts) {
        Ok(output) => output,
        Err(message) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
    };
    for warning in &output.warnings {
        eprintln!("warning: {warning}");
    }
    let rnx_path = out.join(frontend::bindgen::TITLE);
    let oci_path = out.join("bindgen.oci.json");
    if check {
        let mut drift = false;
        match std::fs::read_to_string(&rnx_path) {
            Ok(current) if current == output.rnx => {}
            Ok(_) => {
                eprintln!("bindgen drift: {} differs from the generated bindings", rnx_path.display());
                drift = true;
            }
            Err(e) => {
                eprintln!("bindgen drift: cannot read {}: {e}", rnx_path.display());
                drift = true;
            }
        }
        match std::fs::read_to_string(&oci_path) {
            Ok(current) if current == output.oci_json => {}
            Ok(_) => {
                eprintln!("bindgen drift: {} differs from the generated annotations", oci_path.display());
                drift = true;
            }
            Err(e) => {
                eprintln!("bindgen drift: cannot read {}: {e}", oci_path.display());
                drift = true;
            }
        }
        if drift {
            std::process::exit(1);
        }
        eprintln!(
            "bindgen clean: {} functions, {} constants ({} skipped)",
            output.emitted,
            output.consts,
            output.skipped.len()
        );
        return;
    }
    if let Err(e) = std::fs::create_dir_all(&out) {
        eprintln!("error: cannot create {}: {e}", out.display());
        std::process::exit(1);
    }
    if let Err(e) = std::fs::write(&rnx_path, &output.rnx) {
        eprintln!("error: cannot write {}: {e}", rnx_path.display());
        std::process::exit(1);
    }
    if let Err(e) = std::fs::write(&oci_path, &output.oci_json) {
        eprintln!("error: cannot write {}: {e}", oci_path.display());
        std::process::exit(1);
    }
    eprintln!(
        "bindgen: wrote {} functions to {} ({} skipped)",
        output.emitted,
        rnx_path.display(),
        output.skipped.len()
    );
}
