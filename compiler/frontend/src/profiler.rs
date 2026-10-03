use std::path::PathBuf;
use std::time::Instant;

pub struct TraceEvent {
    pub name: &'static str,
    pub cat: &'static str,
    pub ph: &'static str,
    pub ts_us: u64,
    pub dur_us: u64,
    pub pid: u32,
    pub tid: u32,
}

pub struct PassProfiler {
    enabled: bool,
    trace_file: Option<PathBuf>,
    records: Vec<(&'static str, u64, u64)>,
    sub_records: Vec<(&'static str, &'static str, u64, u64)>,
    notes: Vec<(&'static str, usize)>,
    current: Option<(&'static str, Instant, u64)>,
    start_time: Instant,
    total_lines: usize,
}

impl PassProfiler {
    pub fn new(enabled: bool, trace_file: Option<PathBuf>) -> PassProfiler {
        PassProfiler {
            enabled,
            trace_file,
            records: Vec::new(),
            sub_records: Vec::new(),
            notes: Vec::new(),
            current: None,
            start_time: Instant::now(),
            total_lines: 0,
        }
    }

    pub fn disabled() -> PassProfiler {
        PassProfiler::new(false, None)
    }

    pub fn start(&mut self, pass_name: &'static str) {
        if self.current.is_some() {
            self.stop();
        }
        let now = Instant::now();
        let start_ns = now.duration_since(self.start_time).as_nanos() as u64;
        self.current = Some((pass_name, now, start_ns));
    }

    pub fn stop(&mut self) {
        if let Some((name, instant, start_ns)) = self.current.take() {
            let dur_ns = instant.elapsed().as_nanos() as u64;
            self.records.push((name, start_ns, dur_ns));
        }
    }

    pub fn records(&self) -> &[(&'static str, u64, u64)] {
        &self.records
    }

    pub fn record_sub(&mut self, parent: &'static str, name: &'static str, dur: std::time::Duration) {        let dur_ns = dur.as_nanos().min(u128::from(u64::MAX)) as u64;
        let elapsed_ns = self
            .start_time
            .elapsed()
            .as_nanos()
            .min(u128::from(u64::MAX)) as u64;
        let start_ns = elapsed_ns.saturating_sub(dur_ns);
        self.sub_records.push((parent, name, start_ns, dur_ns));
    }

    pub fn note(&mut self, key: &'static str, value: usize) {
        if let Some(slot) = self.notes.iter_mut().find(|(k, _)| *k == key) {
            slot.1 = value;
        } else {
            self.notes.push((key, value));
        }
    }

    pub fn total_lines(&self) -> usize {
        self.total_lines
    }

    pub fn set_lines(&mut self, total_lines: usize) {
        self.total_lines = total_lines;
    }

    pub fn print_summary(&self, total_lines: usize) {
        if !self.enabled || self.records.is_empty() {
            return;
        }
        let total_ns: u64 = self.records.iter().map(|(_, _, d)| d).sum();
        let total_ms = total_ns as f64 / 1_000_000.0;
        let rate = if total_ms > 0.0 {
            total_lines as f64 / (total_ms / 1000.0)
        } else {
            0.0
        };
        eprintln!("pass timings ({} lines):", total_lines);
        eprintln!("{:<14} {:>10} {:>6}", "pass", "ms", "%");
        for (name, _, dur_ns) in &self.records {
            let ms = *dur_ns as f64 / 1_000_000.0;
            let pct = if total_ns > 0 { *dur_ns as f64 * 100.0 / total_ns as f64 } else { 0.0 };
            eprintln!("{:<14} {:>10.3} {:>5.1}%", name, ms, pct);
            for (parent, sub, _, sub_dur_ns) in &self.sub_records {
                if *parent != *name {
                    continue;
                }
                let sub_ms = *sub_dur_ns as f64 / 1_000_000.0;
                let sub_pct =
                    if total_ns > 0 { *sub_dur_ns as f64 * 100.0 / total_ns as f64 } else { 0.0 };
                eprintln!("  {:<12} {:>10.3} {:>5.1}%", sub, sub_ms, sub_pct);
            }
        }
        eprintln!("{:<14} {:>10.3}  ({:.0} lines/sec)", "total", total_ms, rate);
        for (key, value) in &self.notes {
            eprintln!("note: {key} = {value}");
        }
    }

    fn events(&self) -> Vec<TraceEvent> {
        let pid = std::process::id();
        let mut out: Vec<TraceEvent> = self
            .records
            .iter()
            .map(|(name, start_ns, dur_ns)| TraceEvent {
                name,
                cat: "compiler",
                ph: "X",
                ts_us: start_ns / 1000,
                dur_us: dur_ns / 1000,
                pid,
                tid: 1,
            })
            .collect();
        out.extend(self.sub_records.iter().map(|(_, name, start_ns, dur_ns)| TraceEvent {
            name,
            cat: "compiler",
            ph: "X",
            ts_us: start_ns / 1000,
            dur_us: dur_ns / 1000,
            pid,
            tid: 1,
        }));
        out
    }

    fn escape_json(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        for c in s.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
                c => out.push(c),
            }
        }
        out
    }

    pub fn trace_json(&self) -> String {
        let mut out = String::from("{\"traceEvents\": [");
        for (i, e) in self.events().iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push('\n');
            out.push_str(&format!(
                "{{\"name\": \"{}\", \"cat\": \"{}\", \"ph\": \"{}\", \"ts\": {}, \"dur\": {}, \"pid\": {}, \"tid\": {}}}",
                Self::escape_json(e.name),
                e.cat,
                e.ph,
                e.ts_us,
                e.dur_us,
                e.pid,
                e.tid,
            ));
        }
        out.push_str("\n]}\n");
        out
    }

    pub fn write_trace_json(&self) {
        let path = match &self.trace_file {
            Some(p) => p,
            None => return,
        };
        if let Err(e) = std::fs::write(path, self.trace_json()) {
            eprintln!("warning: cannot write trace {}: {e}", path.display());
        }
    }

    pub fn finish(&self) {
        self.print_summary(self.total_lines);
        self.write_trace_json();
    }
}
