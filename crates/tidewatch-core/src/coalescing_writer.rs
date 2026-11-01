//! A background thread that saves values to files: a burst of saves collapses into the last value
//! per file, and dropping the writer waits for the pending writes, so quitting never loses one.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::JoinHandle;

type Save<T> = fn(&T, &Path) -> std::io::Result<()>;

pub struct CoalescingWriter<T: Send + 'static> {
    sender: Option<Sender<(PathBuf, T)>>,
    worker: Option<JoinHandle<()>>,
}

impl<T: Send + 'static> CoalescingWriter<T> {
    #[must_use]
    pub fn new(save: Save<T>) -> Self {
        let (sender, receiver) = channel();
        let worker = std::thread::spawn(move || write_loop(&receiver, save));
        Self { sender: Some(sender), worker: Some(worker) }
    }

    pub fn save(&self, file: PathBuf, value: T) {
        if let Some(sender) = &self.sender {
            let _ = sender.send((file, value));
        }
    }
}

impl<T: Send + 'static> Drop for CoalescingWriter<T> {
    fn drop(&mut self) {
        self.sender = None;
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn write_loop<T>(receiver: &Receiver<(PathBuf, T)>, save: Save<T>) {
    while let Ok(first) = receiver.recv() {
        let mut latest: Vec<(PathBuf, T)> = vec![first];
        for (file, value) in receiver.try_iter() {
            match latest.iter_mut().find(|(known, _)| *known == file) {
                Some(slot) => slot.1 = value,
                None => latest.push((file, value)),
            }
        }
        for (file, value) in latest {
            if let Err(e) = save(&value, &file) {
                eprintln!("tidewatch: cannot save {}: {e}", file.display());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Number(u64);

    fn write_number(number: &Number, file: &Path) -> std::io::Result<()> {
        crate::fsutil::write_atomic(file, &number.0.to_string())
    }

    #[test]
    fn keeps_the_last_value_per_file_and_flushes_on_drop() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a").join("x.txt");
        let b = dir.path().join("b").join("x.txt");
        let writer = CoalescingWriter::new(write_number);
        for n in 1..=20 {
            writer.save(a.clone(), Number(n));
        }
        writer.save(b.clone(), Number(7));
        drop(writer);
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "20");
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "7");
    }

    #[test]
    fn a_failed_save_does_not_stop_the_next_one() {
        let dir = tempfile::tempdir().unwrap();
        let blocked = dir.path().join("file");
        std::fs::write(&blocked, "x").unwrap();
        let good = dir.path().join("good.txt");
        let writer = CoalescingWriter::new(write_number);
        writer.save(blocked.join("under"), Number(1));
        writer.save(good.clone(), Number(2));
        drop(writer);
        assert_eq!(std::fs::read_to_string(&good).unwrap(), "2");
    }
}
