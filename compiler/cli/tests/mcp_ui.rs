#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

use std::path::PathBuf;
use std::process::Command;

const APP: &str = r#"import { App } from "@std/app"
import { Build, Node } from "@std/ui"

class State {
    let count: Int = 0
    let name: String = ""
}

fn Main(): Int {
    let state = new State()
    let app = App.launch("mcp ui", 260, 140, (b: Build): Node => {
        b.animate(false)
        let root = b.column()
        root.addChild(b.button("Add", () => { state.count += 1 }))
        root.addChild(b.textField(state.name, "name", (v: String) => { state.name = v }))
        return root
    })
    return app.show()
}
"#;

const CONFIG: &str = r#"export default {
    project: { name: "mcp-ui", version: "0.1.0" },
    entries: { main: "main.rnx" },
    dependencies: {
        cpu_cabi: { native: "cpu_cabi", system: true, version: ">=0.1.0" },
        host_cabi: { native: "host_cabi", system: true, version: ">=0.1.0" },
        parley_cabi: { native: "parley_cabi", system: true, version: ">=0.1.0" },
    },
}
"#;

fn rnx() -> String {
    env!("CARGO_BIN_EXE_rnx").to_string()
}

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .expect("repo root")
}

#[test]
fn ui_script_runs_headless_through_run() {
    let dir = crate::scratch::dir("mcp-ui");
    std::fs::write(dir.join("main.rnx"), APP).unwrap();
    std::fs::write(dir.join("Project.config"), CONFIG).unwrap();
    let wrappers = repo().join(".rnx-cache/wrappers");
    let out = Command::new(rnx())
        .args(["run", "main.rnx"])
        .current_dir(&dir)
        .env("NO_COLOR", "1")
        .env(
            "RNX_UI_SCRIPT",
            "press:button:Add;wait:2;assert:button:Add",
        )
        .env("RNX_VENDOR_DIR", repo().join("compiler/stdlib/src"))
        .env("PKG_CONFIG_PATH", &wrappers)
        .env("LD_LIBRARY_PATH", &wrappers)
        .output()
        .expect("run rnx");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{stderr}{stdout}");
    assert!(stdout.contains("ui.result ok"), "{stderr}{stdout}");
    assert!(
        stdout.contains("ui.step 0 press:button:Add ok"),
        "{stderr}{stdout}"
    );
}
