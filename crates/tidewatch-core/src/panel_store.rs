//! A tab's `panel-log.jsonl`: one JSON event per line, kept under 2000 events and 512 KB, and a
//! background writer so the UI thread never waits on the disk.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::JoinHandle;

use crate::fsutil;
use crate::panel_event::Event;
use crate::panel_log::{CAP_EVENTS, Log};

pub const CAP_BYTES: usize = 512 * 1024;
const SMALLEST_KEEP: usize = 8;

fn line_of(event: &Event) -> Option<String> {
    serde_json::to_string(event).ok()
}

/// The log in `file`. A line that does not parse, such as one cut by a crash, is skipped; a missing
/// file is an empty log.
#[must_use]
pub fn load(file: &Path) -> Log {
    let Ok(text) = std::fs::read_to_string(file) else { return Log::new() };
    Log::from_events(text.lines().filter_map(|line| serde_json::from_str::<Event>(line).ok()))
}

/// Writes the whole log, replacing the file in one step.
pub fn save(file: &Path, log: &Log) -> std::io::Result<()> {
    fsutil::write_atomic(file, &text_of(log.events()))
}

fn text_of(events: &[Event]) -> String {
    let mut text = String::new();
    for line in events.iter().filter_map(line_of) {
        text.push_str(&line);
        text.push('\n');
    }
    text
}

fn size_of(log: &Log) -> usize {
    log.events().iter().filter_map(line_of).map(|line| line.len() + 1).sum()
}

/// Trims the log to half of each cap, so the file is not rewritten on every append.
pub fn compact(log: &mut Log) {
    let mut keep = CAP_EVENTS / 2;
    log.trim(keep);
    while size_of(log) > CAP_BYTES / 2 && keep > SMALLEST_KEEP {
        keep /= 2;
        log.trim(keep);
    }
}

struct Disk {
    file: PathBuf,
    lines: usize,
    bytes: usize,
    cut_tail: bool,
}

impl Disk {
    fn open(file: PathBuf) -> Self {
        let text = std::fs::read_to_string(&file).unwrap_or_default();
        let cut_tail = !text.is_empty() && !text.ends_with('\n');
        let mut disk = Self { file, lines: text.lines().count(), bytes: text.len(), cut_tail };
        disk.compact_if_over();
        disk
    }

    fn write(&mut self, batch: &[Event]) {
        let mut text = String::new();
        if self.cut_tail {
            text.push('\n');
        }
        text.push_str(&text_of(batch));
        let written = std::fs::create_dir_all(self.file.parent().unwrap_or(Path::new(".")))
            .and_then(|()| std::fs::OpenOptions::new().create(true).append(true).open(&self.file))
            .and_then(|mut open| open.write_all(text.as_bytes()));
        if let Err(e) = written {
            eprintln!("tidewatch: panel log {}: {e}", self.file.display());
            return;
        }
        self.cut_tail = false;
        self.lines += batch.len();
        self.bytes += text.len();
        self.compact_if_over();
    }

    fn compact_if_over(&mut self) {
        if self.lines < CAP_EVENTS && self.bytes <= CAP_BYTES {
            return;
        }
        let mut log = load(&self.file);
        compact(&mut log);
        match save(&self.file, &log) {
            Ok(()) => {
                self.lines = log.len();
                self.bytes = size_of(&log);
            }
            Err(e) => eprintln!("tidewatch: panel log {}: {e}", self.file.display()),
        }
    }
}

/// Appends events to the file one after the other on a background thread. Dropping it waits for
/// the events still queued.
pub struct Writer {
    sender: Option<Sender<Event>>,
    worker: Option<JoinHandle<()>>,
}

impl Writer {
    #[must_use]
    pub fn new(file: PathBuf) -> Self {
        let (sender, receiver) = channel();
        let worker = std::thread::spawn(move || run(file, &receiver));
        Self { sender: Some(sender), worker: Some(worker) }
    }

    pub fn append(&self, event: Event) {
        if let Some(sender) = &self.sender {
            let _ = sender.send(event);
        }
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        self.sender = None;
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn run(file: PathBuf, receiver: &Receiver<Event>) {
    let mut disk = Disk::open(file);
    while let Ok(first) = receiver.recv() {
        let batch: Vec<Event> = std::iter::once(first).chain(receiver.try_iter()).collect();
        disk.write(&batch);
    }
}

#[cfg(test)]
mod tests;
