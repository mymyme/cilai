use std::collections::{BTreeSet, HashSet, VecDeque};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread::JoinHandle;

type Operation = Box<dyn FnOnce() + Send + 'static>;

struct Task {
    sequence: u64,
    key: Option<String>,
    operation: Operation,
}

#[derive(Default)]
struct State {
    pending: VecDeque<Task>,
    active_keys: HashSet<String>,
    active: usize,
    closed: bool,
    next_sequence: u64,
    unfinished: BTreeSet<u64>,
}

#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}

/// 固定工作线程与总受理容量；同键任务按受理顺序执行，其他键可以越过正在等待的键。
pub(crate) struct CommandExecutor {
    shared: Arc<Shared>,
    workers: Vec<JoinHandle<()>>,
    capacity: usize,
}

pub(crate) struct CompletionFence {
    shared: Arc<Shared>,
    through: u64,
}
impl CompletionFence {
    pub(crate) fn wait(&self) {
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        while state
            .unfinished
            .first()
            .is_some_and(|sequence| *sequence <= self.through)
        {
            state = self
                .shared
                .changed
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }
}

impl CommandExecutor {
    pub(crate) fn new(name: &str, workers: usize, capacity: usize) -> std::io::Result<Self> {
        assert!(workers > 0 && capacity >= workers);
        let mut executor = Self {
            shared: Arc::new(Shared::default()),
            workers: Vec::with_capacity(workers),
            capacity,
        };
        for index in 0..workers {
            let shared = Arc::clone(&executor.shared);
            let worker = std::thread::Builder::new()
                .name(format!("{name}-{index}"))
                .spawn(move || work(&shared))?;
            executor.workers.push(worker);
        }
        Ok(executor)
    }

    pub(crate) fn submit(
        &self,
        key: Option<String>,
        operation: impl FnOnce() + Send + 'static,
    ) -> Result<(), &'static str> {
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if state.closed {
            return Err("host_command_executor_closed");
        }
        if state.active + state.pending.len() >= self.capacity {
            return Err("host_command_capacity_exceeded");
        }
        let sequence = state
            .next_sequence
            .checked_add(1)
            .ok_or("host_command_sequence_exhausted")?;
        state.next_sequence = sequence;
        state.unfinished.insert(sequence);
        state.pending.push_back(Task {
            sequence,
            key,
            operation: Box::new(operation),
        });
        self.shared.changed.notify_all();
        Ok(())
    }

    pub(crate) fn is_idle(&self) -> bool {
        let state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        state.active == 0 && state.pending.is_empty()
    }

    pub(crate) fn wait_idle(&self) {
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        while state.active > 0 || !state.pending.is_empty() {
            state = self
                .shared
                .changed
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    pub(crate) fn completion_fence(&self) -> CompletionFence {
        let state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        CompletionFence {
            shared: self.shared.clone(),
            through: state.next_sequence,
        }
    }

    pub(crate) fn shutdown(&mut self) {
        self.shared
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .closed = true;
        self.shared.changed.notify_all();
        for worker in self.workers.drain(..) {
            if worker.join().is_err() {
                eprintln!("宿主命令工作线程退出异常");
            }
        }
    }
}

impl Drop for CommandExecutor {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn work(shared: &Shared) {
    loop {
        let task = {
            let mut state = shared.state.lock().unwrap_or_else(PoisonError::into_inner);
            loop {
                let eligible = state.pending.iter().position(|task| {
                    task.key
                        .as_ref()
                        .is_none_or(|key| !state.active_keys.contains(key))
                });
                if let Some(index) = eligible {
                    let task = state
                        .pending
                        .remove(index)
                        .expect("selected task is present");
                    if let Some(key) = &task.key {
                        state.active_keys.insert(key.clone());
                    }
                    state.active += 1;
                    break task;
                }
                if state.closed && state.pending.is_empty() {
                    return;
                }
                state = shared
                    .changed
                    .wait(state)
                    .unwrap_or_else(PoisonError::into_inner);
            }
        };
        let Task {
            sequence,
            key,
            operation,
        } = task;
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(operation)).is_err() {
            eprintln!("宿主命令工作任务异常结束");
        }
        let mut state = shared.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.active -= 1;
        state.unfinished.remove(&sequence);
        if let Some(key) = key {
            state.active_keys.remove(&key);
        }
        shared.changed.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::CommandExecutor;
    use std::sync::{Arc, mpsc};
    use std::time::Duration;

    const DEADLINE: Duration = Duration::from_secs(2);

    #[test]
    fn remote_waiter_preserves_file_barrier_while_other_admission_continues() {
        let files = CommandExecutor::new("file-barrier", 1, 2).unwrap();
        let remote = CommandExecutor::new("remote-barrier", 1, 2).unwrap();
        let snapshots = CommandExecutor::new("snapshot-barrier", 1, 2).unwrap();
        let (entered, observed) = mpsc::channel();
        let (release, held) = mpsc::channel();
        files
            .submit(None, move || {
                entered.send(()).unwrap();
                held.recv().unwrap();
            })
            .unwrap();
        observed.recv_timeout(DEADLINE).unwrap();
        let waiter = files.completion_fence();
        let (completed, result) = mpsc::channel();
        remote
            .submit(None, move || {
                waiter.wait();
                completed.send(()).unwrap();
            })
            .unwrap();
        let (ready, status) = mpsc::channel();
        snapshots
            .submit(None, move || ready.send(()).unwrap())
            .unwrap();
        status.recv_timeout(DEADLINE).unwrap();
        assert!(result.try_recv().is_err());
        release.send(()).unwrap();
        result.recv_timeout(DEADLINE).unwrap();
    }

    #[test]
    fn file_fence_waits_for_prior_work_and_excludes_later_admission() {
        let files = CommandExecutor::new("file-fence", 2, 4).unwrap();
        let (prior_started, prior_observed) = mpsc::channel();
        let (release_prior, held_prior) = mpsc::channel();
        files
            .submit(None, move || {
                prior_started.send(()).unwrap();
                held_prior.recv().unwrap();
            })
            .unwrap();
        prior_observed.recv_timeout(DEADLINE).unwrap();
        let fence = files.completion_fence();
        let (later_started, later_observed) = mpsc::channel();
        let (release_later, held_later) = mpsc::channel();
        files
            .submit(None, move || {
                later_started.send(()).unwrap();
                held_later.recv().unwrap();
            })
            .unwrap();
        later_observed.recv_timeout(DEADLINE).unwrap();
        let (completed, observed) = mpsc::channel();
        let waiter = std::thread::spawn(move || {
            fence.wait();
            completed.send(()).unwrap();
        });
        let pending = observed.try_recv().is_err();
        release_prior.send(()).unwrap();
        let completed_before_later = observed.recv_timeout(DEADLINE).is_ok();
        let remaining = files.shared.state.lock().unwrap().unfinished.len();
        release_later.send(()).unwrap();
        files.wait_idle();
        waiter.join().unwrap();
        assert!(pending);
        assert!(completed_before_later);
        assert_eq!(remaining, 1);
        assert!(files.shared.state.lock().unwrap().unfinished.is_empty());
    }

    #[test]
    fn blocked_background_work_does_not_hold_interactive_files() {
        let background = CommandExecutor::new("slow-test", 1, 2).unwrap();
        let files = CommandExecutor::new("file-test", 1, 2).unwrap();
        let (started, observed) = mpsc::channel();
        let (release, held) = mpsc::channel();
        background
            .submit(None, move || {
                started.send(()).unwrap();
                held.recv().unwrap();
            })
            .unwrap();
        observed.recv_timeout(DEADLINE).unwrap();
        let (ready, read) = mpsc::channel();
        files
            .submit(None, move || ready.send("file").unwrap())
            .unwrap();
        assert_eq!(read.recv_timeout(DEADLINE).unwrap(), "file");
        release.send(()).unwrap();
    }

    #[test]
    fn same_application_keeps_fifo_and_other_applications_can_progress() {
        let executor = CommandExecutor::new("application-test", 2, 4).unwrap();
        let (started, observed) = mpsc::channel();
        let (release, held) = mpsc::channel();
        executor
            .submit(Some("app-a".into()), move || {
                started.send(()).unwrap();
                held.recv().unwrap();
            })
            .unwrap();
        observed.recv_timeout(DEADLINE).unwrap();
        let (completed, results) = mpsc::channel();
        let next = completed.clone();
        executor
            .submit(Some("app-a".into()), move || next.send("a-2").unwrap())
            .unwrap();
        let next = completed.clone();
        executor
            .submit(Some("app-a".into()), move || next.send("a-3").unwrap())
            .unwrap();
        executor
            .submit(Some("app-b".into()), move || completed.send("b").unwrap())
            .unwrap();
        assert_eq!(results.recv_timeout(DEADLINE).unwrap(), "b");
        assert!(results.try_recv().is_err());
        release.send(()).unwrap();
        assert_eq!(results.recv_timeout(DEADLINE).unwrap(), "a-2");
        assert_eq!(results.recv_timeout(DEADLINE).unwrap(), "a-3");
    }

    #[test]
    fn capacity_counts_running_and_queued_work_and_rejects_without_blocking() {
        struct OnDrop(mpsc::Sender<()>);
        impl Drop for OnDrop {
            fn drop(&mut self) {
                self.0.send(()).unwrap();
            }
        }
        let executor = CommandExecutor::new("capacity-test", 1, 2).unwrap();
        let (started, observed) = mpsc::channel();
        let (release, held) = mpsc::channel();
        executor
            .submit(None, move || {
                started.send(()).unwrap();
                held.recv().unwrap();
            })
            .unwrap();
        observed.recv_timeout(DEADLINE).unwrap();
        executor.submit(None, || {}).unwrap();
        let (dropped, observed_drop) = mpsc::channel();
        let rejected = OnDrop(dropped);
        assert_eq!(
            executor.submit(None, move || drop(rejected)),
            Err("host_command_capacity_exceeded")
        );
        observed_drop.recv_timeout(DEADLINE).unwrap();
        release.send(()).unwrap();
        executor.wait_idle();
        executor.submit(None, || {}).unwrap();
    }

    #[test]
    fn context_barrier_waits_for_queued_and_running_work() {
        let executor = Arc::new(CommandExecutor::new("barrier-test", 1, 3).unwrap());
        let (started, observed) = mpsc::channel();
        let (release, held) = mpsc::channel();
        executor
            .submit(None, move || {
                started.send(()).unwrap();
                held.recv().unwrap();
            })
            .unwrap();
        observed.recv_timeout(DEADLINE).unwrap();
        let (queued, done) = mpsc::channel();
        executor
            .submit(None, move || queued.send(()).unwrap())
            .unwrap();
        let waiter = Arc::clone(&executor);
        let (idle, barrier) = mpsc::channel();
        let waiting = std::thread::spawn(move || {
            waiter.wait_idle();
            idle.send(()).unwrap();
        });
        assert!(barrier.try_recv().is_err());
        release.send(()).unwrap();
        barrier.recv_timeout(DEADLINE).unwrap();
        done.recv_timeout(DEADLINE).unwrap();
        waiting.join().unwrap();
    }

    #[test]
    fn shutdown_drains_accepted_work_and_panic_does_not_strand_a_key() {
        let mut executor = CommandExecutor::new("shutdown-test", 1, 3).unwrap();
        let (finished, observed) = mpsc::channel();
        executor
            .submit(Some("document".into()), || panic!("request failure"))
            .unwrap();
        executor
            .submit(Some("document".into()), move || finished.send(()).unwrap())
            .unwrap();
        executor.shutdown();
        observed.recv_timeout(DEADLINE).unwrap();
        assert_eq!(
            executor.submit(None, || {}),
            Err("host_command_executor_closed")
        );
    }
}
