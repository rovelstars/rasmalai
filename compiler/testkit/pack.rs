//! Hand-built target pack fixtures for tests.
//!
//! A fixture is a complete `sysroot.json` v2 pack: the manifest is derived
//! from the typed target descriptor, so capabilities and tier always agree
//! with the compiler. Every payload file is a placeholder; Stage 6 never
//! reads link inputs, it only validates and selects the pack.
//!
//! A test target pulls this in at its crate root:
//!
//! ```ignore
//! #[path = "../../testkit/pack.rs"]
//! mod pack_fixture;
//! ```

use std::path::Path;

#[allow(dead_code)]
pub fn digest_of(dir: &Path) -> String {
    sysroot::pack::Pack::digest(dir).unwrap_or_else(|e| panic!("pack fixture digest: {e}"))
}

#[allow(dead_code)]
pub fn write_pack(dir: &Path, triple: &str, salt: &str) -> String {
    write(dir, triple, salt, None)
}

#[allow(dead_code)]
pub fn write_pack_with_search(dir: &Path, triple: &str, search: &str) -> String {
    write(dir, triple, "search", Some(search))
}

fn write(dir: &Path, triple: &str, salt: &str, search: Option<&str>) -> String {
    let target = sysroot::target::Target::parse(triple)
        .unwrap_or_else(|e| panic!("pack fixture target `{triple}`: {e}"));
    let lib = format!("lib/rustlib/{triple}/lib");
    let lib_dir = dir.join(&lib);
    std::fs::create_dir_all(&lib_dir)
        .unwrap_or_else(|e| panic!("create {}: {e}", lib_dir.display()));
    for (name, bytes) in [
        ("crt1.o", format!("crt1 {salt}").into_bytes()),
        ("crtn.o", format!("crtn {salt}").into_bytes()),
        ("libc.a", format!("libc {salt}").into_bytes()),
        (
            "libruntime_native.a",
            format!("runtime {salt}").into_bytes(),
        ),
    ] {
        std::fs::write(lib_dir.join(name), bytes).unwrap();
    }
    let capabilities: Vec<String> = target
        .capabilities
        .iter()
        .map(|capability| format!("\"{}\"", capability.as_str()))
        .collect();
    let search = search.map(str::to_string).unwrap_or(lib.clone());
    let manifest = |digest: &str| {
        format!(
            "{{\n  \"version\": 2,\n  \"target\": \"{triple}\",\n  \"arch\": \"{}\",\n  \"os\": \"{}\",\n  \"env\": \"{}\",\n  \"object\": \"{}\",\n  \"libc\": \"{}\",\n  \"pointer_width\": {},\n  \"endian\": \"{}\",\n  \"tier\": {},\n  \"capabilities\": [{}],\n  \"cpu\": {{\"model\": \"generic\", \"features\": \"\"}},\n  \"loader\": null,\n  \"kernel_min\": null,\n  \"glibc_baseline\": null,\n  \"prelude\": \"@std/prelude\",\n  \"link\": {{\n    \"search\": [\"{}\"],\n    \"crt\": {{\"static\": [\"crt1.o\"]}},\n    \"crt_end\": {{\"static\": [\"crtn.o\"]}},\n    \"libs\": {{\"c\": \"libc.a\"}},\n    \"builtins\": null,\n    \"runtime\": \"libruntime_native.a\"\n  }},\n  \"runtime\": {{\"abi\": 1, \"file\": \"libruntime_native.a\", \"sha256\": null}},\n  \"integrity\": {{\"algorithm\": \"sha256\", \"digest\": \"{digest}\"}}\n}}\n",
            target.arch.as_str(),
            target.os.as_str(),
            target.env.as_str(),
            target.object.as_str(),
            target.libc.as_str(),
            target.pointer_width,
            target.endian.as_str(),
            match target.tier.as_str() {
                "1" => 1,
                "2" => 2,
                _ => 3,
            },
            capabilities.join(", "),
            search,
        )
    };
    std::fs::write(
        dir.join("sysroot.json"),
        manifest(&format!("sha256:{}", "0".repeat(64))),
    )
    .unwrap();
    let digest = digest_of(dir);
    std::fs::write(dir.join("sysroot.json"), manifest(&digest)).unwrap();
    digest
}
