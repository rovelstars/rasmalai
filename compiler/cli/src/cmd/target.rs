use std::collections::BTreeMap;
use sysroot::store::Store;
use sysroot::target::{Target, Tier};

#[derive(serde::Serialize)]
struct TargetRow {
    triple: String,
    tier: String,
    object: String,
    capabilities: Vec<String>,
    pack: String,
}

pub(super) fn run_target(action: cli::args::TargetAction) {
    match action {
        cli::args::TargetAction::List { json, tier } => run_list(json, tier),
    }
}

fn pack_column(installed: &BTreeMap<String, String>, triple: &str) -> String {
    match installed.get(triple) {
        Some(digest) => {
            let prefix: String = digest.chars().take(15).collect();
            format!("installed {prefix}")
        }
        None => "missing".to_string(),
    }
}

fn run_list(json: bool, tier: Option<u8>) {
    let filter = tier.and_then(Tier::from_number);
    let installed: BTreeMap<String, String> = Store::default_store()
        .list()
        .into_iter()
        .filter(|pack| pack.current)
        .map(|pack| (pack.triple, pack.digest))
        .collect();
    let rows: Vec<TargetRow> = Target::all()
        .iter()
        .filter(|target| filter.is_none_or(|tier| target.tier == tier))
        .map(|target| TargetRow {
            triple: target.triple.clone(),
            tier: target.tier.as_str().to_string(),
            object: target.object.as_str().to_string(),
            capabilities: target
                .capabilities
                .iter()
                .map(|capability| capability.as_str().to_string())
                .collect(),
            pack: pack_column(&installed, &target.triple),
        })
        .collect();
    if json {
        match serde_json::to_string_pretty(&rows) {
            Ok(text) => println!("{text}"),
            Err(e) => {
                eprintln!("error: cannot encode target list: {e}");
                std::process::exit(1);
            }
        }
        return;
    }
    let triple_width = Target::all()
        .iter()
        .map(|target| target.triple.len())
        .max()
        .unwrap_or(0)
        .max("TRIPLE".len());
    let object_width = Target::all()
        .iter()
        .map(|target| target.object.as_str().len())
        .max()
        .unwrap_or(0)
        .max("OBJECT".len());
    println!(
        "{:<triple_width$}  {:<4}  {:<object_width$}  CAPABILITIES  PACK",
        "TRIPLE", "TIER", "OBJECT"
    );
    for row in &rows {
        println!(
            "{:<triple_width$}  {:<4}  {:<object_width$}  {}  {}",
            row.triple,
            row.tier,
            row.object,
            row.capabilities.join(","),
            row.pack
        );
    }
}
