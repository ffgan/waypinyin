//! Bounded, speculative dictionary-only first pages. Foreground work never waits.
use crate::lexicon::{plan, Lexicon, Plan, Prepared};
use crate::InputPreferences;
use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex, OnceLock, Weak};

const ENTRY_LIMIT: usize = 128;
const BYTE_LIMIT: usize = 8 * 1024 * 1024;
const INPUT_LIMIT: usize = 128;
const JOB_LIMIT: usize = 128;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Key {
    raw: String,
    preferences: InputPreferences,
}

struct Request {
    key: Key,
    caret: usize,
    plan: Option<Weak<Plan>>,
    hints: [u8; 5],
    favorites: Arc<[String]>,
}

enum Work {
    Expand(Request, u64, VecDeque<Job>),
    Prepare(Job, u64),
}

struct Job {
    key: Key,
    plan: Option<Weak<Plan>>,
}

#[derive(Default)]
struct State {
    generation: u64,
    latest: Option<(Key, usize, Arc<[String]>)>,
    jobs: VecDeque<Job>,
    pending: Option<Request>,
    entries: VecDeque<(Key, Arc<Prepared>, usize)>,
    bytes: usize,
}

impl State {
    fn store(
        &mut self,
        key: Key,
        prepared: Arc<Prepared>,
        bytes: usize,
        entries: usize,
        limit: usize,
    ) -> Vec<Arc<Prepared>> {
        let mut garbage = Vec::new();
        if bytes > limit || entries == 0 {
            return garbage;
        }
        while self.entries.len() >= entries || self.bytes + bytes > limit {
            let favored = |key: &Key| {
                self.latest.as_ref().is_some_and(|(active, _, favorites)| {
                    active.preferences == key.preferences
                        && favorites.iter().any(|f| f.starts_with(&key.raw))
                })
            };
            let index = self
                .entries
                .iter()
                .position(|(k, _, _)| k.raw.len() > 1 && !favored(k))
                .or_else(|| self.entries.iter().position(|(k, _, _)| k.raw.len() > 1))
                .unwrap_or(0);
            let (_, old, size) = self.entries.remove(index).unwrap();
            self.bytes -= size;
            garbage.push(old);
        }
        self.bytes += bytes;
        self.entries.push_back((key, prepared, bytes));
        garbage
    }
}

pub(crate) struct Preloader {
    shared: Arc<(Mutex<State>, Condvar)>,
}

impl std::fmt::Debug for Preloader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Preloader { .. }")
    }
}

impl Preloader {
    pub(crate) fn shared(lex: Arc<Lexicon>) -> Option<Arc<Self>> {
        static SHARED: OnceLock<Option<Arc<Preloader>>> = OnceLock::new();
        SHARED.get_or_init(|| Self::start(lex)).clone()
    }

    fn start(lex: Arc<Lexicon>) -> Option<Arc<Self>> {
        let shared = Arc::new((Mutex::new(State::default()), Condvar::new()));
        let worker = shared.clone();
        std::thread::Builder::new()
            .name("waypinyin-warm".into())
            .spawn(move || {
                // Linux pid=0 changes only this worker's policy. No privileges or
                // real-time priority are required; failure keeps correctness intact.
                let params = libc::sched_param { sched_priority: 0 };
                if unsafe { libc::sched_setscheduler(0, libc::SCHED_IDLE, &params) } != 0 {
                    unsafe { libc::setpriority(libc::PRIO_PROCESS, 0, 19) };
                }
                loop {
                    let work = {
                        let mut state = worker.0.lock().unwrap_or_else(|e| e.into_inner());
                        while state.jobs.is_empty() && state.pending.is_none() {
                            state = worker.1.wait(state).unwrap_or_else(|e| e.into_inner());
                        }
                        if let Some(request) = state.pending.take() {
                            let old = std::mem::take(&mut state.jobs);
                            Work::Expand(request, state.generation, old)
                        } else {
                            Work::Prepare(state.jobs.pop_front().unwrap(), state.generation)
                        }
                    };
                    let (job, generation) = match work {
                        Work::Expand(request, generation, old) => {
                            drop(old);
                            let jobs = expand(request, &lex);
                            let mut state = worker.0.lock().unwrap_or_else(|e| e.into_inner());
                            if state.generation == generation {
                                state.jobs = jobs;
                            }
                            continue;
                        }
                        Work::Prepare(job, generation) => (job, generation),
                    };
                    {
                        let state = worker.0.lock().unwrap_or_else(|e| e.into_inner());
                        if state.generation != generation
                            || state.entries.iter().any(|(key, _, _)| key == &job.key)
                        {
                            continue;
                        }
                    }
                    let selected = lex.with_preferences(job.key.preferences);
                    let planned = job
                        .plan
                        .and_then(|p| p.upgrade())
                        .unwrap_or_else(|| Arc::new(plan(&job.key.raw, &selected)));
                    let prepared =
                        Arc::new(Prepared::new(planned, &selected, crate::session::PAGE_SIZE));
                    let bytes =
                        prepared.bytes() + job.key.raw.capacity() + std::mem::size_of::<Key>();
                    if bytes > BYTE_LIMIT {
                        continue;
                    }
                    let garbage;
                    {
                        let mut state = worker.0.lock().unwrap_or_else(|e| e.into_inner());
                        if state.generation != generation {
                            continue;
                        }
                        garbage = state.store(job.key, prepared, bytes, ENTRY_LIMIT, BYTE_LIMIT);
                    }
                    drop(garbage);
                }
            })
            .ok()?;
        Some(Arc::new(Self { shared }))
    }

    pub(crate) fn get(&self, raw: &str, preferences: InputPreferences) -> Option<Arc<Prepared>> {
        let state = self.shared.0.try_lock().ok()?;
        state
            .entries
            .iter()
            .find(|(key, _, _)| key.preferences == preferences && key.raw == raw)
            .map(|(_, prepared, _)| prepared.clone())
    }

    pub(crate) fn ready(&self, raw: &str, preferences: InputPreferences) -> bool {
        if !raw.is_empty() {
            return self.get(raw, preferences).is_some();
        }
        let Ok(state) = self.shared.0.try_lock() else {
            return false;
        };
        (b'a'..=b'z').all(|c| {
            state
                .entries
                .iter()
                .any(|(k, _, _)| k.preferences == preferences && k.raw.as_bytes() == [c])
        })
    }

    pub(crate) fn request(
        &self,
        raw: &str,
        caret: usize,
        preferences: InputPreferences,
        current: Option<Weak<Plan>>,
        hints: [u8; 5],
        favorites: Arc<[String]>,
    ) {
        if raw.len() > INPUT_LIMIT || !raw.bytes().all(|c| c.is_ascii_lowercase() || c == b'\'') {
            return;
        }
        let Ok(mut state) = self.shared.0.try_lock() else {
            return;
        };
        let key = Key {
            raw: raw.into(),
            preferences,
        };
        if state.latest.as_ref().is_some_and(|(old, pos, previous)| {
            old == &key
                && *pos == caret
                && (Arc::ptr_eq(previous, &favorites) || previous == &favorites)
        }) {
            return;
        }
        state.generation = state.generation.wrapping_add(1);
        state.latest = Some((key.clone(), caret, favorites.clone()));
        state.pending = Some(Request {
            key,
            caret: caret.min(raw.len()),
            plan: current,
            hints,
            favorites,
        });
        self.shared.1.notify_one();
    }
}

// Queue construction and old-job destruction happen only on the idle worker.
fn expand(request: Request, lex: &Lexicon) -> VecDeque<Job> {
    let Request {
        key,
        caret,
        plan,
        hints,
        favorites,
    } = request;
    let raw = key.raw;
    let mut jobs: VecDeque<Job> = VecDeque::new();
    let mut queue = |raw: String, plan: Option<Weak<Plan>>| {
        if jobs.len() >= JOB_LIMIT
            || raw.is_empty()
            || !raw.bytes().all(|b| b.is_ascii_lowercase() || b == b'\'')
            || raw.len() > INPUT_LIMIT
            || matches!(raw.as_str(), "rq" | "sj" | "xq" | "jt" | "mt" | "zt")
        {
            return;
        }
        let key = Key {
            raw,
            preferences: key.preferences,
        };
        if !jobs.iter().any(|j| j.key == key) {
            jobs.push_back(Job { key, plan });
        }
    };
    queue(raw.clone(), plan);
    if raw.is_empty() {
        for favorite in favorites.iter() {
            if let Some(c) = favorite.bytes().next().filter(u8::is_ascii_lowercase) {
                queue((c as char).to_string(), None);
            }
        }
        for c in b'a'..=b'z' {
            queue((c as char).to_string(), None);
        }
        for favorite in favorites.iter() {
            queue(favorite.clone(), None);
        }
        for depth in 2..=INPUT_LIMIT {
            for favorite in favorites.iter() {
                if favorite.is_ascii() && favorite.len() > depth {
                    queue(favorite[..depth].to_string(), None);
                }
            }
        }
        return jobs;
    }
    if caret == raw.len() {
        for favorite in favorites
            .iter()
            .filter(|f| f.is_ascii() && f.starts_with(&raw) && f.len() > raw.len())
        {
            queue(favorite[..raw.len() + 1].to_string(), None);
        }
        for favorite in favorites.iter().filter(|f| f.starts_with(&raw)) {
            queue(favorite.clone(), None);
        }
    }
    for c in hints.into_iter().filter(u8::is_ascii_lowercase) {
        let mut next = raw.clone();
        next.insert(caret, c as char);
        queue(next, None);
    }
    let letters = if caret == raw.len() {
        lex.preload_letters(&raw)
    } else {
        (b'a'..=b'z').collect()
    };
    for c in letters {
        let mut next = raw.clone();
        next.insert(caret, c as char);
        queue(next, None);
    }
    if caret > 0 {
        let mut previous = raw.clone();
        previous.remove(caret - 1);
        queue(previous, None);
    }
    if caret < raw.len() {
        let mut deleted = raw;
        deleted.remove(caret);
        queue(deleted, None);
    }
    jobs
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn wait(preloader: &Preloader, raw: &str, prefs: InputPreferences) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !preloader.ready(raw, prefs) {
            assert!(Instant::now() < deadline, "preload did not complete: {raw}");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn worker_prepares_letters_and_separates_preferences() {
        let policy = unsafe { libc::sched_getscheduler(0) };
        let worker = Preloader::start(Arc::new(Lexicon::load())).unwrap();
        let prefs = InputPreferences::default();
        worker.request("", 0, prefs, None, [0; 5], Arc::from([]));
        wait(&worker, "", prefs);
        assert_eq!(unsafe { libc::sched_getscheduler(0) }, policy);
        let original = worker.get("n", prefs).unwrap();
        let off = InputPreferences {
            enabled: false,
            ..prefs
        };
        assert!(worker.get("n", off).is_none());
        worker.request("", 0, off, None, [0; 5], Arc::from([]));
        wait(&worker, "", off);
        assert!(!Arc::ptr_eq(
            &original.plan(),
            &worker.get("n", off).unwrap().plan()
        ));
        let state = worker.shared.0.lock().unwrap();
        assert!(state.entries.len() <= ENTRY_LIMIT && state.bytes <= BYTE_LIMIT);
    }

    #[test]
    fn foreground_does_not_wait_for_worker_lock() {
        let worker = Preloader::start(Arc::new(Lexicon::load())).unwrap();
        let state = worker.shared.0.lock().unwrap();
        assert!(worker.get("n", Default::default()).is_none());
        worker.request("n", 1, Default::default(), None, [0; 5], Arc::from([]));
        assert!(state.latest.is_none() && state.pending.is_none());
    }

    #[test]
    fn queue_is_bounded_prioritizes_hint_and_excludes_clock_values() {
        let jobs = expand(
            Request {
                key: Key {
                    raw: "s".into(),
                    preferences: Default::default(),
                },
                caret: 1,
                plan: None,
                hints: [b'h', 0, 0, 0, 0],
                favorites: Arc::from([]),
            },
            &Lexicon::load(),
        );
        assert_eq!(jobs[0].key.raw, "s");
        assert_eq!(jobs[1].key.raw, "sh");
        assert!(jobs.len() <= 29);
        assert!(jobs.iter().all(|j| j.key.raw != "sj"));
    }

    #[test]
    fn favorites_warm_initials_full_queries_and_prefixes_ahead_of_guesses() {
        let lex = Lexicon::load();
        let favorites: Arc<[String]> = vec!["zhongguo".into(), "nihao".into()].into();
        let initial = expand(
            Request {
                key: Key {
                    raw: String::new(),
                    preferences: Default::default(),
                },
                caret: 0,
                plan: None,
                hints: [0; 5],
                favorites: favorites.clone(),
            },
            &lex,
        );
        assert_eq!(initial[0].key.raw, "z");
        for raw in ["zh", "zhong", "zhongguo", "ni", "nihao"] {
            assert!(initial.iter().any(|j| j.key.raw == raw), "{raw}");
        }
        assert!(initial.len() <= JOB_LIMIT);
        let next = expand(
            Request {
                key: Key {
                    raw: "zh".into(),
                    preferences: Default::default(),
                },
                caret: 2,
                plan: None,
                hints: [b'a', 0, 0, 0, 0],
                favorites,
            },
            &lex,
        );
        assert_eq!(next[0].key.raw, "zh");
        assert_eq!(next[1].key.raw, "zho");
        assert_eq!(next[2].key.raw, "zhongguo");
    }

    #[test]
    fn idle_worker_prepares_frequent_complete_inputs_without_typing_them() {
        let lex = Arc::new(Lexicon::load());
        let worker = Preloader::start(lex.clone()).unwrap();
        let prefs = InputPreferences::default();
        worker.request(
            "",
            0,
            prefs,
            None,
            [0; 5],
            vec!["nihao".into(), "nh".into()].into(),
        );
        wait(&worker, "nihao", prefs);
        wait(&worker, "nh", prefs);
        let prepared = worker.get("nh", prefs).unwrap();
        let ordinary = Prepared::new(Arc::new(plan("nh", &lex)), &lex, 5);
        assert_eq!(
            prepared
                .out
                .iter()
                .map(|c| (&c.word, &c.reading, c.span))
                .collect::<Vec<_>>(),
            ordinary
                .out
                .iter()
                .map(|c| (&c.word, &c.reading, c.span))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn favorite_pages_survive_generic_eviction() {
        let lex = Lexicon::load();
        let ready = Arc::new(Prepared::new(Arc::new(plan("ni", &lex)), &lex, 5));
        let key = |raw: &str| Key {
            raw: raw.into(),
            preferences: Default::default(),
        };
        let mut state = State {
            latest: Some((key(""), 0, vec!["zhongguo".into()].into())),
            ..Default::default()
        };
        for raw in ["z", "zh", "zz", "za"] {
            state.store(key(raw), ready.clone(), 10, 3, 30);
        }
        assert_eq!(
            state
                .entries
                .iter()
                .map(|(k, _, _)| k.raw.as_str())
                .collect::<Vec<_>>(),
            ["z", "zh", "za"]
        );
    }

    #[test]
    fn cache_budget_and_initial_letter_retention() {
        let lex = Lexicon::load();
        let ready = Arc::new(Prepared::new(Arc::new(plan("ni", &lex)), &lex, 5));
        let mut state = State::default();
        let key = |raw: &str| Key {
            raw: raw.into(),
            preferences: Default::default(),
        };
        state.store(key("n"), ready.clone(), 10, 2, 20);
        state.store(key("ni"), ready.clone(), 10, 2, 20);
        state.store(key("nih"), ready.clone(), 10, 2, 20);
        assert_eq!(
            state
                .entries
                .iter()
                .map(|(k, _, _)| k.raw.as_str())
                .collect::<Vec<_>>(),
            ["n", "nih"]
        );
        assert_eq!(state.bytes, 20);
        state.store(key("too-big"), ready, 21, 2, 20);
        assert_eq!(state.entries.len(), 2);
    }
}
