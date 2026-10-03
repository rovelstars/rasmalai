use crate::value::{Arena, ArrayTable, GenericMapTable, MapTable, Value};
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Condvar, LazyLock, Mutex};

pub struct Heaps {
    pub objs: Arena,
    pub arrs: ArrayTable,
    pub maps: MapTable,
    pub gmaps: GenericMapTable,
}

impl Heaps {
    pub fn share(&self) -> Heaps {
        Heaps {
            objs: self.objs.share(),
            arrs: self.arrs.share(),
            maps: self.maps.share(),
            gmaps: self.gmaps.share(),
        }
    }

    pub fn same(&self, other: &Heaps) -> bool {
        self.objs.same(&other.objs) && self.arrs.same(&other.arrs)
    }

    pub fn retain_value(&mut self, v: &Value) {
        match v {
            Value::Obj { slot, .. } => self.objs.retain(*slot),
            Value::Array { id } => self.arrs.retain(*id),
            Value::Enum { payload, .. } => {
                for p in payload {
                    self.retain_value(p);
                }
            }
            _ => {}
        }
    }

    pub fn release_shallow(&mut self, v: &Value) {
        match v {
            Value::Obj { slot, .. } => {
                self.objs.release(*slot);
            }
            Value::Array { id } => {
                self.arrs.release(*id);
            }
            Value::Enum { payload, .. } => {
                for p in payload {
                    self.release_shallow(p);
                }
            }
            _ => {}
        }
    }
}

struct Entry {
    value: Value,
    heaps: Heaps,
}

struct IChannel {
    queue: Mutex<VecDeque<Entry>>,
    cv: Condvar,
}

static CHANNELS: LazyLock<Mutex<BTreeMap<i64, Arc<IChannel>>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));

fn channel(id: i64) -> Arc<IChannel> {
    let mut table = CHANNELS.lock().unwrap_or_else(|e| e.into_inner());
    table
        .entry(id)
        .or_insert_with(|| {
            Arc::new(IChannel { queue: Mutex::new(VecDeque::new()), cv: Condvar::new() })
        })
        .clone()
}

pub fn send(id: i64, value: Value, heaps: Heaps) {
    let ch = channel(id);
    ch.queue.lock().unwrap_or_else(|e| e.into_inner()).push_back(Entry { value, heaps });
    ch.cv.notify_one();
}

fn reclaim_stale(q: &mut VecDeque<Entry>, heaps: &Heaps) {
    if q.iter().all(|e| e.heaps.same(heaps)) {
        return;
    }
    let entries: Vec<Entry> = q.drain(..).collect();
    let mut owned = heaps.share();
    for e in entries {
        if e.heaps.same(heaps) {
            q.push_back(e);
        } else {
            owned.release_shallow(&e.value);
        }
    }
}

pub fn recv(id: i64, heaps: &Heaps) -> Value {
    let ch = channel(id);
    let mut q = ch.queue.lock().unwrap_or_else(|e| e.into_inner());
    loop {
        if let Some(pos) = q.iter().position(|e| e.heaps.same(heaps)) {
            return q.remove(pos).expect("channel position").value;
        }
        let stale: Vec<Entry> = q.drain(..).collect();
        if stale.is_empty() {
            q = ch.cv.wait(q).unwrap_or_else(|e| e.into_inner());
        } else {
            drop(q);
            let mut owned = heaps.share();
            for e in stale {
                owned.release_shallow(&e.value);
            }
            q = ch.queue.lock().unwrap_or_else(|e| e.into_inner());
        }
    }
}

pub fn try_recv(id: i64, heaps: &Heaps) -> Option<Value> {
    let ch = channel(id);
    let mut q = ch.queue.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(pos) = q.iter().position(|e| e.heaps.same(heaps)) {
        return q.remove(pos).map(|e| e.value);
    }
    reclaim_stale(&mut q, heaps);
    None
}

pub fn drain(id: i64, heaps: &Heaps, mut adopt: impl FnMut(Value)) {
    let ch = channel(id);
    let mut q = ch.queue.lock().unwrap_or_else(|e| e.into_inner());
    let entries: Vec<Entry> = q.drain(..).collect();
    drop(q);
    let mut owned = heaps.share();
    for e in entries {
        if e.heaps.same(heaps) {
            adopt(e.value);
        } else {
            owned.release_shallow(&e.value);
        }
    }
}

pub fn len(id: i64) -> i64 {
    let ch = channel(id);
    ch.queue.lock().unwrap_or_else(|e| e.into_inner()).len() as i64
}
