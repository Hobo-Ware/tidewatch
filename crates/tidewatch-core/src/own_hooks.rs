//! The hooks the owner already runs in Claude Code that Tidewatch would repeat. One comment guard
//! per tab: when the owner's own comment-guard hook is in the Claude settings, Tidewatch leaves its
//! own out of that tab's settings file.

use std::path::Path;

use serde_json::Value;

const TIDEWATCH_OWN: &str = "--guard comments";
const NAMES: [&str; 2] = ["comment-guard", "comment_guard"];

/// The settings files Claude Code reads for a tab running in `dir`, user files first.
fn settings_files(claude_dir: &Path, dir: &Path) -> [std::path::PathBuf; 4] {
    [
        claude_dir.join("settings.json"),
        claude_dir.join("settings.local.json"),
        dir.join(".claude").join("settings.json"),
        dir.join(".claude").join("settings.local.json"),
    ]
}

fn commands(settings: &Value) -> Vec<&str> {
    let Some(events) = settings.get("hooks").and_then(Value::as_object) else {
        return Vec::new();
    };
    events
        .values()
        .filter_map(Value::as_array)
        .flatten()
        .filter_map(|group| group.get("hooks")?.as_array())
        .flatten()
        .filter_map(|hook| hook.get("command")?.as_str())
        .collect()
}

/// The word of `command` that names the guard: `comment-guard.py` for
/// `python3 ~/.claude/hooks/comment-guard.py`.
fn name_of(command: &str) -> String {
    let word = command
        .split_whitespace()
        .find(|word| NAMES.iter().any(|name| word.to_lowercase().contains(name)))
        .unwrap_or(command);
    let word = word.trim_matches(|c| c == '"' || c == '\'');
    word.rsplit('/').next().unwrap_or(word).to_owned()
}

/// The name of the owner's own comment-guard hook in the Claude settings that apply to a tab in
/// `dir`, when there is one. A file that is not JSON is skipped. Tidewatch's own hook never counts.
#[must_use]
pub fn own_comment_guard(claude_dir: &Path, dir: &Path) -> Option<String> {
    settings_files(claude_dir, dir).iter().find_map(|file| {
        let text = std::fs::read_to_string(file).ok()?;
        let settings: Value = serde_json::from_str(&text).ok()?;
        commands(&settings)
            .into_iter()
            .find(|command| {
                let lower = command.to_lowercase();
                !lower.contains(TIDEWATCH_OWN) && NAMES.iter().any(|name| lower.contains(name))
            })
            .map(name_of)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const OWN: &str = r#"{"hooks":{"PostToolUse":[{"matcher":"Write|Edit","hooks":[{"type":"command","command":"python3 ~/.claude/hooks/comment-guard.py"}]}]}}"#;

    fn write(dir: &Path, name: &str, text: &str) {
        let file = dir.join(name);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    }

    #[test]
    fn the_owners_hook_in_the_user_settings_is_found_by_its_name() {
        let home = tempfile::tempdir().unwrap();
        let tab = tempfile::tempdir().unwrap();
        write(home.path(), "settings.json", OWN);
        assert_eq!(own_comment_guard(home.path(), tab.path()), Some("comment-guard.py".to_owned()));
    }

    #[test]
    fn the_project_and_local_settings_count_too() {
        let home = tempfile::tempdir().unwrap();
        let tab = tempfile::tempdir().unwrap();
        write(
            tab.path(),
            ".claude/settings.local.json",
            &OWN.replace("comment-guard", "comment_guard"),
        );
        assert_eq!(own_comment_guard(home.path(), tab.path()), Some("comment_guard.py".to_owned()));
    }

    #[test]
    fn tidewatchs_own_hook_and_unrelated_hooks_are_not_the_owners() {
        let home = tempfile::tempdir().unwrap();
        let tab = tempfile::tempdir().unwrap();
        let ours = r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"/app/tidewatch --guard comments --stop"}]}]}}"#;
        let other =
            r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"notify-send done"}]}]}}"#;
        write(home.path(), "settings.json", ours);
        write(tab.path(), ".claude/settings.json", other);
        assert_eq!(own_comment_guard(home.path(), tab.path()), None);
    }

    #[test]
    fn a_missing_or_broken_file_is_no_hook_and_a_later_file_still_counts() {
        let home = tempfile::tempdir().unwrap();
        let tab = tempfile::tempdir().unwrap();
        assert_eq!(own_comment_guard(home.path(), tab.path()), None);
        write(home.path(), "settings.json", "{ not json");
        assert_eq!(own_comment_guard(home.path(), tab.path()), None);
        write(tab.path(), ".claude/settings.json", OWN);
        assert_eq!(own_comment_guard(home.path(), tab.path()), Some("comment-guard.py".to_owned()));
    }

    #[test]
    fn the_name_is_the_word_that_holds_the_guard() {
        let table = [
            ("python3 ~/.claude/hooks/comment-guard.py", "comment-guard.py"),
            ("'/Users/me/bin/Comment-Guard' --strict", "Comment-Guard"),
            ("comment_guard", "comment_guard"),
        ];
        for (command, want) in table {
            assert_eq!(name_of(command), want, "{command}");
        }
    }
}
