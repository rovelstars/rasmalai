use crate::delta::{self, Delta};
use crate::watcher::Watcher;
use cranelift::jit::Jit;
use diagnostics::Diagnostic;
use frontend::ast as A;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

pub enum DevEvent {
    Swapped { names: Vec<String>, elapsed_ms: u128 },
    Restarted { reason: String },
    Broken { count: usize },
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct HotReport {
    pub status: String,
    pub swapped_functions: Vec<String>,
    pub compile_time_ms: u128,
    pub diagnostics: Vec<String>,
}

pub enum DevCommand {
    Rebuild { file: Option<PathBuf>, reply: mpsc::Sender<HotReport> },
}

fn render_plain(entry_file: &Path, errs: &[Diagnostic]) -> Vec<String> {
    let theme = diagnostics::theme::AuraTheme::plain();
    crate::report::render_compile_errors(&theme, entry_file, errs)
        .to_string()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.to_string())
        .collect()
}

pub fn hot_report(event: &DevEvent, diags: &[Diagnostic], entry_file: &Path, elapsed_ms: u128) -> HotReport {
    match event {
        DevEvent::Swapped { names, .. } => HotReport {
            status: if names.is_empty() { "no_change".to_string() } else { "swapped".to_string() },
            swapped_functions: names.clone(),
            compile_time_ms: elapsed_ms,
            diagnostics: Vec::new(),
        },
        DevEvent::Restarted { reason } => HotReport {
            status: "restarted".to_string(),
            swapped_functions: Vec::new(),
            compile_time_ms: elapsed_ms,
            diagnostics: vec![format!("structural change in {reason}; session restarted")],
        },
        DevEvent::Broken { .. } => HotReport {
            status: "error".to_string(),
            swapped_functions: Vec::new(),
            compile_time_ms: elapsed_ms,
            diagnostics: render_plain(entry_file, diags),
        },
    }
}

pub struct DevRunner {
    entry_file: PathBuf,
    entry: String,
    no_rerun: bool,
    jit: Jit,
    module: A::Module,
    entry_done: bool,
    watcher: Watcher,
}

pub fn build_program(    path: &Path,
    entry: &str,
) -> Result<(A::Module, lir::instr::Module, Vec<PathBuf>), Vec<Diagnostic>> {
    let graph = frontend::modules::ModuleGraph::build_collecting(path)?;
    let mut module = graph.resolve().map_err(|e| vec![e])?;
    let files: Vec<PathBuf> = graph.files.iter().map(|f| f.path.clone()).collect();
    frontend::harness::strip_tests(&mut module);
    frontend::harness::strip_benches(&mut module);
    let mut errors = frontend::desugar::desugar(&mut module);
    if errors.is_empty() {
        for d in frontend::semantic::check(&module) {
            if d.code.is_warning() {
                continue;
            }
            errors.push(d);
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let lowered = lir::lower::lower(&module).map_err(|e| vec![e])?;
    let mut lowered = lowered;
    // Dev builds must not inline: inlined callees bypass the dispatch table, so swaps would stay invisible.
    lir::opt::optimize_lir(&mut lowered, 0, entry);
    let verrs = lir::verify::verify(&lowered);
    if !verrs.is_empty() {
        return Err(verrs);
    }
    let iso = graph.isolation_errors();
    if !iso.is_empty() {
        return Err(iso);
    }
    Ok((module, lowered, files))
}

impl DevRunner {
    pub fn new(entry_file: &Path, entry: &str, no_rerun: bool) -> Result<Self, Vec<Diagnostic>> {
        let (module, lowered, files) = build_program(entry_file, entry)?;
        let jit = Jit::compile_hot(&lowered).map_err(|e| vec![e])?;
        let watcher = Watcher::new(&files).map_err(|m| {
            vec![Diagnostic::new(diagnostics::Code::E108, format!("rnx dev: {m}"))]
        })?;
        Ok(DevRunner {
            entry_file: entry_file.to_path_buf(),
            entry: entry.to_string(),
            no_rerun,
            jit,
            module,
            entry_done: false,
            watcher,
        })
    }

    pub fn call(&mut self, name: &str, args: &[i64]) -> Result<i64, Diagnostic> {
        self.jit.call(name, args)
    }

    pub fn run_entry(&mut self) -> Result<i64, Diagnostic> {
        let entry = self.entry.clone();
        let r = self.jit.call(&entry, &[]);
        if r.is_ok() {
            self.entry_done = true;
        }
        r
    }

    pub fn poll(&mut self, timeout: Duration) -> Option<DevEvent> {
        let changed = self.watcher.poll(timeout)?;
        let _ = changed;
        let (event, diags) = self.rebuild();
        if let DevEvent::Broken { .. } = event {
            let theme = diagnostics::theme::AuraTheme::active();
            eprint!(
                "{}",
                crate::report::render_compile_errors(&theme, &self.entry_file, &diags)
            );
        }
        Some(event)
    }

    pub fn rebuild(&mut self) -> (DevEvent, Vec<Diagnostic>) {
        match build_program(&self.entry_file, &self.entry) {
            Err(errs) => {
                let count = errs.len();
                (DevEvent::Broken { count }, errs)
            }
            Ok((module, lowered, files)) => match delta::classify(&self.module, &module) {
                Delta::Pure { swapped } => {
                    let started = Instant::now();
                    let mut done = Vec::new();
                    for name in &swapped {
                        let slot = match self.jit.slot_of(name) {
                            Some(s) => s,
                            None => continue,
                        };
                        match self.jit.hot_swap_function(slot, &lowered) {
                            Ok(()) => done.push(name.clone()),
                            Err(e) => {
                                eprintln!("[rnx dev] swap of `{name}` failed ({e:?}); restarting");
                                let (event, diags) = self.restart(format!("swap of `{name}` failed"), module, lowered, files);
                                return (event, diags);
                            }
                        }
                    }
                    self.module = module;
                    let _ = self.watcher.set_files(&files);
                    let elapsed_ms = started.elapsed().as_millis();
                    (DevEvent::Swapped { names: done, elapsed_ms }, Vec::new())
                }
                Delta::Structural { name } => self.restart(name, module, lowered, files),
            },
        }
    }

    fn restart(&mut self, reason: String, module: A::Module, lowered: lir::instr::Module, files: Vec<PathBuf>) -> (DevEvent, Vec<Diagnostic>) {
        match Jit::compile_hot(&lowered) {
            Ok(jit) => {
                self.jit = jit;
                self.module = module;
                self.entry_done = false;
                let _ = self.watcher.set_files(&files);
                (DevEvent::Restarted { reason }, Vec::new())
            }
            Err(e) => (DevEvent::Broken { count: 1 }, vec![e]),
        }
    }

    pub fn should_rerun(&self) -> bool {
        self.entry_done && !self.no_rerun
    }

    pub fn entry_name(&self) -> &str {
        &self.entry
    }

    pub fn entry_file(&self) -> &Path {
        &self.entry_file
    }
}

fn rerun(runner: &mut DevRunner) {
    let entry = runner.entry_name().to_string();
    match runner.run_entry() {
        Ok(v) => eprintln!("[rnx dev] {entry}() -> {v}"),
        Err(e) => eprintln!("[rnx dev] entry failed: {e:?}"),
    }
}

pub fn run_watch_loop(mut runner: DevRunner, commands: Option<mpsc::Receiver<DevCommand>>) {
    rerun(&mut runner);
    eprintln!("[rnx dev] watching {}", runner.entry_file().display());
    loop {
        if let Some(rx) = commands.as_ref() {
            match rx.try_recv() {
                Ok(DevCommand::Rebuild { file, reply }) => {
                    if let Some(path) = file.as_ref() {
                        if !path.exists() {
                            let _ = reply.send(HotReport {
                                status: "error".to_string(),
                                swapped_functions: Vec::new(),
                                compile_time_ms: 0,
                                diagnostics: vec![format!("file `{}` does not exist", path.display())],
                            });
                            continue;
                        }
                    }
                    let started = Instant::now();
                    let entry_file = runner.entry_file().to_path_buf();
                    let (event, diags) = runner.rebuild();
                    let elapsed_ms = started.elapsed().as_millis();
                    if let DevEvent::Broken { .. } = event {
                        let theme = diagnostics::theme::AuraTheme::active();
                        eprint!(
                            "{}",
                            crate::report::render_compile_errors(&theme, &entry_file, &diags)
                        );
                    } else if runner.should_rerun() {
                        rerun(&mut runner);
                    }
                    let _ = reply.send(hot_report(&event, &diags, &entry_file, elapsed_ms));
                    continue;
                }
                Err(mpsc::TryRecvError::Disconnected) => break,
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        match runner.poll(Duration::from_millis(50)) {
            None => {}
            Some(DevEvent::Swapped { names, elapsed_ms }) => {
                eprintln!("[rnx dev] hot swapped {} in {elapsed_ms}ms", names.join(", "));
                if runner.should_rerun() {
                    rerun(&mut runner);
                }
            }
            Some(DevEvent::Restarted { reason }) => {
                eprintln!("[rnx dev] structural change detected in {reason} -> restarting");
                rerun(&mut runner);
            }
            Some(DevEvent::Broken { .. }) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn hot_report_maps_all_events() {
        let file = Path::new("main.rnx");
        let swapped = hot_report(
            &DevEvent::Swapped { names: vec!["f".to_string()], elapsed_ms: 3 },
            &[],
            file,
            3,
        );
        assert_eq!(swapped.status, "swapped");
        assert_eq!(swapped.swapped_functions, vec!["f".to_string()]);
        let idle = hot_report(&DevEvent::Swapped { names: vec![], elapsed_ms: 1 }, &[], file, 1);
        assert_eq!(idle.status, "no_change");
        let restarted = hot_report(&DevEvent::Restarted { reason: "g".to_string() }, &[], file, 5);
        assert_eq!(restarted.status, "restarted");
        assert!(restarted.diagnostics.join(" ").contains('g'));
        let broken = hot_report(&DevEvent::Broken { count: 2 }, &[], file, 7);
        assert_eq!(broken.status, "error");
    }
}
