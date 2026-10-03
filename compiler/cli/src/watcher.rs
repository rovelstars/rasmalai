use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher as NotifyWatcher};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

const DEBOUNCE: Duration = Duration::from_millis(20);
const EMPTY_RETRY: Duration = Duration::from_millis(5);

fn ignored(path: &Path) -> bool {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if name.is_empty() || name.starts_with('.') || name.starts_with(".#") {
        return true;
    }
    if name.ends_with(".tmp") || name.ends_with('~') || name.ends_with(".swp") {
        return true;
    }
    path.components().any(|c| {
        let s = c.as_os_str().to_string_lossy();
        s == "target" || s == ".git"
    })
}

pub struct Watcher {
    rx: Receiver<Result<Event, notify::Error>>,
    _watcher: RecommendedWatcher,
    watched: BTreeSet<PathBuf>,
}

impl Watcher {
    pub fn new(files: &[PathBuf]) -> Result<Self, String> {
        let watched: BTreeSet<PathBuf> = files
            .iter()
            .filter(|p| !ignored(p) && p.exists())
            .map(|p| canonical(p))
            .collect();
        let mut dirs: BTreeSet<PathBuf> = BTreeSet::new();
        for f in &watched {
            let dir = f.parent().map(|d| d.to_path_buf()).unwrap_or_else(|| PathBuf::from("."));
            dirs.insert(dir);
        }
        let (tx, rx) = mpsc::channel();
        let mut watcher: RecommendedWatcher =
            NotifyWatcher::new(tx, Config::default()).map_err(|e| format!("watch init: {e}"))?;
        for d in &dirs {
            watcher
                .watch(d, RecursiveMode::NonRecursive)
                .map_err(|e| format!("watch {}: {e}", d.display()))?;
        }
        Ok(Watcher { rx, _watcher: watcher, watched })
    }

    pub fn set_files(&mut self, files: &[PathBuf]) -> Result<(), String> {
        *self = Self::new(files)?;
        Ok(())
    }

    fn relevant(&self, ev: &Event) -> Option<PathBuf> {
        match ev.kind {
            EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_) => {}
            _ => return None,
        }
        ev.paths.iter().find_map(|p| {
            let c = canonical(p);
            if self.watched.contains(&c) {
                Some(c)
            } else {
                None
            }
        })
    }

    pub fn poll(&mut self, timeout: Duration) -> Option<PathBuf> {
        let deadline = Instant::now() + timeout;
        let mut pending: Option<PathBuf> = None;
        loop {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            let wait = deadline.checked_duration_since(now).unwrap_or_default();
            let wait = if pending.is_none() { wait } else { wait.min(DEBOUNCE) };
            match self.rx.recv_timeout(wait) {
                Ok(Ok(ev)) => {
                    if let Some(p) = self.relevant(&ev) {
                        pending = Some(p);
                    }
                }
                Ok(Err(_)) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if pending.is_some() {
                        break;
                    }
                }
            }
        }
        let p = pending?;
        if std::fs::metadata(&p).map(|m| m.len()).unwrap_or(1) == 0 {
            std::thread::sleep(EMPTY_RETRY);
        }
        Some(p)
    }
}

fn canonical(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}
