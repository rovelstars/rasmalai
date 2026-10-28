use super::collections::GENREF_TABLE;
use super::guard::guard_install;
use std::cell::RefCell;

static INIT_ONCE: std::sync::Once = std::sync::Once::new();

thread_local! {
    static EXPORT_STACK: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
    static LAST_ERROR_MSG: RefCell<String> = const { RefCell::new(String::new()) };
    static LAST_ERROR_C: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_ensure_init() {
    INIT_ONCE.call_once(|| {
        guard_install();
        match GENREF_TABLE.lock() {
            Ok(guard) => drop(guard),
            Err(_) => {}
        }
    });
    guard_install();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_export_push(buf: *mut u8) {
    EXPORT_STACK.with(|s| {
        if let Ok(mut stack) = s.try_borrow_mut() {
            stack.push(buf as usize);
        }
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_export_pop() {
    EXPORT_STACK.with(|s| {
        if let Ok(mut stack) = s.try_borrow_mut() {
            stack.pop();
        }
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_export_clear() {
    LAST_ERROR_MSG.with(|m| {
        if let Ok(mut msg) = m.try_borrow_mut() {
            msg.clear();
        }
    });
    LAST_ERROR_C.with(|c| {
        if let Ok(mut bytes) = c.try_borrow_mut() {
            bytes.clear();
        }
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_export_drain() -> bool {
    let w = super::collections::rnx_error_take();
    if w == 0 {
        return false;
    }
    let msg = if super::collections::error_is_object(w) {
        let id = unsafe { super::collections::rnx_obj_class((w ^ 1) as *const u8) };
        if id == 0 {
            "<object>".to_string()
        } else {
            super::collections::TYPE_NAMES
                .lock()
                .map(|g| g.get(&id).cloned().unwrap_or_else(|| "<object>".to_string()))
                .unwrap_or_else(|_| "<object>".to_string())
        }
    } else {
        String::from_utf8_lossy(super::common::str_bytes(w as *const u8)).into_owned()
    };
    unsafe { super::collections::rnx_error_release(w) };
    LAST_ERROR_MSG.with(|m| {
        if let Ok(mut slot) = m.try_borrow_mut() {
            *slot = msg;
        }
    });
    true
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_last_error() -> *const u8 {
    static EMPTY: u8 = 0;
    let msg = LAST_ERROR_MSG.with(|m| m.try_borrow().map(|g| g.clone()).unwrap_or_default());
    if msg.is_empty() {
        return &EMPTY as *const u8;
    }
    LAST_ERROR_C.with(|c| match c.try_borrow_mut() {
        Ok(mut bytes) => {
            bytes.clear();
            bytes.extend_from_slice(msg.as_bytes());
            bytes.push(0);
            bytes.as_ptr()
        }
        Err(_) => &EMPTY as *const u8,
    })
}

pub(crate) fn export_depth() -> usize {
    EXPORT_STACK.with(|s| s.try_borrow().map(|g| g.len()).unwrap_or(0))
}

pub(crate) fn export_trap(msg: String) -> ! {
    LAST_ERROR_MSG.with(|m| {
        if let Ok(mut slot) = m.try_borrow_mut() {
            *slot = msg;
        }
    });
    let top = EXPORT_STACK.with(|s| s.try_borrow_mut().map(|mut g| g.pop()).unwrap_or(None));
    match top {
        Some(addr) if addr != 0 => {
            #[cfg(not(target_arch = "wasm32"))]
            unsafe {
                _longjmp(addr as *mut u8, 1);
            }
            #[cfg(target_arch = "wasm32")]
            std::process::abort();
        }
        _ => std::process::abort(),
    }
}

#[cfg(not(target_arch = "wasm32"))]
unsafe extern "C" {
    fn _longjmp(env: *mut u8, val: i32) -> !;
}
