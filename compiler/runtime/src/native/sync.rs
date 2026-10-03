use super::common::*;
use super::guard;
use super::string::*;
use super::collections::*;
use super::io::*;

std::thread_local! {
    static DEFER_STACK: std::cell::RefCell<Vec<u64>> = std::cell::RefCell::new(Vec::new());
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_defer_push(id: u64) {
    DEFER_STACK.with(|s| s.borrow_mut().push(id));
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_defer_pop() -> u64 {
    DEFER_STACK.with(|s| s.borrow_mut().pop().unwrap_or(u64::MAX))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_defer_len() -> u64 {
    DEFER_STACK.with(|s| s.borrow().len() as u64)
}

pub(crate) struct RawEntry(usize);

unsafe impl Send for RawEntry {}

pub(crate) struct RawPack(usize);

unsafe impl Send for RawPack {}

unsafe extern "C" fn thread_main(arg: usize) -> usize {
    let entry_addr = unsafe { (*((arg as *mut u8) as *const RawEntry)).0 };
    let entry: unsafe extern "C" fn() -> usize = unsafe { std::mem::transmute(entry_addr) };
    let out = unsafe { entry() };
    let _ = unsafe { Box::from_raw(arg as *mut (RawEntry, usize)) };
    out
}

pub(crate) static SYNC_ATOMICS: std::sync::LazyLock<std::sync::Mutex<std::collections::BTreeMap<i64, std::sync::atomic::AtomicI64>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::BTreeMap::new()));

pub(crate) struct ChannelQueue {
    pub(crate) queue: std::sync::Mutex<std::collections::VecDeque<ChanItem>>,
    pub(crate) cv: std::sync::Condvar,
}

pub(crate) enum ChanItem {
    Raw(i64),
    Heap(HeapItem),
}

pub(crate) struct HeapItem {
    pub(crate) addr: usize,
    pub(crate) kind: HeapKind,
}

pub(crate) enum HeapKind {
    Str,
    Obj { size: usize, dtor: usize },
    Arr { elem_size: usize, elem_dtor: usize },
}

fn as_dtor(addr: usize) -> Option<unsafe extern "C" fn(*mut u8)> {
    if addr == 0 {
        None
    } else {
        unsafe { Some(std::mem::transmute::<usize, unsafe extern "C" fn(*mut u8)>(addr)) }
    }
}

impl HeapItem {
    fn release(&self) {
        let ptr = self.addr as *mut u8;
        unsafe {
            match self.kind {
                HeapKind::Str => rnx_release_str(ptr),
                HeapKind::Obj { size, dtor } => rnx_release(ptr, size, as_dtor(dtor)),
                HeapKind::Arr { elem_size, elem_dtor } => {
                    rnx_release_array(ptr, elem_size, as_dtor(elem_dtor))
                }
            }
        }
    }

    fn bits(&self) -> i64 {
        self.addr as i64
    }
}

pub(crate) static SYNC_CHANNELS: std::sync::LazyLock<
    std::sync::Mutex<std::collections::BTreeMap<i64, std::sync::Arc<ChannelQueue>>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::BTreeMap::new()));

pub(crate) fn with_atomic<T>(id: i64, f: impl FnOnce(&std::sync::atomic::AtomicI64) -> T) -> T {
    let mut table = SYNC_ATOMICS.lock().unwrap_or_else(|e| e.into_inner());
    let cell = table.entry(id).or_insert_with(|| std::sync::atomic::AtomicI64::new(0));
    f(cell)
}

pub(crate) fn sync_channel(id: i64) -> std::sync::Arc<ChannelQueue> {
    let mut table = SYNC_CHANNELS.lock().unwrap_or_else(|e| e.into_inner());
    table
        .entry(id)
        .or_insert_with(|| {
            std::sync::Arc::new(ChannelQueue {
                queue: std::sync::Mutex::new(std::collections::VecDeque::new()),
                cv: std::sync::Condvar::new(),
            })
        })
        .clone()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_sync_atomic_get(id: i64) -> i64 {
    with_atomic(id, |a| a.load(std::sync::atomic::Ordering::SeqCst))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_sync_atomic_set(id: i64, val: i64) {
    with_atomic(id, |a| a.store(val, std::sync::atomic::Ordering::SeqCst));
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_sync_atomic_fetch_add(id: i64, delta: i64) -> i64 {
    with_atomic(id, |a| a.fetch_add(delta, std::sync::atomic::Ordering::SeqCst))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_sync_atomic_cas(id: i64, expected: i64, new_val: i64) -> bool {
    with_atomic(id, |a| {
        a.compare_exchange(expected, new_val, std::sync::atomic::Ordering::SeqCst, std::sync::atomic::Ordering::SeqCst)
            .is_ok()
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_sync_channel_send(id: i64, val: i64) {
    unsafe { rnx_any_retain(val as u64) };
    let ch = sync_channel(id);
    ch.queue
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push_back(ChanItem::Raw(val));
    ch.cv.notify_one();
}

fn channel_push_heap(id: i64, item: HeapItem) {
    unsafe { rnx_retain(item.addr as *mut u8) };
    let ch = sync_channel(id);
    ch.queue
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push_back(ChanItem::Heap(item));
    ch.cv.notify_one();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_sync_channel_send_str(id: i64, ptr: *mut u8) {
    channel_push_heap(id, HeapItem { addr: ptr as usize, kind: HeapKind::Str });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_sync_channel_send_obj(
    id: i64,
    ptr: *mut u8,
    size: usize,
    dtor: usize,
) {
    channel_push_heap(id, HeapItem { addr: ptr as usize, kind: HeapKind::Obj { size, dtor } });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_sync_channel_send_array(
    id: i64,
    ptr: *mut u8,
    elem_size: usize,
    elem_dtor: usize,
) {
    channel_push_heap(
        id,
        HeapItem { addr: ptr as usize, kind: HeapKind::Arr { elem_size, elem_dtor } },
    );
}

fn channel_pop(item: Option<ChanItem>) -> Option<i64> {
    match item {
        Some(ChanItem::Raw(v)) => Some(v),
        Some(ChanItem::Heap(h)) => Some(h.bits()),
        None => None,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_sync_channel_recv(id: i64) -> i64 {
    let ch = sync_channel(id);
    let mut q = ch.queue.lock().unwrap_or_else(|e| e.into_inner());
    loop {
        match q.pop_front() {
            Some(ChanItem::Heap(h)) if matches!(h.kind, HeapKind::Str) => {
                let boxed = unsafe { rnx_any_box(TAG_STR, h.bits() as u64) };
                return boxed as i64;
            }
            Some(item) => {
                if let Some(v) = channel_pop(Some(item)) {
                    return v;
                }
            }
            None => {}
        }
        q = ch.cv.wait(q).unwrap_or_else(|e| e.into_inner());
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_sync_channel_try_recv(id: i64) -> i64 {
    let ch = sync_channel(id);
    match channel_pop(
        ch.queue.lock().unwrap_or_else(|e| e.into_inner()).pop_front(),
    ) {
        Some(v) => {
            let out = unsafe { rnx_any_unbox(v as u64) } as i64;
            unsafe { rnx_any_release(v as u64) };
            out
        }
        None => -1,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_sync_channel_drop(id: i64) {
    let ch = sync_channel(id);
    let mut q = ch.queue.lock().unwrap_or_else(|e| e.into_inner());
    while let Some(item) = q.pop_front() {
        match item {
            ChanItem::Heap(h) => h.release(),
            ChanItem::Raw(v) => unsafe { rnx_any_release(v as u64) },
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_sync_channel_len(id: i64) -> i64 {
    let ch = sync_channel(id);
    ch.queue.lock().unwrap_or_else(|e| e.into_inner()).len() as i64
}

pub(crate) struct MutexEntry {
    pub(crate) state: std::sync::Mutex<bool>,
    pub(crate) cv: std::sync::Condvar,
}

pub(crate) struct RwState {
    pub(crate) readers: u32,
    pub(crate) writer: bool,
}

pub(crate) struct RwLockEntry {
    pub(crate) state: std::sync::Mutex<RwState>,
    pub(crate) cv: std::sync::Condvar,
}

pub(crate) static SYNC_MUTEXES: std::sync::LazyLock<
    std::sync::Mutex<std::collections::BTreeMap<i64, std::sync::Arc<MutexEntry>>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::BTreeMap::new()));

pub(crate) static SYNC_RWLOCKS: std::sync::LazyLock<
    std::sync::Mutex<std::collections::BTreeMap<i64, std::sync::Arc<RwLockEntry>>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::BTreeMap::new()));

fn rw_state(e: &std::sync::Arc<RwLockEntry>) -> std::sync::MutexGuard<'_, RwState> {
    e.state.lock().unwrap_or_else(|e| e.into_inner())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_mutex_lock(id: i64) {
    let e = sync_mutex(id);
    let mut held = lock_state(&e);
    while *held {
        held = e.cv.wait(held).unwrap_or_else(|e| e.into_inner());
    }
    *held = true;
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_mutex_unlock(id: i64) {
    let e = sync_mutex(id);
    let mut held = lock_state(&e);
    if !*held {
        unsafe {
            let msg = format!("rnx_mutex_unlock of unheld mutex {id}");
            rnx_panic(msg.as_ptr(), msg.len());
        }
    }
    *held = false;
    e.cv.notify_one();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_mutex_try_lock(id: i64) -> bool {
    let e = sync_mutex(id);
    let mut held = lock_state(&e);
    if *held {
        return false;
    }
    *held = true;
    true
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_rwlock_read_lock(id: i64) {
    let e = sync_rwlock(id);
    let mut st = rw_state(&e);
    while st.writer {
        st = e.cv.wait(st).unwrap_or_else(|e| e.into_inner());
    }
    st.readers += 1;
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_rwlock_read_unlock(id: i64) {
    let e = sync_rwlock(id);
    let mut st = rw_state(&e);
    if st.readers == 0 {
        unsafe {
            let msg = format!("rnx_rwlock_read_unlock of unheld rwlock {id}");
            rnx_panic(msg.as_ptr(), msg.len());
        }
    }
    st.readers -= 1;
    if st.readers == 0 {
        e.cv.notify_all();
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_rwlock_write_lock(id: i64) {
    let e = sync_rwlock(id);
    let mut st = rw_state(&e);
    while st.writer || st.readers > 0 {
        st = e.cv.wait(st).unwrap_or_else(|e| e.into_inner());
    }
    st.writer = true;
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_rwlock_write_unlock(id: i64) {
    let e = sync_rwlock(id);
    let mut st = rw_state(&e);
    if !st.writer {
        unsafe {
            let msg = format!("rnx_rwlock_write_unlock of unheld rwlock {id}");
            rnx_panic(msg.as_ptr(), msg.len());
        }
    }
    st.writer = false;
    e.cv.notify_all();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_rwlock_try_read_lock(id: i64) -> bool {
    let e = sync_rwlock(id);
    let mut st = rw_state(&e);
    if st.writer {
        return false;
    }
    st.readers += 1;
    true
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_rwlock_try_write_lock(id: i64) -> bool {
    let e = sync_rwlock(id);
    let mut st = rw_state(&e);
    if st.writer || st.readers > 0 {
        return false;
    }
    st.writer = true;
    true
}

pub(crate) struct CondvarEntry {
    pub(crate) cv: std::sync::Condvar,
}

pub(crate) struct BarrierState {
    pub(crate) count: usize,
    pub(crate) threshold: usize,
    pub(crate) generation: usize,
}

pub(crate) struct BarrierEntry {
    pub(crate) state: std::sync::Mutex<BarrierState>,
    pub(crate) cv: std::sync::Condvar,
}

pub(crate) static SYNC_CONDVARS: std::sync::LazyLock<
    std::sync::Mutex<std::collections::BTreeMap<i64, std::sync::Arc<CondvarEntry>>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::BTreeMap::new()));

pub(crate) static SYNC_BARRIERS: std::sync::LazyLock<
    std::sync::Mutex<std::collections::BTreeMap<i64, std::sync::Arc<BarrierEntry>>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::BTreeMap::new()));

pub(crate) fn sync_condvar(id: i64) -> std::sync::Arc<CondvarEntry> {
    let mut table = SYNC_CONDVARS.lock().unwrap_or_else(|e| e.into_inner());
    table
        .entry(id)
        .or_insert_with(|| {
            std::sync::Arc::new(CondvarEntry {
                cv: std::sync::Condvar::new(),
            })
        })
        .clone()
}

pub(crate) fn sync_barrier(id: i64) -> std::sync::Arc<BarrierEntry> {
    let mut table = SYNC_BARRIERS.lock().unwrap_or_else(|e| e.into_inner());
    table
        .entry(id)
        .or_insert_with(|| {
            std::sync::Arc::new(BarrierEntry {
                state: std::sync::Mutex::new(BarrierState {
                    count: 0,
                    threshold: 0,
                    generation: 0,
                }),
                cv: std::sync::Condvar::new(),
            })
        })
        .clone()
}

fn reacquire(e: &std::sync::Arc<MutexEntry>, mut held: std::sync::MutexGuard<'_, bool>) {
    while *held {
        held = e.cv.wait(held).unwrap_or_else(|e| e.into_inner());
    }
    *held = true;
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_condvar_wait(cv_id: i64, mutex_id: i64) {
    let cv = sync_condvar(cv_id);
    let m = sync_mutex(mutex_id);
    let mut held = lock_state(&m);
    if !*held {
        unsafe {
            let msg = format!("rnx_condvar_wait on unheld mutex {mutex_id}");
            rnx_panic(msg.as_ptr(), msg.len());
        }
    }
    *held = false;
    m.cv.notify_one();
    held = cv.cv.wait(held).unwrap_or_else(|e| e.into_inner());
    reacquire(&m, held);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_condvar_wait_timeout(cv_id: i64, mutex_id: i64, millis: i64) -> bool {
    let cv = sync_condvar(cv_id);
    let m = sync_mutex(mutex_id);
    let mut held = lock_state(&m);
    if !*held {
        unsafe {
            let msg = format!("rnx_condvar_wait on unheld mutex {mutex_id}");
            rnx_panic(msg.as_ptr(), msg.len());
        }
    }
    *held = false;
    m.cv.notify_one();
    let span = std::time::Duration::from_millis(millis.max(0) as u64);
    let (back, outcome) = cv.cv.wait_timeout(held, span).unwrap_or_else(|e| e.into_inner());
    reacquire(&m, back);
    !outcome.timed_out()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_condvar_notify_one(cv_id: i64) {
    sync_condvar(cv_id).cv.notify_one();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_condvar_notify_all(cv_id: i64) {
    sync_condvar(cv_id).cv.notify_all();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_barrier_wait(id: i64, threshold: i64) -> bool {
    if threshold <= 0 {
        unsafe {
            let msg = format!("Barrier threshold must be greater than 0, got {threshold}");
            rnx_panic(msg.as_ptr(), msg.len());
        }
    }
    let e = sync_barrier(id);
    let mut st = e.state.lock().unwrap_or_else(|e| e.into_inner());
    if st.count == 0 {
        st.threshold = threshold as usize;
    }
    st.count += 1;
    if st.count >= st.threshold {
        st.count = 0;
        st.generation += 1;
        e.cv.notify_all();
        return true;
    }
    let round = st.generation;
    while st.generation == round {
        st = e.cv.wait(st).unwrap_or_else(|e| e.into_inner());
    }
    false
}

pub(crate) enum PoolTarget {
    Native(usize),
    Interp { module: usize, func: usize, heaps: usize },
    Closure { bbox: usize },
    ClosureInterp { module: usize, heaps: usize, bbox: usize },
}

pub(crate) enum PoolCall {
    Once { arg: i64, has_arg: bool },
    Range { start: i64, end: i64 },
}

pub(crate) struct PoolTask {
    pub(crate) target: PoolTarget,
    pub(crate) call: PoolCall,
    pub(crate) result: Option<std::sync::Arc<TaskBox>>,
    pub(crate) ret_tag: u32,
}

pub struct PoolInterpCall {
    pub module: usize,
    pub func: usize,
    pub heaps: usize,
    pub closure_box: usize,
    pub start: i64,
    pub end: i64,
    pub arg: i64,
    pub has_arg: bool,
    pub is_range: bool,
    pub result: Option<std::sync::Arc<TaskBox>>,
    pub ret_tag: u32,
}

pub type PoolInterpRunner = fn(PoolInterpCall);

pub(crate) static POOL_INTERP_RUNNER: std::sync::LazyLock<std::sync::Mutex<Option<PoolInterpRunner>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(None));

pub fn pool_set_interp_runner(runner: PoolInterpRunner) {
    *POOL_INTERP_RUNNER.lock().unwrap_or_else(|e| e.into_inner()) = Some(runner);
}

fn exec_native(entry: usize, arg: Option<i64>) -> u64 {
    unsafe {
        match arg {
            Some(a) => {
                let f: unsafe extern "C-unwind" fn(i64) -> u64 = std::mem::transmute(entry);
                f(a)
            }
            None => {
                let f: unsafe extern "C-unwind" fn() -> u64 = std::mem::transmute(entry);
                f()
            }
        }
    }
}

fn finish_task(task: &PoolTask, tag: u32, payload: u64) {
    if let Some(slot) = task.result.as_ref() {
        task_complete(slot, tag, payload, None);
    }
}

fn fail_task(task: &PoolTask, msg: &str) {
    if let Some(slot) = task.result.as_ref() {
        task_complete(slot, task.ret_tag, 0, Some(msg.to_string()));
    }
}

fn exec_pool_task(task: PoolTask) {
    match task.target {
        PoolTarget::Native(entry) => match task.call {
            PoolCall::Once { arg, has_arg } => {
                let out = exec_native(entry, has_arg.then_some(arg));
                finish_task(&task, task.ret_tag, out);
            }
            PoolCall::Range { start, end } => {
                let mut i = start;
                while i < end {
                    exec_native(entry, Some(i));
                    i += 1;
                }
            }
        },
        PoolTarget::Closure { bbox } => {
            let out = match task.call {
                PoolCall::Once { arg, has_arg } => unsafe {
                    if has_arg {
                        rnx_closure_invoke1(bbox as *mut u8, arg as u64)
                    } else {
                        rnx_closure_invoke0(bbox as *mut u8)
                    }
                },
                PoolCall::Range { start, end } => {
                    let mut i = start;
                    while i < end {
                        unsafe {
                            rnx_closure_invoke1(bbox as *mut u8, i as u64);
                        }
                        i += 1;
                    }
                    unsafe {
                        rnx_closure_release(bbox as *mut u8);
                    }
                    return;
                }
            };
            finish_task(&task, task.ret_tag, out);
            unsafe {
                rnx_closure_release(bbox as *mut u8);
            }
        }
        PoolTarget::Interp { module, func, heaps } => {
            let runner = POOL_INTERP_RUNNER
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone();
            let Some(runner) = runner else {
                fail_task(&task, "pool interpreter runner missing");
                return;
            };
            let (start, end, arg, has_arg, is_range) = match task.call {
                PoolCall::Once { arg, has_arg } => (0, 0, arg, has_arg, false),
                PoolCall::Range { start, end } => (start, end, 0, false, true),
            };
            let slot = task.result.clone();
            runner(PoolInterpCall { module, func, heaps, closure_box: 0, start, end, arg, has_arg, is_range, result: slot, ret_tag: task.ret_tag });
        }
        PoolTarget::ClosureInterp { module, heaps, bbox } => {
            let runner = POOL_INTERP_RUNNER
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone();
            let Some(runner) = runner else {
                fail_task(&task, "pool interpreter runner missing");
                return;
            };
            let (start, end, arg, has_arg, is_range) = match task.call {
                PoolCall::Once { arg, has_arg } => (0, 0, arg, has_arg, false),
                PoolCall::Range { start, end } => (start, end, 0, false, true),
            };
            let slot = task.result.clone();
            runner(PoolInterpCall { module, func: 0, heaps, closure_box: bbox, start, end, arg, has_arg, is_range, result: slot, ret_tag: task.ret_tag });
        }
    }
}

pub(crate) struct ThreadPoolEntry {
    pub(crate) queue: std::sync::Mutex<std::collections::VecDeque<PoolTask>>,
    pub(crate) work_cv: std::sync::Condvar,
    pub(crate) done_cv: std::sync::Condvar,
    pub(crate) active: std::sync::atomic::AtomicUsize,
    pub(crate) shutdown: std::sync::atomic::AtomicBool,
    pub(crate) workers: std::sync::Mutex<Vec<std::thread::JoinHandle<()>>>,
    pub(crate) size: usize,
}

pub(crate) static SYNC_THREAD_POOLS: std::sync::LazyLock<
    std::sync::Mutex<std::collections::BTreeMap<i64, std::sync::Arc<ThreadPoolEntry>>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::BTreeMap::new()));

pub(crate) fn pool_entry(id: i64) -> std::sync::Arc<ThreadPoolEntry> {
    SYNC_THREAD_POOLS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&id)
        .cloned()
        .unwrap_or_else(|| unsafe {
            let msg = format!("thread pool {id} missing");
            rnx_panic(msg.as_ptr(), msg.len());
        })
}

pub(crate) fn pool_push_task(e: &std::sync::Arc<ThreadPoolEntry>, task: PoolTask) {
    e.active.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    e.queue.lock().unwrap_or_else(|e| e.into_inner()).push_back(task);
    e.work_cv.notify_one();
}

pub(crate) fn pool_wait_idle(e: &std::sync::Arc<ThreadPoolEntry>) {
    let mut q = e.queue.lock().unwrap_or_else(|e| e.into_inner());
    while e.active.load(std::sync::atomic::Ordering::SeqCst) > 0 {
        q = e.done_cv.wait(q).unwrap_or_else(|e| e.into_inner());
    }
}

pub(crate) fn pool_worker(e: std::sync::Arc<ThreadPoolEntry>) {
    loop {
        let task = {
            let mut q = e.queue.lock().unwrap_or_else(|e| e.into_inner());
            while q.is_empty() && !e.shutdown.load(std::sync::atomic::Ordering::SeqCst) {
                q = e.work_cv.wait(q).unwrap_or_else(|e| e.into_inner());
            }
            match q.pop_front() {
                Some(t) => t,
                None => return,
            }
        };
        let result = task.result.clone();
        let ret_tag = task.ret_tag;
        let outcome =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| exec_pool_task(task)));
        if outcome.is_err()
            && let Some(slot) = result.as_ref()
        {
            task_complete(slot, ret_tag, 0, Some("pool task panicked".to_string()));
        }
        {
            let _guard = e.queue.lock().unwrap_or_else(|e| e.into_inner());
            if e.active.fetch_sub(1, std::sync::atomic::Ordering::SeqCst) == 1 {
                e.done_cv.notify_all();
            }
        }
    }
}

pub(crate) fn pool_stop(e: std::sync::Arc<ThreadPoolEntry>) {
    // The flag and the wakeup must share the queue lock: a worker
    // parked between check and wait would otherwise miss a lock-free
    // notify and hang join() forever.
    {
        let _guard = e.queue.lock().unwrap_or_else(|e| e.into_inner());
        e.shutdown.store(true, std::sync::atomic::Ordering::SeqCst);
        e.work_cv.notify_all();
    }
    let handles = std::mem::take(&mut *e.workers.lock().unwrap_or_else(|e| e.into_inner()));
    for h in handles {
        let _ = h.join();
    }
}

pub(crate) fn pool_check(id: i64) -> Result<(), String> {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = id;
        return Err(spawn_failed_msg());
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        if SYNC_THREAD_POOLS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(&id)
        {
            Ok(())
        } else {
            Err(format!("thread pool {id} missing"))
        }
    }
}

pub(crate) fn pool_submit_task(id: i64, task: PoolTask) {
    let e = pool_entry(id);
    pool_push_task(&e, task);
}

pub(crate) fn pool_join(id: i64) {
    let e = pool_entry(id);
    pool_wait_idle(&e);
}

#[unsafe(no_mangle)]
#[cfg_attr(target_arch = "wasm32", allow(unreachable_code))]
pub unsafe extern "C" fn rnx_thread_pool_init(id: i64, num_workers: i64) {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (id, num_workers);
        return;
    }
    let workers = num_workers.max(1) as usize;
    // Init is idempotent for a live pool at the same size so shared pools
    // (such as the @std/fs pool, ensured on every *Async call) survive
    // concurrent ensure calls. A different size still resets the pool.
    // The lookup and the swap share one lock hold so racing inits cannot
    // orphan entries; the old pool stops outside the lock.
    let e = {
        let mut table = SYNC_THREAD_POOLS.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(cur) = table.get(&id) {
            if cur.size == workers && !cur.shutdown.load(std::sync::atomic::Ordering::SeqCst) {
                return;
            }
        }
        let e = std::sync::Arc::new(ThreadPoolEntry {
            queue: std::sync::Mutex::new(std::collections::VecDeque::new()),
            work_cv: std::sync::Condvar::new(),
            done_cv: std::sync::Condvar::new(),
            active: std::sync::atomic::AtomicUsize::new(0),
            shutdown: std::sync::atomic::AtomicBool::new(false),
            workers: std::sync::Mutex::new(Vec::new()),
            size: workers,
        });
        let old = table.insert(id, e.clone());
        drop(table);
        if let Some(o) = old {
            pool_stop(o);
        }
        e
    };
    let mut handles = e.workers.lock().unwrap_or_else(|e| e.into_inner());
    for _ in 0..workers {
        let ec = e.clone();
        match std::thread::Builder::new().spawn(move || {
            guard::guard_thread_init();
            pool_worker(ec)
        }) {
            Ok(h) => handles.push(h),
            Err(_) => unsafe {
                rnx_panic(b"pool worker spawn failed\0".as_ptr(), "pool worker spawn failed".len());
            },
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_thread_pool_submit(id: i64, entry: usize, arg: i64, has_arg: i64) {
    pool_submit_task(
        id,
        PoolTask {
            target: PoolTarget::Native(entry),
            call: PoolCall::Once { arg, has_arg: has_arg != 0 },
            result: None,
            ret_tag: 0,
        },
    );
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_thread_pool_submit_handle(
    id: i64,
    entry: usize,
    arg: i64,
    has_arg: i64,
    ret_tag: u32,
) -> *mut u8 {
    let slot = task_box_new(ret_tag);
    let handle = task_box_handle(&slot);
    pool_submit_task(
        id,
        PoolTask {
            target: PoolTarget::Native(entry),
            call: PoolCall::Once { arg, has_arg: has_arg != 0 },
            result: Some(slot),
            ret_tag,
        },
    );
    handle
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_thread_pool_submit_closure(
    id: i64,
    bbox: usize,
    arg: i64,
    has_arg: i64,
    ret_tag: u32,
) -> *mut u8 {
    if bbox == 0 {
        return std::ptr::null_mut();
    }
    unsafe {
        rnx_retain(bbox as *mut u8);
    }
    let slot = task_box_new(ret_tag);
    let handle = task_box_handle(&slot);
    pool_submit_task(
        id,
        PoolTask {
            target: PoolTarget::Closure { bbox },
            call: PoolCall::Once { arg, has_arg: has_arg != 0 },
            result: Some(slot),
            ret_tag,
        },
    );
    handle
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_thread_pool_parallel_closure(
    id: i64,
    bbox: usize,
    start: i64,
    end: i64,
    chunk: i64,
) {
    if bbox == 0 {
        return;
    }
    let e = pool_entry(id);
    if end > start && chunk > 0 {
        let mut s = start;
        while s < end {
            let hi = (s + chunk).min(end);
            unsafe {
                rnx_retain(bbox as *mut u8);
            }
            pool_push_task(
                &e,
                PoolTask {
                    target: PoolTarget::Closure { bbox },
                    call: PoolCall::Range { start: s, end: hi },
                    result: None,
                    ret_tag: 0,
                },
            );
            s = hi;
        }
    }
    pool_wait_idle(&e);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_thread_pool_parallel_for(
    id: i64,
    entry: usize,
    start: i64,
    end: i64,
    chunk: i64,
) {
    let e = pool_entry(id);
    if end > start && chunk > 0 {
        let mut s = start;
        while s < end {
            let hi = (s + chunk).min(end);
            pool_push_task(
                &e,
                PoolTask {
                    target: PoolTarget::Native(entry),
                    call: PoolCall::Range { start: s, end: hi },
                    result: None,
                    ret_tag: 0,
                },
            );
            s = hi;
        }
    }
    pool_wait_idle(&e);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_thread_pool_join(id: i64) {
    pool_join(id);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_thread_pool_shutdown(id: i64) {
    let old = SYNC_THREAD_POOLS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&id);
    match old {
        Some(e) => pool_stop(e),
        None => unsafe {
            let msg = format!("thread pool {id} missing");
            rnx_panic(msg.as_ptr(), msg.len());
        },
    }
}

#[unsafe(no_mangle)]
#[cfg_attr(target_arch = "wasm32", allow(unreachable_code))]
pub unsafe extern "C" fn rnx_thread_spawn(
    entry_fn: unsafe extern "C" fn(*mut u8) -> *mut u8,
    _arg: *mut u8,
) -> i64 {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (entry_fn, _arg);
        return -1;
    }
    let inner = RawEntry(entry_fn as usize);
    let pack = RawPack(Box::into_raw(Box::new((inner, 0usize))) as usize);
    let pack_addr = pack.0;
    let (id, slot) = thread_alloc();
    let handle = std::thread::Builder::new()
        .spawn(move || {
            guard::guard_thread_init();
            let out = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
                thread_main(pack_addr)
            })) {
                Ok(v) => (v as u64, None),
                Err(_) => {
                    let _ = unsafe { Box::from_raw(pack_addr as *mut (RawEntry, usize)) };
                    (0, None)
                }
            };
            thread_complete(&slot, out.0, out.1);
        })
        .ok();
    match handle {
        Some(_) => id,
        None => {
            let _ = unsafe { Box::from_raw(pack_addr as *mut (RawEntry, usize)) };
            thread_drop(id);
            -id
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_thread_join(handle: i64) -> u64 {
    let slot = match thread_take(handle) {
        Some(s) => s,
        None => return 0,
    };
    match thread_await(&slot) {
        Some((v, _)) => v,
        None => 0,
    }
}

pub(crate) static THREAD_OUTCOMES: std::sync::LazyLock<
    std::sync::Mutex<std::collections::BTreeMap<usize, Option<String>>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::BTreeMap::new()));

pub(crate) static THREAD_NEXT_ID: AtomicI64 = AtomicI64::new(1);
pub(crate) const THREAD_JOIN_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) struct NativeThreadSlot {
    pub(crate) state: Mutex<Option<(u64, Option<String>)>>,
    pub(crate) cond: Condvar,
}

pub(crate) static THREAD_SLOTS: std::sync::LazyLock<std::sync::Mutex<std::collections::BTreeMap<i64, Arc<NativeThreadSlot>>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::BTreeMap::new()));

pub(crate) fn thread_alloc() -> (i64, Arc<NativeThreadSlot>) {
    let id = THREAD_NEXT_ID.fetch_add(1, Ordering::SeqCst);
    let slot = Arc::new(NativeThreadSlot { state: Mutex::new(None), cond: Condvar::new() });
    THREAD_SLOTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(id, slot.clone());
    (id, slot)
}

pub(crate) fn thread_drop(id: i64) {
    THREAD_SLOTS.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
}

pub(crate) fn thread_complete(slot: &NativeThreadSlot, value: u64, err: Option<String>) {
    if let Ok(mut s) = slot.state.lock() {
        if s.is_none() {
            *s = Some((value, err));
        }
    }
    slot.cond.notify_all();
}

pub(crate) fn thread_take(id: i64) -> Option<Arc<NativeThreadSlot>> {
    THREAD_SLOTS.lock().unwrap_or_else(|e| e.into_inner()).remove(&id)
}

pub(crate) fn thread_await(slot: &NativeThreadSlot) -> Option<(u64, Option<String>)> {
    let mut s = slot.state.lock().unwrap_or_else(|e| e.into_inner());
    let deadline = std::time::Instant::now() + THREAD_JOIN_TIMEOUT;
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

extern "C" fn closure_thread_main(box_arg: *mut u8, ret_tag: u32) -> u64 {
    let out = unsafe { rnx_closure_invoke0(box_arg) };
    let boxed = unsafe { rnx_any_box(ret_tag, out) };
    unsafe {
        rnx_closure_release(box_arg);
    }
    boxed
}

#[unsafe(no_mangle)]
#[cfg_attr(target_arch = "wasm32", allow(unreachable_code))]
pub unsafe extern "C" fn rnx_thread_spawn_closure(box_ptr: *mut u8, ret_tag: u32) -> i64 {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (box_ptr, ret_tag);
        return -1;
    }
    if box_ptr.is_null() {
        return -1;
    }
    unsafe {
        rnx_retain(box_ptr);
    }
    let (id, slot) = thread_alloc();
    let handle = std::thread::Builder::new()
        .spawn({
            struct SendPtr(usize);
            unsafe impl Send for SendPtr {}
            struct SendTag(u32);
            unsafe impl Send for SendTag {}
            let arg = SendPtr(box_ptr as usize);
            let tag = SendTag(ret_tag);
            move || {
                guard::guard_thread_init();
                let out = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    closure_thread_main(arg.0 as *mut u8, tag.0)
                })) {
                    Ok(v) => (v, None),
                    Err(_) => (0, Some("thread panicked".to_string())),
                };
                thread_complete(&slot, out.0, out.1);
            }
        })
        .ok();
    match handle {
        Some(_) => id,
        None => {
            unsafe {
                rnx_closure_release(box_ptr);
            }
            thread_drop(id);
            -id
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_thread_join_val(handle: i64) -> u64 {
    if handle < 0 {
        THREAD_OUTCOMES
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(handle as usize, Some(spawn_failed_msg()));
        return 0;
    }
    let slot = match thread_take(handle) {
        Some(s) => s,
        None => return 0,
    };
    match thread_await(&slot) {
        Some((v, err)) => {
            THREAD_OUTCOMES
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(handle as usize, err);
            v
        }
        None => {
            THREAD_OUTCOMES
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(handle as usize, Some(format!("thread {handle} join timed out after 30s")));
            0
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_thread_join_err(handle: i64) -> *mut u8 {
    let err = THREAD_OUTCOMES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&(handle as usize))
        .flatten();
    match err {
        Some(msg) => alloc_str(&msg),
        None => alloc_str(""),
    }
}

pub(crate) struct TaskSlot {
    pub(crate) done: bool,
    pub(crate) tag: u32,
    pub(crate) payload: u64,
    pub(crate) err: Option<String>,
}

pub(crate) struct TaskBox {
    pub(crate) slot: std::sync::Mutex<TaskSlot>,
    pub(crate) cv: std::sync::Condvar,
}

pub(crate) fn task_box_new(tag: u32) -> std::sync::Arc<TaskBox> {
    std::sync::Arc::new(TaskBox {
        slot: std::sync::Mutex::new(TaskSlot { done: false, tag, payload: 0, err: None }),
        cv: std::sync::Condvar::new(),
    })
}

pub(crate) fn task_box_handle(tb: &std::sync::Arc<TaskBox>) -> *mut u8 {
    let ptr = std::sync::Arc::into_raw(tb.clone()) as *mut u8;
    LIVE_TASKS.lock().unwrap_or_else(|e| e.into_inner()).insert(ptr as usize);
    ptr
}

fn task_stall_timeout() -> std::time::Duration {
    std::env::var("RNX_TASK_STALL_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(std::time::Duration::from_millis)
        .unwrap_or(std::time::Duration::from_secs(5))
}

pub(crate) static LIVE_TASKS: std::sync::LazyLock<std::sync::Mutex<std::collections::BTreeSet<usize>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::BTreeSet::new()));

pub(crate) fn task_complete_pub(tb: &TaskBox, tag: u32, payload: u64, err: Option<String>) {
    task_complete(tb, tag, payload, err)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_task_await_val(handle: *mut u8) -> u64 {
    let tb = match task_live(handle).then(|| task_box_of(handle)).flatten() {
        Some(t) => t,
        None => return 0,
    };
    let mut s = tb.slot.lock().unwrap_or_else(|e| e.into_inner());
    let timeout = task_stall_timeout();
    while !s.done {
        s = match tb.cv.wait_timeout(s, timeout) {
            Ok((mut g, res)) => {
                if res.timed_out() && !g.done {
                    g.err = Some(format!(
                        "rnx task await stall: task {:p} (tag {}) saw no completion within {:?} (deadlocked or orphaned worker)",
                        handle, g.tag, timeout
                    ));
                    return 0;
                }
                g
            }
            Err(e) => e.into_inner().0,
        };
    }
    let (tag, payload) = (s.tag, s.payload);
    drop(s);
    unsafe {
        match any_box_addr(payload) {
            Some(_) => {
                rnx_any_retain(payload);
                payload
            }
            None => {
                let boxed = rnx_any_box(tag, payload);
                rnx_any_retain(boxed);
                boxed
            }
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_task_await_err(handle: *mut u8) -> *mut u8 {
    // The worker holds its own Arc, so reclaiming the handle ref here only
    // drops the awaiter's share; a late worker completion writes to a live
    // box instead of freed memory.
    let (msg, payload) = match task_live(handle).then(|| task_box_of(handle)).flatten() {
        Some(tb) => {
            let s = tb.slot.lock().unwrap_or_else(|e| e.into_inner());
            (s.err.clone(), s.payload)
        }
        None => (Some("dead task".to_string()), 0),
    };
    if LIVE_TASKS.lock().unwrap_or_else(|e| e.into_inner()).remove(&(handle as usize)) {
        unsafe {
            drop(std::sync::Arc::from_raw(handle as *const TaskBox));
            rnx_any_release(payload);
        }
    }
    match msg {
        Some(m) => alloc_str(&m),
        None => alloc_str(""),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_task_tag(handle: *mut u8) -> u32 {
    match task_live(handle).then(|| task_box_of(handle)).flatten() {
        Some(tb) => tb.slot.lock().unwrap_or_else(|e| e.into_inner()).tag,
        None => TAG_NULL,
    }
}

pub(crate) static NEXT_POOL_ID: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(1_000_000);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_pool_new(num_workers: i64) -> i64 {
    let id = NEXT_POOL_ID.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    unsafe {
        rnx_thread_pool_init(id, num_workers);
    }
    id
}

pub const STACK_RESERVE_BYTES: usize = 256 * 1024;

pub const STACK_EXHAUSTED_MSG: &str =
    "call stack exhausted (maximum recursion depth exceeded); rewrite deep recursion as a loop";

thread_local! {
    static STACK_FLOOR: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

pub(crate) fn current_sp() -> usize {
    let probe = 0u64;
    std::hint::black_box(&probe) as *const u64 as usize
}

#[cfg(all(target_os = "linux", not(target_arch = "wasm32")))]
pub(crate) fn query_stack_floor() -> (usize, usize) {
    unsafe {
        let mut attr: libc::pthread_attr_t = std::mem::zeroed();
        if libc::pthread_getattr_np(libc::pthread_self(), &mut attr) != 0 {
            return (0, 0);
        }
        let mut addr: *mut libc::c_void = std::ptr::null_mut();
        let mut size: usize = 0;
        let r = libc::pthread_attr_getstack(&attr, &mut addr, &mut size);
        libc::pthread_attr_destroy(&mut attr);
        if r != 0 || addr.is_null() || size == 0 {
            return (0, 0);
        }
        (addr as usize, size)
    }
}

#[cfg(any(not(target_os = "linux"), target_arch = "wasm32"))]
pub(crate) fn query_stack_floor() -> (usize, usize) {
    (0, 0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rnx_stack_check() -> i64 {
    let sp = current_sp();
    let mut floor = STACK_FLOOR.with(|c| c.get());
    if floor == 0 || sp < floor {
        (floor, _) = query_stack_floor();
        if floor == 0 {
            return i64::MAX;
        }
        STACK_FLOOR.with(|c| c.set(floor));
    }
    sp.saturating_sub(floor).min(i64::MAX as usize) as i64
}

#[cfg(test)]
mod thread_tests {
    use super::*;

    unsafe extern "C" fn worker(_arg: *mut u8) -> *mut u8 {
        42 as *mut u8
    }

    #[test]
    fn spawn_join_roundtrip() {
        unsafe {
            let h = rnx_thread_spawn(worker, std::ptr::null_mut());
            assert!(h > 0);
            assert_eq!(rnx_thread_join(h), 42);
            assert_eq!(rnx_thread_join(-1), 0);
        }
    }
}

#[cfg(test)]
mod sync_tests {
    use super::*;

    #[test]
    fn atomic_ops() {
        unsafe {
            rnx_sync_atomic_set(9001, 0);
            assert_eq!(rnx_sync_atomic_get(9001), 0);
            assert_eq!(rnx_sync_atomic_fetch_add(9001, 10), 0);
            assert_eq!(rnx_sync_atomic_get(9001), 10);
            assert!(rnx_sync_atomic_cas(9001, 10, 20));
            assert!(!rnx_sync_atomic_cas(9001, 10, 30));
            assert_eq!(rnx_sync_atomic_get(9001), 20);
        }
    }

    #[test]
    fn channel_send_recv_try_len() {
        unsafe {
            assert_eq!(rnx_sync_channel_try_recv(9002), -1);
            assert_eq!(rnx_sync_channel_len(9002), 0);
            rnx_sync_channel_send(9002, 7);
            rnx_sync_channel_send(9002, 8);
            assert_eq!(rnx_sync_channel_len(9002), 2);
            assert_eq!(rnx_sync_channel_try_recv(9002), 7);
            assert_eq!(rnx_sync_channel_recv(9002), 8);
            assert_eq!(rnx_sync_channel_try_recv(9002), -1);
        }
    }

    #[test]
    fn channel_blocks_until_send() {
        unsafe {
            let h = std::thread::spawn(|| rnx_sync_channel_recv(9003));
            std::thread::sleep(std::time::Duration::from_millis(50));
            rnx_sync_channel_send(9003, 123);
            assert_eq!(h.join().unwrap(), 123);
        }
    }

    #[test]
    fn mutex_lock_unlock_balanced() {
        unsafe {
            rnx_mutex_lock(9100);
            rnx_mutex_unlock(9100);
            assert!(rnx_mutex_try_lock(9100));
            rnx_mutex_unlock(9100);
        }
    }

    #[test]
    fn mutex_try_lock_fails_while_held() {
        unsafe {
            rnx_mutex_lock(9101);
            let seen = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
            let probe = seen.clone();
            let h = std::thread::spawn(move || {
                probe.store(rnx_mutex_try_lock(9101), std::sync::atomic::Ordering::SeqCst);
            });
            h.join().unwrap();
            assert!(!seen.load(std::sync::atomic::Ordering::SeqCst));
            rnx_mutex_unlock(9101);
            assert!(rnx_mutex_try_lock(9101));
            rnx_mutex_unlock(9101);
        }
    }

    #[test]
    fn mutex_serializes_increments() {
        unsafe {
            rnx_sync_atomic_set(9102, 0);
            let hs: Vec<_> = (0..4)
                .map(|_| {
                    std::thread::spawn(|| {
                        for _ in 0..500 {
                            rnx_mutex_lock(9102);
                            let v = rnx_sync_atomic_get(9102);
                            rnx_sync_atomic_set(9102, v + 1);
                            rnx_mutex_unlock(9102);
                        }
                    })
                })
                .collect();
            for h in hs {
                h.join().unwrap();
            }
            assert_eq!(rnx_sync_atomic_get(9102), 2000);
        }
    }

    #[test]
    fn rwlock_readers_share_writer_excludes() {
        unsafe {
            rnx_rwlock_read_lock(9103);
            assert!(!rnx_rwlock_try_write_lock(9103));
            assert!(rnx_rwlock_try_read_lock(9103));
            rnx_rwlock_read_unlock(9103);
            rnx_rwlock_read_unlock(9103);
            assert!(rnx_rwlock_try_write_lock(9103));
            assert!(!rnx_rwlock_try_read_lock(9103));
            assert!(!rnx_rwlock_try_write_lock(9103));
            rnx_rwlock_write_unlock(9103);
            assert!(rnx_rwlock_try_read_lock(9103));
            rnx_rwlock_read_unlock(9103);
        }
    }

    #[test]
    fn rwlock_writer_blocks_until_readers_release() {        unsafe {
            rnx_rwlock_read_lock(9104);
            rnx_rwlock_read_lock(9104);
            let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let flag = done.clone();
            let h = std::thread::spawn(move || {
                rnx_rwlock_write_lock(9104);
                flag.store(true, std::sync::atomic::Ordering::SeqCst);
                rnx_rwlock_write_unlock(9104);
            });
            std::thread::sleep(std::time::Duration::from_millis(50));
            assert!(!done.load(std::sync::atomic::Ordering::SeqCst));
            rnx_rwlock_read_unlock(9104);
            std::thread::sleep(std::time::Duration::from_millis(50));
            assert!(!done.load(std::sync::atomic::Ordering::SeqCst));
            rnx_rwlock_read_unlock(9104);
            h.join().unwrap();
            assert!(done.load(std::sync::atomic::Ordering::SeqCst));
        }
    }

    #[test]
    fn condvar_wait_wakes_on_notify() {
        unsafe {
            rnx_mutex_lock(9200);
            let fired = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let flag = fired.clone();
            let h = std::thread::spawn(move || {
                rnx_mutex_lock(9200);
                rnx_condvar_notify_one(9200);
                rnx_mutex_unlock(9200);
                flag.store(true, std::sync::atomic::Ordering::SeqCst);
            });
            rnx_condvar_wait(9200, 9200);
            rnx_mutex_unlock(9200);
            h.join().unwrap();
            assert!(fired.load(std::sync::atomic::Ordering::SeqCst));
        }
    }

    #[test]
    fn condvar_wait_timeout_false_when_silent() {
        unsafe {
            rnx_mutex_lock(9201);
            let t0 = std::time::Instant::now();
            assert!(!rnx_condvar_wait_timeout(9201, 9201, 20));
            assert!(t0.elapsed() < std::time::Duration::from_secs(5));
            rnx_mutex_unlock(9201);
        }
    }

    #[test]
    fn barrier_rendezvous_single_leader() {
        unsafe {
            let leaders = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
            let followers = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
            let hs: Vec<_> = (0..4)
                .map(|_| {
                    let l = leaders.clone();
                    let f = followers.clone();
                    std::thread::spawn(move || {
                        if rnx_barrier_wait(9202, 4) {
                            l.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        } else {
                            f.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        }
                    })
                })
                .collect();
            for h in hs {
                h.join().unwrap();
            }
            assert_eq!(leaders.load(std::sync::atomic::Ordering::SeqCst), 1);
            assert_eq!(followers.load(std::sync::atomic::Ordering::SeqCst), 3);
        }
    }

    #[test]
    fn pool_runs_submit_tasks_to_completion() {
        static COUNT: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);
        unsafe extern "C" fn inc(_: i64) -> i64 {
            COUNT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            0
        }
        unsafe {
            COUNT.store(0, std::sync::atomic::Ordering::SeqCst);
            rnx_thread_pool_init(9300, 4);
            for _ in 0..10 {
                rnx_thread_pool_submit(9300, inc as *const () as usize, 0, 0);
            }
            rnx_thread_pool_join(9300);
            assert_eq!(COUNT.load(std::sync::atomic::Ordering::SeqCst), 10);
            rnx_thread_pool_shutdown(9300);
        }
    }

    #[test]
    fn pool_parallel_for_covers_every_index() {
        static SUM: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);
        unsafe extern "C" fn add(i: i64) -> i64 {
            SUM.fetch_add(i, std::sync::atomic::Ordering::SeqCst);
            0
        }
        unsafe {
            SUM.store(0, std::sync::atomic::Ordering::SeqCst);
            rnx_thread_pool_init(9301, 4);
            rnx_thread_pool_parallel_for(9301, add as *const () as usize, 0, 100, 7);
            assert_eq!(SUM.load(std::sync::atomic::Ordering::SeqCst), 4950);
            rnx_thread_pool_shutdown(9301);
        }
    }

    #[test]
    fn pool_id_reusable_after_shutdown() {
        static COUNT: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);
        unsafe extern "C" fn inc(_: i64) -> i64 {
            COUNT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            0
        }
        unsafe {
            COUNT.store(0, std::sync::atomic::Ordering::SeqCst);
            for _ in 0..2 {
                rnx_thread_pool_init(9302, 2);
                for _ in 0..5 {
                    rnx_thread_pool_submit(9302, inc as *const () as usize, 0, 1);
                }
                rnx_thread_pool_join(9302);
                rnx_thread_pool_shutdown(9302);
            }
            assert_eq!(COUNT.load(std::sync::atomic::Ordering::SeqCst), 10);
        }
    }

    #[test]
    fn pool_init_same_size_keeps_queued_tasks() {
        static COUNT: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);
        unsafe extern "C" fn slow_inc(_: i64) -> i64 {
            std::thread::sleep(std::time::Duration::from_millis(50));
            COUNT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            0
        }
        unsafe {
            COUNT.store(0, std::sync::atomic::Ordering::SeqCst);
            rnx_thread_pool_init(9310, 2);
            for _ in 0..6 {
                rnx_thread_pool_submit(9310, slow_inc as *const () as usize, 0, 1);
            }
            rnx_thread_pool_init(9310, 2);
            rnx_thread_pool_join(9310);
            assert_eq!(COUNT.load(std::sync::atomic::Ordering::SeqCst), 6);
            rnx_thread_pool_shutdown(9310);
        }
    }

    #[test]
    fn pool_task_panic_completes_with_error() {
        unsafe extern "C-unwind" fn boom(_: i64) -> u64 {
            panic!("boom");
        }
        unsafe {
            rnx_thread_pool_init(9320, 2);
            let h = rnx_thread_pool_submit_handle(9320, boom as *const () as usize, 0, 0, 0);
            assert!(!h.is_null());
            let _ = rnx_task_await_val(h);
            let e = rnx_task_await_err(h);
            assert!(!str_bytes(e).is_empty());
            rnx_thread_pool_join(9320);
            rnx_thread_pool_shutdown(9320);
        }
    }

    #[test]
    fn task_await_stall_then_late_completion_no_use_after_free() {
        unsafe extern "C-unwind" fn slow(_: i64) -> u64 {
            std::thread::sleep(std::time::Duration::from_millis(300));
            0
        }
        unsafe {
            std::env::set_var("RNX_TASK_STALL_MS", "20");
            rnx_thread_pool_init(9331, 2);
            for _ in 0..5 {
                let h = rnx_thread_pool_submit_handle(9331, slow as *const () as usize, 0, 0, 0);
                assert!(!h.is_null());
                let _ = rnx_task_await_val(h);
                let e = rnx_task_await_err(h);
                assert!(!str_bytes(e).is_empty());
            }
            rnx_thread_pool_join(9331);
            rnx_thread_pool_shutdown(9331);
            std::env::remove_var("RNX_TASK_STALL_MS");
        }
    }

    #[test]
    fn interp_runner_lock_released_during_task() {
        fn probe(_: PoolInterpCall) {
            assert!(
                POOL_INTERP_RUNNER.try_lock().is_ok(),
                "interp runner must not hold the runner lock while running"
            );
        }
        pool_set_interp_runner(probe);
        exec_pool_task(PoolTask {
            target: PoolTarget::Interp { module: 0, func: 0, heaps: 0 },
            call: PoolCall::Once { arg: 0, has_arg: false },
            result: None,
            ret_tag: 0,
        });
        *POOL_INTERP_RUNNER.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    #[test]
    fn pool_init_new_size_resets() {
        static COUNT: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);
        unsafe extern "C" fn inc(_: i64) -> i64 {
            COUNT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            0
        }
        unsafe {
            COUNT.store(0, std::sync::atomic::Ordering::SeqCst);
            rnx_thread_pool_init(9311, 2);
            for _ in 0..4 {
                rnx_thread_pool_submit(9311, inc as *const () as usize, 0, 1);
            }
            rnx_thread_pool_join(9311);
            assert_eq!(COUNT.load(std::sync::atomic::Ordering::SeqCst), 4);
            rnx_thread_pool_init(9311, 3);
            for _ in 0..4 {
                rnx_thread_pool_submit(9311, inc as *const () as usize, 0, 1);
            }
            rnx_thread_pool_join(9311);
            assert_eq!(COUNT.load(std::sync::atomic::Ordering::SeqCst), 8);
            rnx_thread_pool_shutdown(9311);
        }
    }

    #[test]
    fn atomics_race_cleanly() {
        unsafe {
            rnx_sync_atomic_set(9004, 0);
            let hs: Vec<_> = (0..8).map(|_| std::thread::spawn(|| {
                for _ in 0..500 {
                    rnx_sync_atomic_fetch_add(9004, 1);
                }
            })).collect();
            for h in hs {
                h.join().unwrap();
            }
            assert_eq!(rnx_sync_atomic_get(9004), 4000);
        }
    }

    macro_rules! arc_counter {
        ($hits:ident, $dtor:ident) => {
            static $hits: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            unsafe extern "C" fn $dtor(_ptr: *mut u8) {
                $hits.fetch_add(1, Ordering::SeqCst);
            }
        };
    }

    arc_counter!(ARC_HITS_RACE, arc_dtor_race);
    arc_counter!(ARC_HITS_HANDOFF, arc_dtor_handoff);
    arc_counter!(ARC_HITS_DROP, arc_dtor_drop);

    fn live_obj() -> *mut u8 {
        unsafe {
            let p = rnx_alloc(24, 8);
            assert!(!p.is_null());
            (p as *mut u32).write(1);
            p
        }
    }

    #[test]
    fn arc_retain_release_race_cleanly() {
        unsafe {
            ARC_HITS_RACE.store(0, Ordering::SeqCst);
            let p = live_obj();
            let addr = p as usize;
            let hs: Vec<_> = (0..8)
                .map(|_| {
                    std::thread::spawn(move || {
                        let q = addr as *mut u8;
                        for _ in 0..500 {
                            rnx_retain(q);
                            rnx_release(q, 24, Some(arc_dtor_race));
                        }
                    })
                })
                .collect();
            for h in hs {
                h.join().unwrap();
            }
            assert_eq!(ARC_HITS_RACE.load(Ordering::SeqCst), 0);
            rnx_release(p, 24, Some(arc_dtor_race));
            assert_eq!(ARC_HITS_RACE.load(Ordering::SeqCst), 1);
        }
    }

    #[test]
    fn channel_object_handoff_transfers_ref() {
        unsafe {
            ARC_HITS_HANDOFF.store(0, Ordering::SeqCst);
            let dtor = arc_dtor_handoff as *const () as usize;
            let p = live_obj();
            rnx_sync_channel_send_obj(9101, p, 24, dtor);
            assert_eq!(rnx_sync_channel_len(9101), 1);
            let got = rnx_sync_channel_recv(9101);
            assert_eq!(got, p as i64);
            assert_eq!(ARC_HITS_HANDOFF.load(Ordering::SeqCst), 0);
            rnx_release(p, 24, Some(arc_dtor_handoff));
            assert_eq!(ARC_HITS_HANDOFF.load(Ordering::SeqCst), 0);
            rnx_release(got as *mut u8, 24, Some(arc_dtor_handoff));
            assert_eq!(ARC_HITS_HANDOFF.load(Ordering::SeqCst), 1);
        }
    }

    #[test]
    fn channel_drop_releases_queued_objects() {
        unsafe {
            ARC_HITS_DROP.store(0, Ordering::SeqCst);
            let dtor = arc_dtor_drop as *const () as usize;
            let mut ptrs = Vec::new();
            for _ in 0..5 {
                let p = live_obj();
                rnx_sync_channel_send_obj(9102, p, 24, dtor);
                ptrs.push(p);
            }
            assert_eq!(rnx_sync_channel_len(9102), 5);
            rnx_sync_channel_drop(9102);
            assert_eq!(rnx_sync_channel_len(9102), 0);
            assert_eq!(ARC_HITS_DROP.load(Ordering::SeqCst), 0);
            for p in ptrs {
                rnx_release(p, 24, Some(arc_dtor_drop));
            }
            assert_eq!(ARC_HITS_DROP.load(Ordering::SeqCst), 5);
        }
    }

    #[test]
    fn channel_str_and_array_roundtrip() {
        unsafe {
            let s = alloc_str("hi");
            rnx_sync_channel_send_str(9103, s);
            let got = rnx_sync_channel_recv(9103);
            let ptr = rnx_any_unbox(got as u64) as *const u8;
            assert!(rnx_string_eq(ptr, s));
            rnx_any_release(got as u64);
            rnx_release_str(s);
            let a = rnx_array_new(0, 8);
            rnx_array_push(a, 11, 8);
            rnx_sync_channel_send_array(9103, a, 8, 0);
            let back = rnx_sync_channel_recv(9103);
            assert_eq!(back, a as i64);
            assert_eq!(rnx_array_get(back as *const u8, 0, 8), 11);
            rnx_release_array(a, 8, None);
            rnx_release_array(back as *mut u8, 8, None);
        }
    }
}
