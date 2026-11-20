use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

pub type Installer = unsafe extern "C" fn() -> i64;

static INSTALLER: AtomicUsize = AtomicUsize::new(0);
static RESTART: AtomicU64 = AtomicU64::new(0);

pub fn install(cb: Installer) {
    INSTALLER.store(cb as usize, Ordering::Release);
}

pub fn uninstall() {
    INSTALLER.store(0, Ordering::Release);
}

pub fn request_restart() {
    RESTART.store(1, Ordering::Release);
}

pub fn clear_restart() {
    RESTART.store(0, Ordering::Release);
}

pub fn restart_requested() -> bool {
    RESTART.load(Ordering::Acquire) != 0
}

/// Called by long-running runtime loops. Returns 0 when nothing is pending,
/// 1 when an update was installed, and 2 when the caller must unwind so the
/// supervisor can restart the process.
#[unsafe(no_mangle)]
pub extern "C" fn rnx_dev_tick() -> i64 {
    if restart_requested() {
        return 2;
    }
    let raw = INSTALLER.load(Ordering::Acquire);
    if raw == 0 {
        return 0;
    }
    let install: Installer = unsafe { std::mem::transmute(raw) };
    unsafe { install() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicI64;

    static NEXT: AtomicI64 = AtomicI64::new(0);

    unsafe extern "C" fn tick_once() -> i64 {
        NEXT.store(1, Ordering::SeqCst);
        1
    }

    #[test]
    fn tick_state_machine() {
        uninstall();
        clear_restart();
        NEXT.store(0, Ordering::SeqCst);
        assert_eq!(rnx_dev_tick(), 0);

        install(tick_once);
        assert_eq!(rnx_dev_tick(), 1);
        assert_eq!(NEXT.load(Ordering::SeqCst), 1);

        NEXT.store(0, Ordering::SeqCst);
        request_restart();
        assert_eq!(rnx_dev_tick(), 2);
        assert_eq!(NEXT.load(Ordering::SeqCst), 0);

        clear_restart();
        uninstall();
    }
}
