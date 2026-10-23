use super::*;

fn words(line: &str) -> Vec<String> {
    line.split_whitespace().map(str::to_owned).collect()
}

fn inline(part: Part, text: &str) -> TextArg {
    TextArg { part, source: Source::Inline(text.to_owned()) }
}

fn file(part: Part, path: &str) -> TextArg {
    TextArg { part, source: Source::File(PathBuf::from(path)) }
}

fn git_texts(args: &[&str]) -> Vec<TextArg> {
    let args: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
    parse_git(&args).expect("a commit").texts
}

#[test]
fn a_git_command_that_is_not_a_commit_is_not_looked_at() {
    for line in ["status", "push origin main", "log -m", "-C /repo diff", "", "-c a=b", "commi"] {
        assert_eq!(parse_git(&words(line)), None, "{line}");
    }
}

#[test]
fn commit_messages_come_from_every_spelling_of_m() {
    let msg = |text: &str| vec![inline(Part::Message, text)];
    assert_eq!(git_texts(&["commit", "-m", "one"]), msg("one"));
    assert_eq!(git_texts(&["commit", "-mone"]), msg("one"));
    assert_eq!(git_texts(&["commit", "--message", "one"]), msg("one"));
    assert_eq!(git_texts(&["commit", "--message=one"]), msg("one"));
    assert_eq!(git_texts(&["commit", "-am", "one"]), msg("one"));
    assert_eq!(git_texts(&["commit", "-a", "-m", "one"]), msg("one"));
    assert_eq!(git_texts(&["commit", "-qm", "one"]), msg("one"));
    assert_eq!(
        git_texts(&["commit", "-m", "title", "-m", "body"]),
        vec![inline(Part::Message, "title"), inline(Part::Message, "body")]
    );
    assert_eq!(git_texts(&["commit", "--amend", "--no-edit"]), vec![]);
}

#[test]
fn message_files_and_stdin_are_sources_and_trailer_flags_are_text() {
    assert_eq!(git_texts(&["commit", "-F", "msg.txt"]), vec![file(Part::Message, "msg.txt")]);
    assert_eq!(git_texts(&["commit", "--file=msg.txt"]), vec![file(Part::Message, "msg.txt")]);
    assert_eq!(
        git_texts(&["commit", "-F", "-"]),
        vec![TextArg { part: Part::Message, source: Source::Stdin }]
    );
    assert_eq!(
        git_texts(&["commit", "--trailer", "Co-authored-by: X", "-m", "m"]),
        vec![inline(Part::Message, "Co-authored-by: X"), inline(Part::Message, "m")]
    );
}

#[test]
fn flags_that_reuse_a_message_or_take_a_value_never_become_text() {
    assert_eq!(git_texts(&["commit", "-C", "HEAD~1"]), vec![]);
    assert_eq!(
        git_texts(&["commit", "-c", "abc", "-t", "tpl", "-S", "--author", "A <a@b.c>"]),
        vec![]
    );
    assert_eq!(git_texts(&["commit", "--fixup=abc", "--", "-m", "file"]), vec![]);
}

#[test]
fn global_options_before_commit_are_skipped_and_the_directory_follows_dash_c() {
    let args: Vec<String> =
        ["-C", "/repo", "-c", "user.name=a", "--no-pager", "-C", "sub", "commit", "-m", "x"]
            .iter()
            .map(|a| (*a).to_owned())
            .collect();
    let commit = parse_git(&args).unwrap();
    assert_eq!(commit.dir, Some(PathBuf::from("/repo/sub")));
    assert_eq!(commit.texts, vec![inline(Part::Message, "x")]);
    let split = parse_git(&words("--git-dir /x/.git commit -m y")).unwrap();
    assert_eq!(split.texts, vec![inline(Part::Message, "y")]);
}

fn gh(line: &[&str]) -> GhCall {
    let args: Vec<String> = line.iter().map(|a| (*a).to_owned()).collect();
    parse_gh(&args).expect("a guarded gh command")
}

#[test]
fn only_the_guarded_gh_commands_are_parsed() {
    for line in [
        "pr view 1",
        "pr list",
        "issue list",
        "issue close 3",
        "run view 9",
        "repo view",
        "api repos/x/y",
        "pr",
        "",
        "-R a/b pr create",
    ] {
        assert_eq!(parse_gh(&words(line)), None, "{line}");
    }
    let actions = [
        ("pr create", GhAction::PrCreate),
        ("pr edit 1", GhAction::PrEdit),
        ("pr comment 1", GhAction::PrComment),
        ("pr review 1", GhAction::PrReview),
        ("issue create", GhAction::IssueCreate),
        ("issue comment 2", GhAction::IssueComment),
        ("issue edit 2", GhAction::IssueEdit),
        ("run rerun 9", GhAction::RunRerun),
    ];
    for (line, want) in actions {
        assert_eq!(parse_gh(&words(line)).map(|c| c.action), Some(want), "{line}");
    }
}

#[test]
fn title_and_body_are_read_in_every_spelling() {
    let call = gh(&["pr", "create", "--title", "T", "--body=B", "-t", "T2", "-bB2", "-F", "b.md"]);
    assert_eq!(
        call.texts,
        vec![
            inline(Part::Title, "T"),
            inline(Part::Body, "B"),
            inline(Part::Title, "T2"),
            inline(Part::Body, "B2"),
            file(Part::Body, "b.md"),
        ]
    );
    let long = gh(&["issue", "create", "--body-file", "b.md", "--body-file=-"]);
    assert_eq!(
        long.texts,
        vec![file(Part::Body, "b.md"), TextArg { part: Part::Body, source: Source::Stdin }]
    );
    assert_eq!(gh(&["pr", "comment", "7", "-b", "hi"]).texts, vec![inline(Part::Body, "hi")]);
    assert_eq!(gh(&["pr", "review", "7", "-r", "-b", "no"]).texts, vec![inline(Part::Body, "no")]);
    assert_eq!(gh(&["issue", "edit", "7", "-t", "x"]).texts, vec![inline(Part::Title, "x")]);
}

#[test]
fn a_short_flag_cluster_ends_at_the_first_flag_that_takes_a_value() {
    let call = gh(&["pr", "create", "-dt", "Title"]);
    assert!(call.draft);
    assert_eq!(call.texts, vec![inline(Part::Title, "Title")]);
    let attached = gh(&["pr", "create", "-wtTitle"]);
    assert_eq!(attached.texts, vec![inline(Part::Title, "Title")]);
    assert!(!attached.draft);
}

#[test]
fn a_flag_value_that_looks_like_a_flag_is_still_its_value() {
    let call = gh(&["pr", "create", "--title", "-d", "--label", "-b"]);
    assert_eq!(call.texts, vec![inline(Part::Title, "-d")]);
    assert!(!call.draft);
}

#[test]
fn draft_is_set_by_the_flag_in_either_spelling_and_cleared_by_false() {
    assert!(gh(&["pr", "create", "--draft"]).draft);
    assert!(gh(&["pr", "create", "-d"]).draft);
    assert!(gh(&["pr", "create", "--draft=true"]).draft);
    assert!(!gh(&["pr", "create", "--draft=false"]).draft);
    assert!(!gh(&["pr", "create", "--fill"]).draft);
    assert!(!gh(&["run", "rerun", "9", "-d"]).draft);
}

#[test]
fn the_repo_is_read_from_r_or_repo() {
    assert_eq!(gh(&["pr", "create", "-R", "acme/web"]).repo.as_deref(), Some("acme/web"));
    assert_eq!(gh(&["pr", "create", "--repo", "acme/web"]).repo.as_deref(), Some("acme/web"));
    assert_eq!(gh(&["pr", "comment", "1", "--repo=acme/web"]).repo.as_deref(), Some("acme/web"));
    assert_eq!(gh(&["pr", "comment", "1", "-Racme/web"]).repo.as_deref(), Some("acme/web"));
    assert_eq!(gh(&["pr", "comment", "1"]).repo, None);
}

#[test]
fn a_rerun_is_narrow_with_failed_or_a_job() {
    assert!(!gh(&["run", "rerun", "9"]).narrow_rerun);
    assert!(!gh(&["run", "rerun", "9", "--debug"]).narrow_rerun);
    assert!(gh(&["run", "rerun", "9", "--failed"]).narrow_rerun);
    assert!(gh(&["run", "rerun", "9", "--job", "5"]).narrow_rerun);
    assert!(gh(&["run", "rerun", "9", "-j", "5"]).narrow_rerun);
    assert!(!gh(&["run", "rerun", "9", "--failed=false"]).narrow_rerun);
}
