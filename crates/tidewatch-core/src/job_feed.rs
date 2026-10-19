//! The side channel of a long job: a script appends JSON lines to the file in `TIDEWATCH_PROGRESS`
//! and Tidewatch tails it. The reader skips bad lines and survives a file that is cut or replaced.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use serde_json::Value;

use crate::job::{JobState, Report};

const MAX_READ: u64 = 256 * 1024;
const MAX_PARTIAL: usize = 64 * 1024;

/// One line as a report. `name` (or `job`) is required; numbers may be quoted; unknown keys and a
/// state nobody knows are ignored.
#[must_use]
pub fn parse_line(line: &str) -> Option<Report> {
    let line = line.trim();
    if !line.starts_with('{') {
        return None;
    }
    let Value::Object(fields) = serde_json::from_str(line).ok()? else { return None };
    let text =
        |keys: &[&str]| keys.iter().find_map(|key| fields.get(*key)?.as_str().map(str::to_owned));
    let number = |key: &str| match fields.get(key)? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().replace(',', "").parse().ok(),
        _ => None,
    };
    let report = Report {
        name: text(&["name", "job"])?,
        done: number("done"),
        total: number("total"),
        unit: text(&["unit"]),
        phase: text(&["phase"]),
        state: text(&["state"]).and_then(|word| JobState::parse(&word)),
        eta_s: number("eta_s").filter(|secs| *secs >= 0.0).map(|secs| secs as u64),
    };
    report.cleaned()
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Feed {
    offset: u64,
    partial: Vec<u8>,
}

impl Feed {
    #[must_use]
    pub fn from_end(file: &Path) -> Self {
        let offset = std::fs::metadata(file).map_or(0, |meta| meta.len());
        Self { offset, partial: Vec::new() }
    }

    pub fn poll(&mut self, file: &Path) -> Vec<Report> {
        let Ok(meta) = std::fs::metadata(file) else { return Vec::new() };
        if meta.len() < self.offset {
            *self = Self::default();
        }
        if meta.len() == self.offset {
            return Vec::new();
        }
        let Ok(mut open) = std::fs::File::open(file) else { return Vec::new() };
        if open.seek(SeekFrom::Start(self.offset)).is_err() {
            return Vec::new();
        }
        let mut chunk = Vec::new();
        if open.take(MAX_READ).read_to_end(&mut chunk).is_err() {
            return Vec::new();
        }
        self.offset += chunk.len() as u64;
        self.partial.extend_from_slice(&chunk);
        let mut reports = Vec::new();
        while let Some(end) = self.partial.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = self.partial.drain(..=end).collect();
            reports.extend(parse_line(&String::from_utf8_lossy(&line)));
        }
        if self.partial.len() > MAX_PARTIAL {
            self.partial.clear();
        }
        reports
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    fn append(file: &Path, text: &str) {
        let mut open = std::fs::OpenOptions::new().create(true).append(true).open(file).unwrap();
        open.write_all(text.as_bytes()).unwrap();
    }

    #[test]
    fn a_full_line_becomes_a_report() {
        let line = r#"{"name":"orders","done":12431,"total":58650,"unit":"rows","phase":"write","state":"running"}"#;
        assert_eq!(
            parse_line(line),
            Some(Report {
                name: "orders".into(),
                done: Some(12431.0),
                total: Some(58650.0),
                unit: Some("rows".into()),
                phase: Some("write".into()),
                state: Some(JobState::Running),
                eta_s: None,
            })
        );
    }

    #[test]
    fn lines_are_read_tolerantly_and_bad_ones_are_dropped() {
        let cases = [
            (r#"{"job":"users","done":"8,755","total":"21940"}"#, Some(("users", Some(8755.0)))),
            (r#"{"name":"x"}"#, Some(("x", None))),
            (r#"{"name":"x","done":1,"extra":{"a":[1]}}"#, Some(("x", Some(1.0)))),
            (r#"  {"name":"x","done":2}  "#, Some(("x", Some(2.0)))),
            (r#"{"name":"x","done":null,"total":null}"#, Some(("x", None))),
            (r#"{"name":"x","state":"sleeping","done":3}"#, Some(("x", Some(3.0)))),
            (r#"{"done":3}"#, None),
            (r#"{"name":"  "}"#, None),
            (r#"{"name":5}"#, None),
            (r"[1,2]", None),
            (r#"{"name":"x","done":"#, None),
            ("progress 5/10", None),
            ("", None),
        ];
        for (line, want) in cases {
            let got = parse_line(line).map(|report| (report.name.clone(), report.done));
            let want = want.map(|(name, done)| (name.to_owned(), done));
            assert_eq!(got, want, "{line}");
        }
    }

    #[test]
    fn negative_and_zero_numbers_are_cleaned() {
        let report = parse_line(r#"{"name":"x","done":-4,"total":0,"eta_s":-1}"#).unwrap();
        assert_eq!((report.done, report.total, report.eta_s), (Some(0.0), None, None));
    }

    #[test]
    fn a_tail_returns_each_line_once_and_waits_for_a_line_to_finish() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("feed.jsonl");
        let mut feed = Feed::default();
        assert_eq!(feed.poll(&file), []);
        append(&file, "{\"name\":\"a\",\"done\":1}\nnot json\n{\"name\":\"a\",\"do");
        let first = feed.poll(&file);
        assert_eq!(first.len(), 1);
        assert_eq!(feed.poll(&file), []);
        append(&file, "ne\":2}\n{\"name\":\"b\",\"done\":5}\n");
        let more: Vec<_> = feed.poll(&file).into_iter().map(|r| (r.name, r.done)).collect();
        assert_eq!(more, [("a".into(), Some(2.0)), ("b".into(), Some(5.0))]);
        assert_eq!(feed.poll(&file), []);
    }

    #[test]
    fn a_file_that_is_cut_or_replaced_is_read_from_its_start() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("feed.jsonl");
        let mut feed = Feed::default();
        append(&file, "{\"name\":\"a\",\"done\":1}\n{\"name\":\"a\",\"done\":2}\n");
        assert_eq!(feed.poll(&file).len(), 2);
        std::fs::write(&file, "{\"name\":\"a\",\"done\":9}\n").unwrap();
        let again = feed.poll(&file);
        assert_eq!(again.len(), 1);
        assert_eq!(again[0].done, Some(9.0));
    }

    #[test]
    fn a_tail_started_at_the_end_skips_what_was_there() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("feed.jsonl");
        append(&file, "{\"name\":\"old\",\"done\":1}\n");
        let mut feed = Feed::from_end(&file);
        assert_eq!(feed.poll(&file), []);
        append(&file, "{\"name\":\"new\",\"done\":1}\n");
        assert_eq!(feed.poll(&file).len(), 1);
        assert_eq!(Feed::from_end(&dir.path().join("missing")), Feed::default());
    }

    #[test]
    fn binary_noise_and_a_runaway_line_do_not_break_the_tail() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("feed.jsonl");
        let mut feed = Feed::default();
        std::fs::write(&file, [0xff, 0xfe, b'\n', b'{', 0x00, b'\n']).unwrap();
        assert_eq!(feed.poll(&file), []);
        append(&file, &"x".repeat(2 * MAX_PARTIAL));
        assert_eq!(feed.poll(&file), []);
        assert_eq!(feed.partial, Vec::<u8>::new());
        append(&file, "\n{\"name\":\"ok\",\"done\":1}\n");
        assert_eq!(feed.poll(&file).len(), 1);
    }
}
