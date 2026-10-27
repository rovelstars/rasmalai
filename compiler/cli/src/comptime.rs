use diagnostics::Diagnostic;
use frontend::macros::{ComptimeRunner, MacroOptions};
use runtime::comptime::InterpComptime;

pub struct ExpandSession {
    opts: MacroOptions,
}

impl ExpandSession {
    pub fn new(base_dir: &std::path::Path, dump: Option<std::path::PathBuf>) -> ExpandSession {
        let mut opts = MacroOptions::new(base_dir.to_path_buf());
        opts.dump_dir = dump;
        ExpandSession { opts }
    }

    pub fn phase_a(
        &mut self,
        module: &mut frontend::ast::Module,
        profiler: Option<&mut frontend::profiler::PassProfiler>,
    ) -> Vec<Diagnostic> {
        if let Some(p) = profiler {
            p.start("expandA");
            let out = frontend::macros::expand_phase_a(module, &mut self.opts);
            p.stop();
            out
        } else {
            frontend::macros::expand_phase_a(module, &mut self.opts)
        }
    }

    pub fn phase_b(
        &mut self,
        module: &mut frontend::ast::Module,
        profiler: Option<&mut frontend::profiler::PassProfiler>,
    ) -> Vec<Diagnostic> {
        let runner = match InterpComptime::build(module) {
            Ok(r) => r,
            Err(e) => return vec![e],
        };
        let mut run = |opts: &mut MacroOptions| {
            frontend::macros::expand_phase_b(
                module,
                opts,
                runner.as_ref().map(|r| r as &dyn ComptimeRunner),
            )
        };
        if let Some(p) = profiler {
            p.start("expandB");
            let out = run(&mut self.opts);
            p.stop();
            out
        } else {
            run(&mut self.opts)
        }
    }
}

fn reborrow<'a, 'b>(
    p: Option<&'a mut &'b mut frontend::profiler::PassProfiler>,
) -> Option<&'a mut frontend::profiler::PassProfiler> {
    p.map(|p| &mut **p)
}

pub fn expand_all(
    module: &mut frontend::ast::Module,
    base_dir: &std::path::Path,
    dump: Option<std::path::PathBuf>,
    mut profiler: Option<&mut frontend::profiler::PassProfiler>,
) -> Vec<Diagnostic> {
    let mut session = ExpandSession::new(base_dir, dump);
    let mut diags = session.phase_a(module, reborrow(profiler.as_mut()));
    if diags.iter().any(|d| !d.code.is_warning()) {
        return diags;
    }
    diags.extend(session.phase_b(module, reborrow(profiler.as_mut())));
    diags
}

#[cfg(test)]
mod tests {
    #[test]
    fn expand_records_time_passes_spans_and_dump() {
        let dir = std::env::temp_dir().join(format!(
            "rnx-cli-macro-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("main.rnx"),
            "#[Derive(Clone)] struct S { let a: Int }\nfn Main(): Int { let v = vec![1]; StaticAssert!(1 == 1, \"x\"); return v[0]; }\n",
        )
        .unwrap();
        let graph = frontend::modules::ModuleGraph::build(&dir.join("main.rnx")).unwrap();
        let mut module = graph.resolve().unwrap();
        let dump = dir.join(".rnx-cache").join("expansions");
        let mut session = super::ExpandSession::new(&dir, Some(dump.clone()));
        let mut profiler = frontend::profiler::PassProfiler::new(true, None);
        let a = session.phase_a(&mut module, Some(&mut profiler));
        assert!(a.iter().all(|d| d.code.is_warning()), "{a:?}");
        let b = session.phase_b(&mut module, Some(&mut profiler));
        assert!(b.iter().all(|d| d.code.is_warning()), "{b:?}");
        let names: Vec<&str> = profiler.records().iter().map(|(n, _, _)| *n).collect();
        assert!(names.contains(&"expandA"), "{names:?}");
        assert!(names.contains(&"expandB"), "{names:?}");
        for f in ["phase-a.before", "phase-a.after", "phase-b.before", "phase-b.after"] {
            assert!(dump.join(f).is_file(), "{f}");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn derive_free_build_never_initializes_interpreter() {
        frontend::macros::reset_comptime_runs();
        let dir = std::env::temp_dir().join(format!(
            "rnx-cli-macro-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("main.rnx"), "fn Main(): Int { return 1; }\n").unwrap();
        let graph = frontend::modules::ModuleGraph::build(&dir.join("main.rnx")).unwrap();
        let mut module = graph.resolve().unwrap();
        let mut session = super::ExpandSession::new(&dir, None);
        let a = session.phase_a(&mut module, None);
        assert!(a.iter().all(|d| d.code.is_warning()), "{a:?}");
        let b = session.phase_b(&mut module, None);
        assert!(b.iter().all(|d| d.code.is_warning()), "{b:?}");
        assert_eq!(frontend::macros::comptime_runs(), 0);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
