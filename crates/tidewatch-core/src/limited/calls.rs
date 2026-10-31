//! The calls running through the limited runner right now, so the window can show a slow one and
//! cancel it. Shared between job threads and the UI thread; every lock is grab-copy-drop.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, Instant};

pub const SLOW_AFTER: Duration = Duration::from_secs(10);

#[derive(Clone, Debug, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Running {
    pub id: u64,
    pub label: String,
    pub started: Instant,
}

struct Entry {
    running: Running,
    cancel: Cancel,
}

type Waker = Box<dyn Fn() + Send + Sync>;

static CALLS: Mutex<Vec<Entry>> = Mutex::new(Vec::new());
static NEXT_ID: AtomicU64 = AtomicU64::new(1);
static WAKER: OnceLock<Waker> = OnceLock::new();

fn calls() -> MutexGuard<'static, Vec<Entry>> {
    CALLS.lock().unwrap_or_else(PoisonError::into_inner)
}

pub fn set_waker(waker: impl Fn() + Send + Sync + 'static) {
    let _ = WAKER.set(Box::new(waker));
}

fn wake() {
    if let Some(waker) = WAKER.get() {
        waker();
    }
}

#[must_use]
pub fn running() -> Vec<Running> {
    let mut all: Vec<Running> = calls().iter().map(|entry| entry.running.clone()).collect();
    all.sort_by_key(|call| call.started);
    all
}

/// Cancels the call with this id. False when it already ended.
pub fn cancel_call(id: u64) -> bool {
    let cancel = calls().iter().find(|entry| entry.running.id == id).map(|e| e.cancel.clone());
    cancel.inspect(Cancel::cancel).is_some()
}

/// A call's place in the list. Dropping it ends the entry, however the call ended.
pub struct Registration {
    id: u64,
    cancel: Cancel,
    told: bool,
}

impl Registration {
    #[must_use]
    pub fn new(label: String, cancel: Cancel, started: Instant) -> Self {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let running = Running { id, label, started };
        calls().push(Entry { running, cancel: cancel.clone() });
        Self { id, cancel, told: false }
    }

    #[must_use]
    pub fn cancel(&self) -> &Cancel {
        &self.cancel
    }

    pub fn note(&mut self, elapsed: Duration) {
        if !self.told && elapsed >= SLOW_AFTER {
            self.told = true;
            wake();
        }
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        calls().retain(|entry| entry.running.id != self.id);
        if self.told {
            wake();
        }
    }
}

#[must_use]
pub fn hold(label: &str, age: Duration) -> Registration {
    let now = Instant::now();
    let mut registration =
        Registration::new(label.to_owned(), Cancel::new(), now.checked_sub(age).unwrap_or(now));
    registration.note(age);
    registration
}

const GIT_FAMILIES: [&str; 6] = ["worktree", "remote", "stash", "submodule", "notes", "config"];
const GH_FAMILIES: [&str; 8] = ["pr", "run", "issue", "repo", "release", "workflow", "auth", "api"];

fn is_word(arg: &str) -> bool {
    arg.len() <= 16
        && arg.starts_with(|c: char| c.is_ascii_lowercase())
        && arg.chars().all(|c| c.is_ascii_lowercase() || c == '-')
}

/// `git worktree add`, `gh pr view`, `git status`, or the bare program name.
#[must_use]
pub fn label_from(program: &str, args: &[String]) -> String {
    let families: &[&str] = match program {
        "git" => &GIT_FAMILIES,
        "gh" => &GH_FAMILIES,
        _ => return program.to_owned(),
    };
    let mut words = Vec::new();
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        if arg == "-C" || arg == "-c" {
            rest.next();
        } else if !arg.starts_with('-') && is_word(arg) {
            words.push(arg.as_str());
            if words.len() == 1 && !families.contains(&arg.as_str()) {
                break;
            }
            if words.len() == 2 {
                break;
            }
        } else if !arg.starts_with('-') {
            break;
        }
    }
    std::iter::once(program).chain(words).collect::<Vec<_>>().join(" ")
}
