use std::collections::HashMap;

use super::*;
use crate::guard_args::{parse_gh, parse_git};

#[derive(Default)]
struct Fake {
    files: HashMap<String, String>,
    public: Option<bool>,
    asked: Vec<(Option<PathBuf>, Option<String>)>,
}

impl Machine for Fake {
    fn read(&mut self, dir: Option<&Path>, source: &Source) -> Option<String> {
        match source {
            Source::Inline(text) => Some(text.clone()),
            Source::File(path) => {
                let full = dir.map_or_else(|| path.clone(), |d| d.join(path));
                self.files.get(&full.display().to_string()).cloned()
            }
            Source::Stdin => self.files.get("-").cloned(),
        }
    }

    fn is_public(&mut self, dir: Option<&Path>, repo: Option<&str>) -> Option<bool> {
        self.asked.push((dir.map(Path::to_path_buf), repo.map(str::to_owned)));
        self.public
    }
}

fn args(line: &[&str]) -> Vec<String> {
    line.iter().map(|a| (*a).to_owned()).collect()
}

fn git(line: &[&str], rules: Rules, private: &[&str], machine: &mut Fake) -> Vec<Rule> {
    let private: Vec<String> = private.iter().map(|n| (*n).to_owned()).collect();
    let commit = parse_git(&args(line)).unwrap();
    check_git(&commit, rules, &private, machine).iter().map(|v| v.rule).collect()
}

fn gh(line: &[&str], rules: Rules, private: &[&str], machine: &mut Fake) -> Vec<Violation> {
    let private: Vec<String> = private.iter().map(|n| (*n).to_owned()).collect();
    check_gh(&parse_gh(&args(line)).unwrap(), rules, &private, machine)
}

#[test]
fn a_dashed_commit_message_is_refused_and_a_clean_one_is_not() {
    let rules = Rules::default();
    let mut machine = Fake::default();
    assert_eq!(
        git(&["commit", "-m", "fix: a \u{2014} b"], rules, &[], &mut machine),
        [Rule::Dashes]
    );
    assert_eq!(git(&["commit", "-m", "fix: a - b"], rules, &[], &mut machine), []);
}

#[test]
fn each_text_of_a_commit_is_scanned_and_each_rule_reports_once() {
    let rules = Rules::default();
    let mut machine = Fake::default();
    let found = git(
        &[
            "commit",
            "-m",
            "ok",
            "-m",
            "x \u{2013} y",
            "-m",
            "z \u{2014} w",
            "-m",
            "Co-Authored-By: A <a@b.c>",
        ],
        rules,
        &[],
        &mut machine,
    );
    assert_eq!(found, [Rule::Dashes, Rule::Trailers]);
}

#[test]
fn a_message_file_is_read_relative_to_the_dash_c_directory() {
    let mut machine = Fake::default();
    machine.files.insert("/repo/msg.txt".into(), "a \u{2014} b".into());
    assert_eq!(
        git(&["-C", "/repo", "commit", "-F", "msg.txt"], Rules::default(), &[], &mut machine),
        [Rule::Dashes]
    );
    assert_eq!(
        git(&["-C", "/elsewhere", "commit", "-F", "msg.txt"], Rules::default(), &[], &mut machine),
        []
    );
}

#[test]
fn stdin_messages_are_scanned() {
    let mut machine = Fake::default();
    machine.files.insert("-".into(), "Generated with a tool".into());
    assert_eq!(git(&["commit", "-F", "-"], Rules::default(), &[], &mut machine), [Rule::Trailers]);
}

#[test]
fn a_rule_that_is_off_is_skipped() {
    let rules = Rules { dashes: false, trailers: false, ..Rules::default() };
    let mut machine = Fake::default();
    assert_eq!(
        git(&["commit", "-m", "a \u{2014} b\n\nCo-Authored-By: A"], rules, &[], &mut machine),
        []
    );
}

#[test]
fn github_text_is_scanned_in_titles_and_bodies() {
    let rules = Rules::default();
    let mut machine = Fake::default();
    let titled =
        gh(&["pr", "create", "--title", "a \u{2014} b", "--body", "ok"], rules, &[], &mut machine);
    assert_eq!(titled.iter().map(|v| v.rule).collect::<Vec<_>>(), [Rule::Dashes]);
    assert!(titled[0].detail.contains("the title"), "{}", titled[0].detail);
    let footer = gh(
        &["pr", "comment", "1", "-b", "done\n\n\u{1F916} Generated with Claude"],
        rules,
        &[],
        &mut machine,
    );
    assert_eq!(footer.iter().map(|v| v.rule).collect::<Vec<_>>(), [Rule::Trailers]);
    assert!(footer[0].detail.contains("line 3"), "{}", footer[0].detail);
    assert_eq!(gh(&["issue", "create", "-t", "ok", "-b", "fine"], rules, &[], &mut machine), []);
}

#[test]
fn private_names_block_only_in_a_public_repo() {
    let rules = Rules::default();
    let text = ["pr", "comment", "1", "-R", "acme/web", "-b", "see acme/vault#3"];
    let mut public = Fake { public: Some(true), ..Fake::default() };
    let found = gh(&text, rules, &["acme/vault"], &mut public);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].rule, Rule::PrivateNames);
    assert_eq!(public.asked, [(None, Some("acme/web".to_owned()))]);

    let mut private = Fake { public: Some(false), ..Fake::default() };
    assert_eq!(gh(&text, rules, &["acme/vault"], &mut private), []);
    let mut unknown = Fake::default();
    assert_eq!(gh(&text, rules, &["acme/vault"], &mut unknown), []);
}

#[test]
fn visibility_is_not_asked_when_nothing_matches_or_the_list_is_empty_or_the_rule_is_off() {
    let mut machine = Fake { public: Some(true), ..Fake::default() };
    let text = ["pr", "comment", "1", "-b", "acme/vault"];
    assert_eq!(gh(&text, Rules::default(), &[], &mut machine), []);
    assert_eq!(
        gh(&["pr", "comment", "1", "-b", "clean"], Rules::default(), &["acme/vault"], &mut machine),
        []
    );
    let off = Rules { private_names: false, ..Rules::default() };
    assert_eq!(gh(&text, off, &["acme/vault"], &mut machine), []);
    assert_eq!(machine.asked, []);
}

#[test]
fn a_commit_asks_about_the_repo_in_its_directory() {
    let mut machine = Fake { public: Some(true), ..Fake::default() };
    let found = git(
        &["-C", "/repo", "commit", "-m", "mentions ledger-api"],
        Rules::default(),
        &["ledger-api"],
        &mut machine,
    );
    assert_eq!(found, [Rule::PrivateNames]);
    assert_eq!(machine.asked, [(Some(PathBuf::from("/repo")), None)]);
}

#[test]
fn drafts_are_required_only_when_the_rule_is_on() {
    let on = Rules { draft_prs: true, ..Rules::default() };
    let mut machine = Fake::default();
    let missing = gh(&["pr", "create", "--title", "t"], on, &[], &mut machine);
    assert_eq!(missing.iter().map(|v| v.rule).collect::<Vec<_>>(), [Rule::DraftPrs]);
    assert_eq!(gh(&["pr", "create", "--draft"], on, &[], &mut machine), []);
    assert_eq!(gh(&["pr", "create", "-d"], on, &[], &mut machine), []);
    assert_eq!(gh(&["pr", "create"], Rules::default(), &[], &mut machine), []);
    assert_eq!(gh(&["pr", "edit", "1", "--title", "t"], on, &[], &mut machine), []);
}

#[test]
fn a_whole_run_rerun_is_refused_and_a_failed_one_is_not() {
    let mut machine = Fake::default();
    let rules = Rules::default();
    let refused = gh(&["run", "rerun", "9"], rules, &[], &mut machine);
    assert_eq!(refused.iter().map(|v| v.rule).collect::<Vec<_>>(), [Rule::RerunFailed]);
    assert_eq!(gh(&["run", "rerun", "9", "--failed"], rules, &[], &mut machine), []);
    assert_eq!(gh(&["run", "rerun", "9", "-j", "4"], rules, &[], &mut machine), []);
    let off = Rules { rerun_failed: false, ..Rules::default() };
    assert_eq!(gh(&["run", "rerun", "9"], off, &[], &mut machine), []);
}

#[test]
fn the_message_names_the_rule_the_reason_and_the_place() {
    let mut machine = Fake::default();
    let found = gh(&["pr", "create", "-t", "a \u{2014} b"], Rules::default(), &[], &mut machine);
    let line = found[0].message();
    assert!(line.starts_with("tidewatch guard [dashes]: "), "{line}");
    assert!(line.contains(Rule::Dashes.reason()), "{line}");
    assert!(line.contains("the title has an em dash at line 1, column 3"), "{line}");
    assert!(!line.contains('\u{2014}') && !line.contains('\u{2013}'), "{line}");
    assert!(!line.contains('\n'));
    let bare = Violation { rule: Rule::DraftPrs, detail: String::new() }.message();
    assert!(bare.contains("gh pr create. Fix it"), "{bare}");
}

#[test]
fn remote_urls_and_repo_flags_become_a_slug() {
    let remotes = [
        ("git@github.com:acme/web.git", Some("acme/web")),
        ("https://github.com/acme/web", Some("acme/web")),
        ("https://github.com/acme/web.git\n", Some("acme/web")),
        ("https://user:tok@github.com/acme/web.git", Some("acme/web")),
        ("ssh://git@github.com/acme/web.git", Some("acme/web")),
        ("git@gitlab.com:acme/web.git", None),
        ("https://github.com/acme", None),
        ("https://github.com/acme/web/extra", None),
        ("/local/bare/repo.git", None),
        ("", None),
    ];
    for (url, want) in remotes {
        assert_eq!(slug_from_remote(url).as_deref(), want, "{url}");
    }
    let flags = [
        ("acme/web", Some("acme/web")),
        ("github.com/acme/web", Some("acme/web")),
        ("https://github.com/acme/web", Some("acme/web")),
        ("web", None),
        ("", None),
    ];
    for (flag, want) in flags {
        assert_eq!(slug_from_repo_flag(flag).as_deref(), want, "{flag}");
    }
}

#[test]
fn visibility_words_are_public_or_not() {
    assert_eq!(parse_visibility("PUBLIC\n"), Some(true));
    assert_eq!(parse_visibility("public"), Some(true));
    assert_eq!(parse_visibility("PRIVATE"), Some(false));
    assert_eq!(parse_visibility("INTERNAL"), Some(false));
    assert_eq!(parse_visibility(""), None);
    assert_eq!(parse_visibility("HTTP 404"), None);
}

fn tool_in(dir: &Path, name: &str) {
    let file = dir.join(name);
    std::fs::write(&file, "#!/bin/sh\n").unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn the_real_tool_is_the_first_one_that_is_not_in_the_shim_directory() {
    let root = tempfile::tempdir().unwrap();
    let (shims, first, second) =
        (root.path().join("shims"), root.path().join("first"), root.path().join("second"));
    for dir in [&shims, &first, &second] {
        std::fs::create_dir(dir).unwrap();
        tool_in(dir, "git");
    }
    tool_in(&second, "gh");
    let path = std::env::join_paths([&shims, &first, &second]).unwrap();
    assert_eq!(resolve_on_path("git", &path, &shims), Some(first.join("git")));
    assert_eq!(resolve_on_path("gh", &path, &shims), Some(second.join("gh")));
    assert_eq!(resolve_on_path("hub", &path, &shims), None);
}

#[test]
fn resolution_skips_the_shim_directory_however_it_is_spelled() {
    let root = tempfile::tempdir().unwrap();
    let (shims, real) = (root.path().join("shims"), root.path().join("real"));
    std::fs::create_dir(&shims).unwrap();
    std::fs::create_dir(&real).unwrap();
    tool_in(&shims, "git");
    tool_in(&real, "git");
    std::os::unix::fs::symlink(&shims, root.path().join("alias")).unwrap();
    let spelled = [
        root.path().join("alias"),
        PathBuf::from(format!("{}/", shims.display())),
        root.path().join("real/../shims"),
        PathBuf::new(),
        real.clone(),
    ];
    let path = std::env::join_paths(spelled).unwrap();
    assert_eq!(resolve_on_path("git", &path, &shims), Some(real.join("git")));
}

#[test]
fn a_file_that_is_not_executable_or_is_a_directory_is_not_the_tool() {
    let root = tempfile::tempdir().unwrap();
    let (plain, dirs, real) =
        (root.path().join("plain"), root.path().join("dirs"), root.path().join("real"));
    for dir in [&plain, &dirs, &real] {
        std::fs::create_dir(dir).unwrap();
    }
    std::fs::write(plain.join("git"), "x").unwrap();
    std::fs::create_dir(dirs.join("git")).unwrap();
    tool_in(&real, "git");
    let path = std::env::join_paths([&plain, &dirs, &real]).unwrap();
    assert_eq!(resolve_on_path("git", &path, Path::new("/nowhere")), Some(real.join("git")));
}

#[test]
fn the_guard_wakes_only_with_a_settings_file_in_the_environment() {
    let env: HashMap<&str, &str> = HashMap::from([
        (CONFIG_ENV, "/cfg/settings.toml"),
        (REPO_ENV, "/work/web"),
        (CACHE_ENV, "/state/guard-cache"),
        (DIR_ENV, "/state/guard-bin"),
        ("PATH", "/state/guard-bin:/usr/bin"),
    ]);
    let get = |key: &str| env.get(key).map(OsString::from);
    let guard = GuardEnv::from_env(get).unwrap();
    assert_eq!(guard.config, PathBuf::from("/cfg/settings.toml"));
    assert_eq!(guard.repo, PathBuf::from("/work/web"));
    assert_eq!(guard.cache, Some(PathBuf::from("/state/guard-cache")));
    assert_eq!(guard.shim_dir, PathBuf::from("/state/guard-bin"));
    assert_eq!(GuardEnv::from_env(|_| None), None);
    assert_eq!(GuardEnv::from_env(|key| (key == CONFIG_ENV).then(OsString::new)), None);
}

#[test]
fn tools_are_named_git_and_gh_only() {
    assert_eq!(Tool::parse("git"), Some(Tool::Git));
    assert_eq!(Tool::parse("gh"), Some(Tool::Gh));
    assert_eq!(Tool::parse("hub"), None);
    assert_eq!(Tool::parse(""), None);
}
