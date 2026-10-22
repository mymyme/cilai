//! 保存前台已空闲或卸载线程的运行时。原生记忆可继续运行，回收不等待其成功完成。
use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

const IDLE_TIMEOUT: Duration = Duration::from_mins(2);
const MAX_IDLE: usize = 2;

pub(crate) struct IdlePool<T> {
    state: Mutex<State<T>>,
}

struct State<T> {
    epoch: u64,
    serial: u64,
    entries: HashMap<String, (u64, T)>,
}

pub(crate) struct Lease<T> {
    pool: Weak<IdlePool<T>>,
    epoch: u64,
    serial: u64,
    key: String,
}

impl<T> Default for IdlePool<T> {
    fn default() -> Self {
        Self {
            state: Mutex::new(State {
                epoch: 0,
                serial: 0,
                entries: HashMap::new(),
            }),
        }
    }
}

impl<T: Send + 'static> IdlePool<T> {
    pub(crate) fn take(self: &Arc<Self>, key: String) -> (Lease<T>, Option<T>) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let value = state.entries.remove(&key).map(|(_, value)| value);
        state.serial = state.serial.wrapping_add(1);
        (
            Lease {
                pool: Arc::downgrade(self),
                epoch: state.epoch,
                serial: state.serial,
                key,
            },
            value,
        )
    }

    pub(crate) fn clear(&self) {
        drop(self.drain());
    }

    pub(crate) fn drain(&self) -> Vec<T> {
        let removed = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.epoch = state.epoch.wrapping_add(1);
            std::mem::take(&mut state.entries)
        };
        removed.into_values().map(|(_, value)| value).collect()
    }

    fn expire(&self, key: &str, serial: u64) {
        let removed = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state
                .entries
                .get(key)
                .is_some_and(|(current, _)| *current == serial)
            {
                state.entries.remove(key)
            } else {
                None
            }
        };
        drop(removed);
    }
}

impl<T: Send + 'static> Lease<T> {
    fn insert(&self, value: T) -> Option<u64> {
        let pool = self.pool.upgrade()?;
        let mut retired = Vec::new();
        let serial = {
            let mut state = pool
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.epoch != self.epoch {
                return None;
            }
            // 较早轮次的后台记忆可能更晚结束，不能替换较新轮次已经保留的原生线程。
            if state
                .entries
                .get(&self.key)
                .is_some_and(|(serial, _)| *serial > self.serial)
            {
                return None;
            }
            if let Some(old) = state.entries.remove(&self.key) {
                retired.push(old);
            }
            while state.entries.len() >= MAX_IDLE {
                let oldest = state
                    .entries
                    .iter()
                    .min_by_key(|(_, (serial, _))| serial)
                    .map(|(key, _)| key.clone());
                if let Some(oldest) = oldest.and_then(|key| state.entries.remove(&key)) {
                    retired.push(oldest);
                }
            }
            let serial = self.serial;
            state.entries.insert(self.key.clone(), (serial, value));
            serial
        };
        drop(retired);
        Some(serial)
    }

    pub(crate) fn recycle(self, value: T) {
        let Some(serial) = self.insert(value) else {
            return;
        };
        let pool = self.pool.clone();
        let key = self.key.clone();
        if let Err(error) = std::thread::Builder::new()
            .name("harness-idle-expiry".into())
            .spawn(move || {
                std::thread::sleep(IDLE_TIMEOUT);
                if let Some(pool) = pool.upgrade() {
                    pool.expire(&key, serial);
                }
            })
        {
            eprintln!("harness_idle_expiry_failed:{error}");
            if let Some(pool) = self.pool.upgrade() {
                pool.expire(&self.key, serial);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn late_memory_completion_cannot_replace_a_newer_idle_thread() {
        let pool = Arc::new(IdlePool::default());
        let (older, _) = pool.take("conversation".into());
        let (newer, _) = pool.take("conversation".into());
        newer.insert("new-thread").unwrap();
        assert!(older.insert("old-memory-runtime").is_none());
        assert_eq!(pool.take("conversation".into()).1, Some("new-thread"));
    }

    #[test]
    fn lease_is_exclusive_and_old_expiry_cannot_remove_reused_entry() {
        let pool = Arc::new(IdlePool::default());
        let (lease, value) = pool.take("conversation".into());
        assert!(value.is_none());
        let old_serial = lease.insert(1).unwrap();
        let (lease, value) = pool.take("conversation".into());
        assert_eq!(value, Some(1));
        assert!(pool.take("conversation".into()).1.is_none());
        let serial = lease.insert(2).unwrap();
        pool.expire("conversation", old_serial);
        assert_eq!(pool.state.lock().unwrap().entries["conversation"].1, 2);
        pool.expire("conversation", serial);
        assert!(pool.take("conversation".into()).1.is_none());
    }
    #[test]
    fn clearing_invalidates_in_flight_leases_and_bounds_idle_entries() {
        let pool = Arc::new(IdlePool::default());
        let (late, _) = pool.take("late".into());
        pool.clear();
        assert_eq!(late.insert(1), None);
        for key in ["a", "b", "c"] {
            pool.take(key.into()).0.insert(1);
        }
        assert!(pool.take("a".into()).1.is_none());
        assert_eq!(pool.take("b".into()).1, Some(1));
        assert_eq!(pool.take("c".into()).1, Some(1));
    }
}
