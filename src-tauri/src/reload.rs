//! Swapping the engine at runtime: requests collapse within a debounce
//! window, one background thread builds the newest, and a failed build
//! keeps the previous engine.
//!
//! speechkit 0.2 shipped this as `asr::reload::EngineManager`; 0.3 dropped
//! it, so Viary keeps its own, with the same behavior.

use std::{
    sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, RwLock},
    time::{Duration, Instant},
};

use speechkit::SpeechError;

type Factory<E> = Box<dyn FnOnce() -> Result<E, SpeechError> + Send>;

/// Numbers reload requests. Later requests have larger generations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Generation(pub u64);

struct State<E> {
    /// The newest requested generation.
    requested: u64,
    /// The newest generation whose outcome is known.
    settled: u64,
    /// The outcome of `settled`: `None` if it loaded.
    outcome: Option<SpeechError>,
    /// The newest factory, not yet run.
    pending: Option<(u64, Factory<E>)>,
    /// When the pending request may start: every request pushes it back.
    due: Instant,
    worker: bool,
}

struct Inner<E> {
    current: RwLock<E>,
    state: Mutex<State<E>>,
    changed: Condvar,
    debounce: Duration,
}

/// Holds the current engine, and replaces it in the background.
pub struct EngineManager<E> {
    inner: Arc<Inner<E>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl<E: Clone + Send + Sync + 'static> EngineManager<E> {
    /// A manager starting with `engine`, whose requests wait `debounce`.
    pub fn with_debounce(engine: E, debounce: Duration) -> Self {
        Self {
            inner: Arc::new(Inner {
                current: RwLock::new(engine),
                state: Mutex::new(State {
                    requested: 0,
                    settled: 0,
                    outcome: None,
                    pending: None,
                    due: Instant::now(),
                    worker: false,
                }),
                changed: Condvar::new(),
                debounce,
            }),
        }
    }

    /// The engine for new work.
    pub fn current(&self) -> E {
        self.inner
            .current
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Asks for a new engine built by `factory`, after the debounce window.
    /// A newer request made before it starts replaces it.
    pub fn request_reload(
        &self,
        factory: impl FnOnce() -> Result<E, SpeechError> + Send + 'static,
    ) -> Generation {
        let inner = &self.inner;
        let mut state = lock(&inner.state);
        state.requested += 1;
        let generation = state.requested;
        state.pending = Some((generation, Box::new(factory)));
        state.due = Instant::now() + inner.debounce;
        if !state.worker {
            state.worker = true;
            let worker = inner.clone();
            let spawned = std::thread::Builder::new()
                .name("viary-reload".into())
                .spawn(move || worker.run());
            if let Err(error) = spawned {
                state.worker = false;
                state.pending = None;
                settle(
                    &mut state,
                    generation,
                    Some(SpeechError::backend("viary", true, error)),
                );
            }
        }
        drop(state);
        inner.changed.notify_all();
        Generation(generation)
    }

    /// Waits until `generation` is decided, or `deadline`. A generation a
    /// newer request replaced is decided by that request's outcome.
    ///
    /// # Errors
    ///
    /// `DeadlineExceeded` on timeout, or the deciding load's error.
    pub fn wait(&self, generation: Generation, deadline: Instant) -> Result<(), SpeechError> {
        let inner = &self.inner;
        let mut state = lock(&inner.state);
        while state.settled < generation.0 {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(SpeechError::DeadlineExceeded);
            }
            state = inner
                .changed
                .wait_timeout(state, left)
                .map_or_else(|p| p.into_inner().0, |(state, _)| state);
        }
        state.outcome.clone().map_or(Ok(()), Err)
    }
}

fn settle<E>(state: &mut State<E>, generation: u64, outcome: Option<SpeechError>) {
    state.settled = state.settled.max(generation);
    state.outcome = outcome;
}

impl<E: Clone + Send + Sync + 'static> Inner<E> {
    fn run(self: Arc<Self>) {
        loop {
            let mut state = lock(&self.state);
            // Wait out the debounce; each new request pushes `due` back.
            loop {
                let left = state.due.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    break;
                }
                state = self
                    .changed
                    .wait_timeout(state, left)
                    .map_or_else(|p| p.into_inner().0, |(state, _)| state);
            }
            let Some((generation, factory)) = state.pending.take() else {
                state.worker = false;
                return;
            };
            drop(state);
            let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(factory))
                .unwrap_or_else(|_| {
                    Err(SpeechError::backend("viary", false, "the engine factory panicked"))
                });
            let outcome = match built {
                Ok(engine) => {
                    *self.current.write().unwrap_or_else(PoisonError::into_inner) = engine;
                    None
                }
                Err(error) => {
                    tracing::warn!(generation, %error, "engine reload failed; keeping the previous engine");
                    Some(error)
                }
            };
            settle(&mut lock(&self.state), generation, outcome);
            self.changed.notify_all();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn soon() -> Instant {
        Instant::now() + Duration::from_secs(10)
    }

    fn manager() -> EngineManager<u32> {
        EngineManager::with_debounce(0, Duration::from_millis(30))
    }

    #[test]
    fn requests_within_the_window_collapse() {
        let manager = manager();
        let calls = Arc::new(AtomicUsize::new(0));
        let mut last = Generation(0);
        for value in 1..=3 {
            let calls = calls.clone();
            last = manager.request_reload(move || {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(value)
            });
        }
        manager.wait(Generation(1), soon()).unwrap();
        manager.wait(last, soon()).unwrap();
        assert_eq!(manager.current(), 3);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn failure_keeps_the_previous_engine() {
        let manager = manager();
        manager.wait(manager.request_reload(|| Ok(7)), soon()).unwrap();
        let failed = manager.request_reload(|| Err(SpeechError::InvalidModel("broken".into())));
        assert!(matches!(
            manager.wait(failed, soon()),
            Err(SpeechError::InvalidModel(_))
        ));
        assert_eq!(manager.current(), 7);
        let panicked = manager.request_reload(|| panic!("factory bug"));
        assert!(manager.wait(panicked, soon()).is_err());
        assert_eq!(manager.current(), 7);
    }

    #[test]
    fn wait_times_out() {
        let manager = EngineManager::with_debounce(0_u32, Duration::from_secs(5));
        let generation = manager.request_reload(|| Ok(1));
        let waited = manager.wait(generation, Instant::now() + Duration::from_millis(20));
        assert!(matches!(waited, Err(SpeechError::DeadlineExceeded)));
    }
}
