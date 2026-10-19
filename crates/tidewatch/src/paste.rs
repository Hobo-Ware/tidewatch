//! What a paste puts into a terminal: the clipboard text with line breaks as CR and no control bytes.
//! egui turns Cmd+V into `Event::Paste` and never into a key press, so the terminal reads it here.

use stdusk_core::terminal::PtyTerm;

pub(crate) fn prepared(text: &str) -> Option<String> {
    let flat = text.replace("\r\n", "\r").replace('\n', "\r");
    let kept: String =
        flat.chars().filter(|c| matches!(c, '\r' | '\t') || !c.is_control()).collect();
    let end = kept.trim_end();
    let text = if end.contains('\r') { end } else { end.trim_start() };
    (!text.is_empty()).then(|| text.to_owned())
}

pub(crate) fn texts(events: &[egui::Event], terminal_has_keys: bool) -> Vec<String> {
    if !terminal_has_keys {
        return Vec::new();
    }
    events
        .iter()
        .filter_map(|event| match event {
            egui::Event::Paste(text) => prepared(text),
            _ => None,
        })
        .collect()
}

pub(crate) fn into_terminal(term: &mut PtyTerm, text: &str) {
    term.paste(text);
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::time::{Duration, Instant};

    use stdusk_core::repaint::Repaint;
    use stdusk_core::shell::ShellProfile;
    use stdusk_core::terminal::SpawnOpts;

    use super::*;

    #[test]
    fn line_breaks_become_cr_and_control_bytes_are_dropped() {
        let cases = [
            ("one\r\ntwo\nthree", Some("one\rtwo\rthree")),
            ("a\x1b[201~b\x03c", Some("a[201~bc")),
            ("  padded  \n", Some("padded")),
            ("  a\n  b\n\n", Some("  a\r  b")),
            ("tab\tstays", Some("tab\tstays")),
            ("naive \u{e9} text \u{1f600}", Some("naive \u{e9} text \u{1f600}")),
            ("", None),
            (" \r\n\t ", None),
        ];
        for (text, want) in cases {
            assert_eq!(prepared(text).as_deref(), want, "{text:?}");
        }
    }

    #[test]
    fn only_paste_events_reach_a_terminal_that_has_the_keys() {
        let events = [
            egui::Event::Text("x".into()),
            egui::Event::Paste("hello".into()),
            egui::Event::Copy,
            egui::Event::Paste("   ".into()),
            egui::Event::Paste("a\nb".into()),
        ];
        assert_eq!(texts(&events, true), ["hello", "a\rb"]);
        assert_eq!(texts(&events, false), Vec::<String>::new());
        assert_eq!(texts(&[], true), Vec::<String>::new());
    }

    fn opts(script: &str) -> SpawnOpts {
        SpawnOpts {
            detect_progress: false,
            shell_integration: false,
            autosuggestions: false,
            scrollback_lines: 200,
            word_separators: " ".into(),
            bold_bright: false,
            agent_tracking: None,
            cwd: None,
            profile: Some(ShellProfile {
                shell: Some("/bin/sh".into()),
                args: vec!["-c".into(), script.into()],
                cwd: None,
                env: BTreeMap::new(),
            }),
        }
    }

    fn received(arm: &str, text: &str, len: usize) -> Vec<u8> {
        let dir = tempfile::Builder::new().prefix("tw").tempdir_in("/tmp").unwrap();
        let (out, ready) = (dir.path().join("got"), dir.path().join("ready"));
        let script = format!(
            "{arm}stty raw -echo; : > '{}'; head -c {len} > '{}'",
            ready.display(),
            out.display()
        );
        let mut term = PtyTerm::spawn(80, 24, Repaint::none(), &opts(&script));
        let deadline = Instant::now() + Duration::from_secs(8);
        while !ready.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        std::thread::sleep(Duration::from_millis(200));
        into_terminal(&mut term, text);
        while Instant::now() < deadline {
            if std::fs::read(&out).is_ok_and(|bytes| bytes.len() >= len) {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        std::fs::read(&out).unwrap_or_default()
    }

    #[test]
    fn a_program_that_armed_bracketed_paste_gets_the_text_inside_the_markers() {
        let text = "alpha\rbeta";
        let want = format!("\x1b[200~{text}\x1b[201~");
        let got = received("printf '\\033[?2004h'; ", text, want.len());
        assert_eq!(String::from_utf8_lossy(&got), want);
    }

    #[test]
    fn a_program_that_did_not_arm_it_gets_the_bare_text() {
        let text = "alpha\rbeta";
        let got = received("", text, text.len());
        assert_eq!(String::from_utf8_lossy(&got), text);
    }
}
