use crate::ichan::Heaps;
use crate::machine::ExecError;
use crate::value::Value;
use lir::instr::Module;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Condvar, LazyLock, Mutex};
use std::time::Duration;

struct ThreadReq {
    module: *const Module,
    func: usize,
    heaps: Heaps,
}

pub use crate::native::{TAG_BOOL, TAG_CLOSURE, TAG_FLOAT, TAG_INT, TAG_NULL, TAG_STR};

pub fn outcome_of(tag: u32, v: Value) -> (u64, Option<String>) {
    match (tag, v) {
        (TAG_INT, Value::Int(i)) => (i as u64, None),
        (TAG_BOOL, Value::Bool(b)) => (b as u64, None),
        (TAG_FLOAT, Value::Float(f, _)) => (f.to_bits(), None),
        (TAG_STR, Value::Str(s)) => (crate::native::alloc_str(&s) as u64, None),
        (TAG_NULL, Value::Null) => (0, None),
        (_, other) => (0, Some(format!("task returned {}, expected {}", actual_name(&other), tag_name(tag)))),
    }
}

fn actual_name(v: &Value) -> &'static str {
    match v {
        Value::Int(_) => "Int",
        Value::Bool(_) => "Bool",
        Value::Float(_, _) => "Float",
        Value::Str(_) => "String",
        Value::Null => "Null",
        _ => "a heap value",
    }
}

fn tag_name(tag: u32) -> &'static str {
    match tag {
        TAG_INT => "Int",
        TAG_BOOL => "Bool",
        TAG_FLOAT => "Float",
        TAG_STR => "String",
        TAG_NULL => "Null",
        _ => "a value",
    }
}


struct RawArg(usize);

unsafe impl Send for RawArg {}

static NEXT: AtomicI64 = AtomicI64::new(1);
static OUTPUTS: LazyLock<Mutex<BTreeMap<i64, Vec<String>>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));

struct ThreadOut {
    value: Value,
    output: Vec<String>,
}

#[derive(Clone)]
struct ThreadOutcome {
    value: Value,
    err: Option<String>,
    output: Vec<String>,
}

static OUTCOME_ERRS: LazyLock<Mutex<BTreeMap<i64, Option<String>>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));

const JOIN_TIMEOUT: Duration = Duration::from_secs(30);

struct ThreadSlot {
    state: Mutex<Option<ThreadOutcome>>,
    cond: Condvar,
}

static SLOTS: LazyLock<Mutex<BTreeMap<i64, Arc<ThreadSlot>>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));

fn alloc_slot() -> (i64, Arc<ThreadSlot>) {
    let token = NEXT.fetch_add(1, Ordering::SeqCst);
    let slot = Arc::new(ThreadSlot { state: Mutex::new(None), cond: Condvar::new() });
    SLOTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(token, slot.clone());
    (token, slot)
}

fn drop_slot(token: i64) {
    SLOTS.lock().unwrap_or_else(|e| e.into_inner()).remove(&token);
}

fn complete_slot(slot: &ThreadSlot, out: ThreadOutcome) {
    if let Ok(mut s) = slot.state.lock() {
        if s.is_none() {
            *s = Some(out);
        }
    }
    slot.cond.notify_all();
}

fn await_slot(slot: &ThreadSlot) -> Option<ThreadOutcome> {
    let mut s = slot.state.lock().unwrap_or_else(|e| e.into_inner());
    let deadline = std::time::Instant::now() + JOIN_TIMEOUT;
    while s.is_none() {
        let now = std::time::Instant::now();
        if now >= deadline {
            return None;
        }
        s = match slot.cond.wait_timeout(s, deadline - now) {
            Ok((g, _)) => g,
            Err(e) => e.into_inner().0,
        };
    }
    s.clone()
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn spawn_failed_msg() -> String {
    "Threading is disabled in the web playground. Rasmalai threads require a native OS target.".to_string()
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn spawn_failed_msg() -> String {
    "Thread failed to spawn (threading is unsupported on this platform or resource limit reached)".to_string()
}

fn thread_run(addr: usize) -> ThreadOut {
    let req = unsafe { Box::from_raw(addr as *mut ThreadReq) };
    let module = unsafe { &*req.module };
    let mut machine = crate::machine::Machine::new(module);
    machine.arena = req.heaps.objs.share();
    machine.arrays = req.heaps.arrs.share();
    machine.maps = req.heaps.maps.share();
    machine.gmaps = req.heaps.gmaps.share();
    let value = machine
        .run_fn_public(req.func, Vec::new())
        .map(|vs| vs.into_iter().next().unwrap_or(Value::Null))
        .unwrap_or(Value::Null);
    ThreadOut { value, output: machine.output }
}

#[cfg_attr(target_arch = "wasm32", allow(unreachable_code))]
pub fn spawn_with(module: *const Module, func: usize, heaps: Heaps) -> i64 {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (module, func, heaps);
        return -1;
    }
    let (token, slot) = alloc_slot();
    let req = RawArg(Box::into_raw(Box::new(ThreadReq { module, func, heaps })) as usize);
    let h = std::thread::Builder::new()
        .spawn(move || {
            crate::guard::guard_thread_init();
            let out = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| thread_run(req.0))) {
                Ok(o) => ThreadOutcome { value: o.value, err: None, output: o.output },
                Err(_) => ThreadOutcome {
                    value: Value::Null,
                    err: Some("Thread panicked".to_string()),
                    output: Vec::new(),
                },
            };
            complete_slot(&slot, out);
        })
        .ok();
    match h {
        Some(_) => token,
        None => {
            drop_slot(token);
            -token
        }
    }
}

pub fn join(token: i64) -> Result<Value, ExecError> {
    if token < 0 {
        return Err(ExecError::Fatal(spawn_failed_msg()));
    }
    let slot = SLOTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&token)
        .ok_or_else(|| ExecError::Fatal(format!("join of dead thread {token}")))?;
    let out = await_slot(&slot)
        .ok_or_else(|| ExecError::Fatal(format!("thread {token} join timed out after 30s")))?;
    OUTPUTS.lock().unwrap_or_else(|e| e.into_inner()).insert(token, out.output);
    match out.err {
        Some(m) => Err(ExecError::Fatal(m)),
        None => Ok(out.value),
    }
}

pub fn take_output(token: i64) -> Vec<String> {
    OUTPUTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&token)
        .unwrap_or_default()
}

#[cfg_attr(target_arch = "wasm32", allow(unreachable_code))]
pub fn spawn_closure_with(    module: *const Module,
    func: usize,
    captures: Vec<Value>,
    heaps: Heaps,
) -> i64 {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (module, func, captures, heaps);
        return -1;
    }
    struct ClosureReq {
        module: *const Module,
        func: usize,
        captures: Vec<Value>,
        heaps: Heaps,
    }
    unsafe impl Send for ClosureReq {}
    fn run(addr: usize) -> ThreadOutcome {
        let req = unsafe { Box::from_raw(addr as *mut ClosureReq) };
        let module = unsafe { &*req.module };
        let mut machine = crate::machine::Machine::new(module);
        machine.arena = req.heaps.objs.share();
        machine.arrays = req.heaps.arrs.share();
        machine.maps = req.heaps.maps.share();
        machine.gmaps = req.heaps.gmaps.share();
        let (value, err) = match machine
            .run_fn_public(req.func, req.captures)
            .map(|vs| vs.into_iter().next().unwrap_or(Value::Null)) {
            Ok(v) => (v, None),
            Err(ExecError::Fatal(m)) => (Value::Null, Some(m)),
            Err(ExecError::Throw(v)) => (Value::Null, Some(format!("uncaught {}", v.display()))),
        };
        if err.is_some() {
            eprintln!("worker-err func={} err={}", req.func, err.as_deref().unwrap_or(""));
        }
        ThreadOutcome { value, err, output: machine.output }
    }
    let (token, slot) = alloc_slot();
    let req = Box::into_raw(Box::new(ClosureReq { module, func, captures, heaps })) as usize;
    let h = std::thread::Builder::new()
        .spawn(move || {
            crate::guard::guard_thread_init();
            let out = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(req))) {
                Ok(o) => o,
                Err(_) => ThreadOutcome {
                    value: Value::Null,
                    err: Some("Thread panicked".to_string()),
                    output: Vec::new(),
                },
            };
            complete_slot(&slot, out);
        })
        .ok();
    match h {
        Some(_) => token,
        None => {
            drop_slot(token);
            -token
        }
    }
}

/// Convert a worker result into transport form, allocating native copies
/// for strings. Anything outside Int/Bool/Float/String/Null becomes an
/// error outcome instead of crossing the thread boundary.
pub fn store_value(v: Value) -> (u32, u64, Option<String>) {
    match v {
        Value::Int(i) => (TAG_INT, i as u64, None),
        Value::Bool(b) => (TAG_BOOL, b as u64, None),
        Value::Float(f, _) => (TAG_FLOAT, f.to_bits(), None),
        Value::Str(s) => (TAG_STR, crate::native::alloc_str(&s) as u64, None),
        Value::Null => (TAG_NULL, 0, None),
        other => (TAG_INT, 0, Some(format!("task returned {}, Thread.spawn supports Int/Bool/Float/String/Null results", actual_name(&other)))),
    }
}

pub fn join_value(token: i64) -> Result<(Value, Vec<String>), String> {
    if token < 0 {
        return Err(spawn_failed_msg());
    }
    if let Some(slot) = SLOTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&token)
    {
        let out = await_slot(&slot).ok_or_else(|| format!("thread {token} join timed out after 30s"))?;
        OUTPUTS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(token, out.output.clone());
        OUTCOME_ERRS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(token, out.err.clone());
        return Ok((out.value, out.output));
    }
    match join(token) {
        Ok(v) => {
            let output = take_output(token);
            Ok((v, output))
        }
        Err(ExecError::Fatal(m)) => {
            let output = take_output(token);
            OUTCOME_ERRS
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(token, Some(m));
            Ok((Value::Null, output))
        }
        Err(ExecError::Throw(v)) => {
            let output = take_output(token);
            OUTCOME_ERRS
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(token, Some(format!("uncaught {}", v.display())));
            Ok((Value::Null, output))
        }
    }
}

pub fn join_error_text(token: i64) -> String {
    OUTCOME_ERRS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&token)
        .flatten()
        .unwrap_or_default()
}

/// Build a native closure box from an interpreter closure value, copying
/// string captures. The caller owns the box and must release it.
pub fn closure_box_of(v: &Value) -> Result<*mut u8, String> {
    let Value::Closure(c) = v else {
        return Err("expected a closure".to_string());
    };
    unsafe {
        let b = crate::native::rnx_closure_new(c.func as u64, c.captures.len());
        if b.is_null() {
            return Err("out of memory".to_string());
        }
        for (i, cap) in c.captures.iter().enumerate() {
            let (tag, val) = match cap {
                Value::Int(x) => (TAG_INT, *x as u64),
                Value::Bool(x) => (TAG_BOOL, *x as u64),
                Value::Float(x, _) => (TAG_FLOAT, x.to_bits()),
                Value::Str(s) => (TAG_STR, crate::native::alloc_str(s) as u64),
                Value::Null => (TAG_INT, 0),
                Value::Closure(_) => {
                    let inner = closure_box_of(cap)?;
                    (TAG_CLOSURE, inner as u64)
                }
                other => {
                    crate::native::rnx_closure_release(b);
                    return Err(format!("closures cannot capture {}", actual_name(other)));
                }
            };
            crate::native::rnx_closure_set(b, i, tag as u64, val, 0, 0, 0, 0);
        }
        Ok(b)
    }
}


pub fn pool_interp_run(call: crate::native::PoolInterpCall) {
    let heaps: Box<crate::ichan::Heaps> =
        unsafe { Box::from_raw(call.heaps as *mut crate::ichan::Heaps) };
    let module = unsafe { &*(call.module as *const Module) };
    let mut machine = crate::machine::Machine::new(module);
    machine.arena = heaps.objs.share();
    machine.arrays = heaps.arrs.share();
    machine.maps = heaps.maps.share();
    machine.gmaps = heaps.gmaps.share();
    if call.closure_box != 0 {
        run_closure_task(&mut machine, module, call, &heaps);
        return;
    }
    let mut run = |arg: Option<i64>| {
        let args = arg.map(|a| vec![Value::Int(a)]).unwrap_or_default();
        match machine
            .run_fn_public(call.func, args)
            .map(|vs| vs.into_iter().next().unwrap_or(Value::Null)) {
            Ok(v) => {
                if let Some(slot) = call.result.as_ref() {
                    let (payload, err) = outcome_of(call.ret_tag, v);
                    crate::native::task_complete_pub(slot, call.ret_tag, payload, err);
                }
            }
            Err(e) => {
                let msg = match e {
                    crate::machine::ExecError::Fatal(m) => m,
                    crate::machine::ExecError::Throw(v) => format!("uncaught {}", v.display()),
                };
                if let Some(slot) = call.result.as_ref() {
                    crate::native::task_complete_pub(slot, TAG_INT, 0, Some(msg));
                }
            }
        }
    };
    if call.is_range {
        let mut i = call.start;
        while i < call.end {
            run(Some(i));
            i += 1;
        }
    } else {
        run(call.has_arg.then_some(call.arg));
    }
}

fn box_captures(box_ptr: usize) -> Option<(usize, Vec<Value>)> {
    if box_ptr == 0 {
        return None;
    }
    unsafe {
        let b = box_ptr as *const u8;
        let fid = (b.add(8) as *const u64).read_unaligned() as usize;
        let ncaps = (b.add(16) as *const u64).read_unaligned() as usize;
        if ncaps > 64 {
            return None;
        }
        let mut caps = Vec::with_capacity(ncaps);
        for i in 0..ncaps {
            let base = b.add(24).add(i.wrapping_mul(16));
            let tag = (base as *const u64).read_unaligned() as u32;
            let val = (base.add(8) as *const u64).read_unaligned();
            caps.push(match tag {
                TAG_BOOL => Value::Bool(val != 0),
                TAG_FLOAT => Value::Float(f64::from_bits(val), lir::instr::FloatKind::Strict),
                TAG_STR => Value::Str(crate::native::native_str(val as *const u8)),
                TAG_CLOSURE => match box_captures(val as usize) {
                    Some((inner_fid, inner_caps)) => Value::Closure(crate::value::ClosureVal {
                        func: inner_fid,
                        captures: inner_caps,
                        decay: false,
                        decay_this: false,
                    }),
                    None => return None,
                },
                _ => Value::Int(val as i64),
            });
        }
        Some((fid, caps))
    }
}

fn run_closure_task(machine: &mut crate::machine::Machine, _module: &Module, call: crate::native::PoolInterpCall, heaps: &crate::ichan::Heaps) {
    struct Guard(usize);
    impl Drop for Guard {
        fn drop(&mut self) {
            unsafe {
                crate::native::rnx_closure_release(self.0 as *mut u8);
            }
        }
    }
    let _guard = Guard(call.closure_box);
    machine.arena = heaps.objs.share();
    machine.arrays = heaps.arrs.share();
    machine.maps = heaps.maps.share();
    machine.gmaps = heaps.gmaps.share();
    let finish = |tag: u32, payload: u64, err: Option<String>| {
        if let Some(slot) = call.result.as_ref() {
            crate::native::task_complete_pub(slot, tag, payload, err);
        }
    };
    let Some((fid, mut caps)) = box_captures(call.closure_box) else {
        finish(TAG_INT, 0, Some("bad closure box".to_string()));
        return;
    };
    if call.is_range {
        let mut i = call.start;
        while i < call.end {
            let mut argv = vec![Value::Int(i)];
            argv.extend(caps.clone());
            if machine.run_fn_public(fid, argv).is_err() {
                break;
            }
            i += 1;
        }
        return;
    }
    if call.has_arg {
        caps.insert(0, Value::Int(call.arg));
    }
    let slot: &crate::native::TaskBox = match call.result.as_ref() {
        Some(s) => s,
        None => return,
    };
    match machine.run_fn_public(fid, caps).map(|vs| {
        vs.into_iter().next().unwrap_or(Value::Null)
    }) {
        Ok(v) => {
            let (payload, err) = outcome_of(call.ret_tag, v);
            crate::native::task_complete_pub(slot, call.ret_tag, payload, err);
        }
        Err(e) => {
            let msg = match e {
                crate::machine::ExecError::Fatal(m) => m,
                crate::machine::ExecError::Throw(v) => format!("uncaught {}", v.display()),
            };
            crate::native::task_complete_pub(slot, TAG_INT, 0, Some(msg));
        }
    }
}

#[cfg(test)]
mod thread_negative_tests {
    use super::join;
    use super::join_value;
    use crate::machine::ExecError;

    #[test]
    fn join_of_negative_handle_reports_spawn_failure() {
        match join(-1) {
            Err(ExecError::Fatal(m)) => {
                assert!(!m.contains("dead thread"), "{m}");
                assert!(
                    m.contains("failed to spawn") || m.contains("disabled"),
                    "{m}"
                );
            }
            other => panic!("expected spawn-failure error, got {other:?}"),
        }
    }

    #[test]
    fn join_value_of_negative_handle_reports_spawn_failure() {
        match join_value(-7) {
            Err(m) => {
                assert!(!m.contains("dead thread"), "{m}");
                assert!(
                    m.contains("failed to spawn") || m.contains("disabled"),
                    "{m}"
                );
            }
            other => panic!("expected spawn-failure error, got {other:?}"),
        }
    }
}
