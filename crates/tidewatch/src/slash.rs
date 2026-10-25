//! The Reader reply box's slash commands: which Claude Code commands open a dialog in the CLI's own
//! screen, and the completion list built from its commands, skills and custom commands.

use std::fs;
use std::path::{Path, PathBuf};

pub(crate) const MAX_ROWS: usize = 8;
const DESCRIPTION_CHARS: usize = 90;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Origin {
    Builtin,
    Command,
    Skill,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Entry {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) origin: Origin,
}

/// Command names as read from Claude Code 2.1.310's own command table.
const BUILTINS: &[(&str, &str, bool)] = &[
    ("add-dir", "Add a working directory", false),
    ("agents", "Manage subagents", true),
    ("btw", "Ask a side question without interrupting", false),
    ("bug", "Send feedback about Claude Code", true),
    ("clear", "Clear the conversation", false),
    ("compact", "Summarize the conversation to free context", false),
    ("config", "Open the settings", true),
    ("context", "Show how much context is used", false),
    ("copy", "Copy the last reply", false),
    ("diff", "View the changes made in this session", true),
    ("effort", "Set how hard the model thinks", true),
    ("exit", "Quit Claude Code", false),
    ("export", "Export the conversation", false),
    ("fast", "Toggle fast mode", false),
    ("feedback", "Send feedback about Claude Code", true),
    ("goal", "Set a goal for the session", false),
    ("help", "Show the commands and shortcuts", true),
    ("hooks", "View the hook configuration", true),
    ("ide", "Connect to an IDE", true),
    ("init", "Create a CLAUDE.md for this repo", false),
    ("insights", "Report on your sessions", false),
    ("install-github-app", "Set up the GitHub app for pull requests", true),
    ("login", "Sign in", true),
    ("logout", "Sign out", true),
    ("mcp", "Manage MCP servers", true),
    ("memory", "Edit memory files", true),
    ("model", "Pick the model", true),
    ("output-style", "Pick an output style", true),
    ("permissions", "Manage tool permissions", true),
    ("plan", "Enter plan mode", false),
    ("plugin", "Manage plugins", true),
    ("privacy-settings", "View and update privacy settings", true),
    ("recap", "Recap the session so far", false),
    ("remote-env", "Pick the remote environment", true),
    ("rename", "Rename the session", false),
    ("resume", "Resume an earlier session", true),
    ("skills", "List the available skills", true),
    ("status", "Show version, model and account", true),
    ("tasks", "Manage background tasks", true),
    ("terminal-setup", "Set up terminal key bindings", false),
    ("theme", "Pick the color theme", true),
    ("usage", "Show plan usage and limits", true),
    ("version", "Show the version", false),
];

pub(crate) fn builtins() -> Vec<Entry> {
    BUILTINS
        .iter()
        .map(|(name, description, _)| Entry {
            name: (*name).to_owned(),
            description: (*description).to_owned(),
            origin: Origin::Builtin,
        })
        .collect()
}

/// The interactive built-in that a bare `/name` sends; with arguments it only does its work.
pub(crate) fn interactive(text: &str) -> Option<&'static str> {
    let word = text.trim().strip_prefix('/')?;
    BUILTINS.iter().find(|(name, _, opens)| *opens && *name == word).map(|(name, _, _)| *name)
}

/// The first word after the leading slash, while it is still being typed.
pub(crate) fn query(text: &str) -> Option<&str> {
    let word = text.strip_prefix('/')?;
    (!word.contains(char::is_whitespace)).then_some(word)
}

/// Names that start with `query` first, then names that contain it, each in name order.
pub(crate) fn matches<'a>(entries: &'a [Entry], query: &str) -> Vec<&'a Entry> {
    let wanted = query.to_lowercase();
    let mut starts: Vec<&Entry> = Vec::new();
    let mut inside: Vec<&Entry> = Vec::new();
    for entry in entries {
        let name = entry.name.to_lowercase();
        if name.starts_with(&wanted) {
            starts.push(entry);
        } else if name.contains(&wanted) {
            inside.push(entry);
        }
    }
    starts.sort_by(|a, b| a.name.cmp(&b.name));
    inside.sort_by(|a, b| a.name.cmp(&b.name));
    starts.into_iter().chain(inside).take(MAX_ROWS).collect()
}

pub(crate) fn completion(entry: &Entry) -> String {
    format!("/{} ", entry.name)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Key {
    Up,
    Down,
    Accept,
    Close,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Pressed {
    Moved,
    Accepted(String),
    Closed,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Menu {
    pub(crate) picked: usize,
    pub(crate) open: bool,
    closed_for: Option<String>,
}

impl Menu {
    /// The rows to show for `text`: none after Esc until the text changes.
    pub(crate) fn rows<'a>(&self, text: &str, entries: &'a [Entry]) -> Vec<&'a Entry> {
        if self.closed_for.as_deref() == Some(text) {
            return Vec::new();
        }
        query(text).map(|query| matches(entries, query)).unwrap_or_default()
    }

    pub(crate) fn picked_in(&self, rows: usize) -> usize {
        self.picked.min(rows.saturating_sub(1))
    }

    pub(crate) fn enter_sends(&self, text: &str, rows: &[&Entry]) -> bool {
        let typed = query(text).unwrap_or_default();
        rows.get(self.picked_in(rows.len())).is_some_and(|row| row.name == typed)
    }

    pub(crate) fn press(&mut self, key: Key, text: &str, rows: &[&Entry]) -> Pressed {
        let at = self.picked_in(rows.len());
        let count = rows.len().max(1);
        match key {
            Key::Down => {
                self.picked = (at + 1) % count;
                Pressed::Moved
            }
            Key::Up => {
                self.picked = (at + count - 1) % count;
                Pressed::Moved
            }
            Key::Accept => match rows.get(at) {
                Some(row) => {
                    self.picked = 0;
                    Pressed::Accepted(completion(row))
                }
                None => Pressed::Closed,
            },
            Key::Close => {
                self.closed_for = Some(text.to_owned());
                Pressed::Closed
            }
        }
    }
}

fn frontmatter(markdown: &str) -> Option<Vec<&str>> {
    let mut lines = markdown.lines();
    (lines.next()?.trim_end() == "---").then_some(())?;
    Some(lines.take_while(|line| line.trim_end() != "---").collect())
}

fn field(front: &[&str], key: &str) -> Option<String> {
    let at = front.iter().position(|line| {
        line.strip_prefix(key).is_some_and(|rest| rest.trim_start().starts_with(':'))
    })?;
    let value = front[at].split_once(':')?.1.trim();
    let folded = matches!(value, ">" | "|" | ">-" | "|-" | ">+" | "|+");
    let text = if folded {
        front[at + 1..]
            .iter()
            .take_while(|line| line.starts_with([' ', '\t']) || line.is_empty())
            .map(|line| line.trim())
            .find(|line| !line.is_empty())?
    } else {
        value
    };
    let text = text.trim_matches(|c| c == '"' || c == '\'').trim();
    (!text.is_empty()).then(|| text.to_owned())
}

fn one_line(text: &str) -> String {
    let line = text.lines().next().unwrap_or_default().trim();
    if line.chars().count() <= DESCRIPTION_CHARS {
        return line.to_owned();
    }
    let cut: String = line.chars().take(DESCRIPTION_CHARS - 3).collect();
    format!("{}...", cut.trim_end())
}

/// The name and the one-line description in a skill or command file's front matter.
pub(crate) fn describe(markdown: &str) -> (Option<String>, String) {
    let Some(front) = frontmatter(markdown) else { return (None, String::new()) };
    let description = field(&front, "description").map(|text| one_line(&text)).unwrap_or_default();
    (field(&front, "name"), description)
}

fn folder_name(path: &Path) -> Option<String> {
    path.file_name().map(|name| name.to_string_lossy().into_owned())
}

fn skills_in(root: &Path, prefix: &str) -> Vec<Entry> {
    let Ok(dirs) = fs::read_dir(root) else { return Vec::new() };
    let mut found: Vec<Entry> = dirs
        .flatten()
        .filter_map(|dir| {
            let text = fs::read_to_string(dir.path().join("SKILL.md")).ok()?;
            let (name, description) = describe(&text);
            let name = name.or_else(|| folder_name(&dir.path()))?;
            Some(Entry { name: format!("{prefix}{name}"), description, origin: Origin::Skill })
        })
        .collect();
    found.sort_by(|a, b| a.name.cmp(&b.name));
    found
}

fn commands_in(root: &Path, prefix: &str) -> Vec<Entry> {
    let Ok(files) = fs::read_dir(root) else { return Vec::new() };
    let mut found: Vec<Entry> = files
        .flatten()
        .filter(|file| file.path().extension().is_some_and(|ext| ext == "md"))
        .filter_map(|file| {
            let stem = file.path().file_stem()?.to_string_lossy().into_owned();
            let text = fs::read_to_string(file.path()).ok()?;
            Some(Entry {
                name: format!("{prefix}{stem}"),
                description: describe(&text).1,
                origin: Origin::Command,
            })
        })
        .collect();
    found.sort_by(|a, b| a.name.cmp(&b.name));
    found
}

fn subfolders(dir: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    dirs.sort();
    dirs
}

fn plugin_roots(claude_dir: &Path) -> Vec<(String, PathBuf)> {
    let cache = claude_dir.join("plugins").join("cache");
    let mut roots = Vec::new();
    for marketplace in subfolders(&cache) {
        for plugin in subfolders(&marketplace) {
            if let (Some(name), Some(version)) = (folder_name(&plugin), subfolders(&plugin).pop()) {
                roots.push((name, version));
            }
        }
    }
    roots
}

/// Built-ins, then repo, user and plugin skills and commands; the first of a name wins. Blocks on disk.
pub(crate) fn load(claude_dir: &Path, folder: &Path) -> Vec<Entry> {
    let mut all = builtins();
    let mut add = |entries: Vec<Entry>| {
        for entry in entries {
            if !all.iter().any(|known| known.name == entry.name) {
                all.push(entry);
            }
        }
    };
    let repo = folder.join(".claude");
    add(commands_in(&repo.join("commands"), ""));
    add(skills_in(&repo.join("skills"), ""));
    add(commands_in(&claude_dir.join("commands"), ""));
    add(skills_in(&claude_dir.join("skills"), ""));
    for (plugin, root) in plugin_roots(claude_dir) {
        let prefix = format!("{plugin}:");
        add(skills_in(&root.join("skills"), &prefix));
        add(commands_in(&root.join("commands"), &prefix));
    }
    all
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, origin: Origin) -> Entry {
        Entry { name: name.to_owned(), description: String::new(), origin }
    }

    fn names<'a>(rows: &[&'a Entry]) -> Vec<&'a str> {
        rows.iter().map(|row| row.name.as_str()).collect()
    }

    #[test]
    fn a_bare_interactive_command_opens_the_terminal_and_nothing_else_does() {
        let cases = [
            ("/login", Some("login")),
            ("/logout", Some("logout")),
            ("  /usage \n", Some("usage")),
            ("/config", Some("config")),
            ("/model", Some("model")),
            ("/mcp", Some("mcp")),
            ("/permissions", Some("permissions")),
            ("/resume", Some("resume")),
            ("/agents", Some("agents")),
            ("/plugin", Some("plugin")),
            ("/status", Some("status")),
            ("/theme", Some("theme")),
            ("/model opus", None),
            ("/context", None),
            ("/cost", None),
            ("/compact", None),
            ("/clear", None),
            ("/my-skill", None),
            ("login", None),
            ("please /login", None),
            ("", None),
        ];
        for (text, want) in cases {
            assert_eq!(interactive(text), want, "text {text:?}");
        }
    }

    #[test]
    fn the_query_is_the_first_word_while_it_is_still_being_typed() {
        let cases = [
            ("/", Some("")),
            ("/lo", Some("lo")),
            ("/login", Some("login")),
            ("/login ", None),
            ("/model opus", None),
            ("/a\nb", None),
            ("hello /lo", None),
            ("", None),
        ];
        for (text, want) in cases {
            assert_eq!(query(text), want, "text {text:?}");
        }
    }

    #[test]
    fn names_that_start_with_the_query_come_first_then_names_that_contain_it() {
        let entries = [
            entry("pillow", Origin::Skill),
            entry("logout", Origin::Builtin),
            entry("login", Origin::Builtin),
            entry("catalog", Origin::Skill),
            entry("Loop", Origin::Command),
        ];
        assert_eq!(
            names(&matches(&entries, "lo")),
            ["Loop", "login", "logout", "catalog", "pillow"]
        );
        assert_eq!(names(&matches(&entries, "LOGI")), ["login"]);
        assert_eq!(matches(&entries, "zzz").len(), 0);
        assert_eq!(matches(&entries, "").len(), 5);
    }

    #[test]
    fn the_list_is_capped() {
        let entries: Vec<Entry> =
            (0..30).map(|n| entry(&format!("skill-{n:02}"), Origin::Skill)).collect();
        assert_eq!(matches(&entries, "s").len(), MAX_ROWS);
    }

    #[test]
    fn a_description_is_read_from_the_front_matter() {
        let cases = [
            ("---\nname: foo\ndescription: Use when X.\n---\nbody", Some("foo"), "Use when X."),
            ("---\ndescription: \"Quoted: yes\"\n---\n", None, "Quoted: yes"),
            ("---\ndescription: 'single'\n---\n", None, "single"),
            (
                "---\nname: f\ndescription: >\n  First line\n  second line\n---\n",
                Some("f"),
                "First line",
            ),
            ("---\ndescription: |-\n\n  Literal\n---\n", None, "Literal"),
            ("---\nname: only\n---\nbody", Some("only"), ""),
            ("no front matter", None, ""),
            ("", None, ""),
            ("---\ndescription: never closed", None, "never closed"),
        ];
        for (text, name, description) in cases {
            assert_eq!(
                describe(text),
                (name.map(str::to_owned), description.to_owned()),
                "text {text:?}"
            );
        }
    }

    #[test]
    fn a_long_description_is_cut_to_one_line_with_dots() {
        let long = "word ".repeat(40);
        let (_, description) = describe(&format!("---\ndescription: {long}\n---\n"));
        assert!(description.chars().count() <= DESCRIPTION_CHARS);
        assert!(description.ends_with("..."));
    }

    #[test]
    fn skills_commands_and_plugins_are_found_on_disk_once_each() {
        let home = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        let write = |path: PathBuf, text: &str| {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        };
        let skill = |root: &Path, rest: &str, text: &str| {
            write(root.join(rest).join("SKILL.md"), text);
        };
        skill(home.path(), "skills/alpha", "---\nname: alpha\ndescription: Mine.\n---\n");
        skill(home.path(), "skills/dup", "---\ndescription: From home.\n---\n");
        skill(repo.path(), ".claude/skills/dup", "---\ndescription: From repo.\n---\n");
        skill(repo.path(), ".claude/skills/nofront", "just text");
        write(home.path().join("commands/deploy.md"), "---\ndescription: Ship it.\n---\n");
        let plug = "plugins/cache/market/plug";
        skill(
            home.path(),
            &format!("{plug}/1.0.0/skills/deep"),
            "---\ndescription: Old one.\n---\n",
        );
        skill(
            home.path(),
            &format!("{plug}/2.0.0/skills/deep"),
            "---\ndescription: New one.\n---\n",
        );
        let all = load(home.path(), repo.path());
        let by = |name: &str| all.iter().filter(|entry| entry.name == name).collect::<Vec<_>>();
        assert_eq!(by("dup").len(), 1);
        assert_eq!(by("dup")[0].description, "From repo.");
        assert_eq!(by("alpha")[0].description, "Mine.");
        assert_eq!(by("nofront")[0].description, "");
        assert_eq!(by("deploy")[0].origin, Origin::Command);
        assert_eq!(by("plug:deep").len(), 1);
        assert_eq!(by("plug:deep")[0].description, "New one.");
        assert_eq!(by("login").len(), 1);
        assert_eq!(by("login")[0].origin, Origin::Builtin);
    }

    #[test]
    fn a_missing_folder_gives_just_the_built_ins() {
        let all = load(Path::new("/nonexistent-claude"), Path::new("/nonexistent-repo"));
        assert_eq!(all, builtins());
    }

    #[test]
    fn keys_move_wrap_accept_and_close() {
        let entries = [entry("login", Origin::Builtin), entry("logout", Origin::Builtin)];
        let rows = matches(&entries, "lo");
        let mut menu = Menu::default();
        assert_eq!(menu.press(Key::Down, "/lo", &rows), Pressed::Moved);
        assert_eq!(menu.picked_in(rows.len()), 1);
        assert_eq!(menu.press(Key::Down, "/lo", &rows), Pressed::Moved);
        assert_eq!(menu.picked_in(rows.len()), 0);
        assert_eq!(menu.press(Key::Up, "/lo", &rows), Pressed::Moved);
        assert_eq!(menu.picked_in(rows.len()), 1);
        assert_eq!(menu.press(Key::Accept, "/lo", &rows), Pressed::Accepted("/logout ".to_owned()));
        assert_eq!(menu.press(Key::Close, "/lo", &rows), Pressed::Closed);
        assert_eq!(menu.rows("/lo", &entries).len(), 0);
        assert_eq!(menu.rows("/log", &entries).len(), 2);
    }

    #[test]
    fn enter_sends_a_whole_command_name_but_completes_a_part_of_one() {
        let entries = [entry("login", Origin::Builtin), entry("logout", Origin::Builtin)];
        let menu = Menu::default();
        let part = menu.rows("/lo", &entries);
        assert!(!menu.enter_sends("/lo", &part));
        let whole = menu.rows("/login", &entries);
        assert!(menu.enter_sends("/login", &whole));
    }
}
