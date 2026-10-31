use super::*;

fn owner(text: &str) -> Document {
    Document { label: "~/.claude/CLAUDE.md".into(), scope: Scope::Owner, text: text.into() }
}

fn repo(label: &str, text: &str) -> Document {
    Document { label: label.into(), scope: Scope::Repo, text: text.into() }
}

const OWNER_RULES: &str = "\
## PR review replies

- **Never add standalone \"address review\" commits.** `git commit --fixup=<sha>`
  onto the commit the change belongs to, then
  `GIT_SEQUENCE_EDITOR=true git rebase --autosquash origin/main` and push with
  `--force-with-lease`.
";

const REPO_RULES: &str = "\
- Conventional Commits, clean author line. Commit or push only when asked.
- Review feedback is folded into the commit it belongs to (`git commit --fixup=<sha>`, then a
  non-interactive autosquash), never a standalone \"address review\" commit. Push with
  `--force-with-lease`.
- Interactive flags (`-i`, e.g. `git rebase -i`, `git add -i`) are not supported in this environment.
";

#[test]
fn the_owners_real_rules_give_rebase_and_fixup_with_their_evidence() {
    let found = detect(&[owner(OWNER_RULES)], false);
    assert_eq!(found.workflow(), Workflow::default());
    assert_eq!(
        found.catch_up.source.line(),
        "found: \"rebase --autosquash origin/main\" in ~/.claude/CLAUDE.md"
    );
    assert_eq!(
        found.fixes.source,
        Source::Found { quote: "--fixup=<sha>".into(), file: "~/.claude/CLAUDE.md".into() }
    );
}

#[test]
fn the_repo_rule_files_are_evidence_too_and_interactive_rebase_is_not_a_rule() {
    let docs = [repo(".agents/rules/workflow.md", REPO_RULES)];
    let found = detect(&docs, false);
    assert_eq!(found.fixes.value, Fixes::Fixup);
    assert!(
        matches!(&found.fixes.source, Source::Found { file, .. } if file == ".agents/rules/workflow.md")
    );
    assert!(matches!(found.catch_up.source, Source::Found { .. }), "the force-with-lease hint");
    assert_eq!(found.catch_up.value, CatchUp::Rebase);
}

#[test]
fn the_message_that_caused_the_incident_reads_as_merge() {
    let text = "Fetch, merge origin/main into this branch, resolve the conflicts. Do not rebase or force-push.";
    let found = detect(&[repo("CLAUDE.md", text)], false);
    assert_eq!(found.catch_up.value, CatchUp::Merge);
    assert_eq!(found.catch_up.source.line(), "found: \"Do not rebase or force-push\" in CLAUDE.md");
}

#[test]
fn a_repo_line_that_names_the_method_beats_the_owners_file() {
    let docs = [
        owner("Rebase onto origin/main before you push."),
        repo("AGENTS.md", "Catch up with `git merge origin/main`; history stays as it happened."),
    ];
    let found = detect(&docs, false);
    assert_eq!(found.catch_up.value, CatchUp::Merge);
    assert_eq!(
        found.catch_up.source,
        Source::Found { quote: "merge origin/main".into(), file: "AGENTS.md".into() }
    );
}

#[test]
fn a_repo_hint_never_beats_an_owner_line_that_names_the_method() {
    let docs = [
        repo("AGENTS.md", "Never force-push shared branches."),
        owner("Rebase onto origin/main before you push."),
    ];
    let found = detect(&docs, false);
    assert_eq!(found.catch_up.value, CatchUp::Rebase);
    assert!(
        matches!(&found.catch_up.source, Source::Found { file, .. } if file == "~/.claude/CLAUDE.md")
    );
}

#[test]
fn a_repo_hint_beats_an_owner_hint_and_stands_alone_when_nothing_else_speaks() {
    let docs =
        [owner("Prefer a linear history."), repo("AGENTS.md", "Never force-push shared branches.")];
    assert_eq!(detect(&docs, false).catch_up.value, CatchUp::Merge);
    assert_eq!(detect(&docs[..1], false).catch_up.value, CatchUp::Rebase);
}

#[test]
fn equal_evidence_in_two_files_goes_to_the_first_file() {
    let docs = [
        repo("CLAUDE.md", "Merge origin/main into your branch."),
        repo("AGENTS.md", "Rebase onto origin/main."),
    ];
    assert_eq!(detect(&docs, false).catch_up.value, CatchUp::Merge);
    let reversed = [docs[1].clone(), docs[0].clone()];
    assert_eq!(detect(&reversed, false).catch_up.value, CatchUp::Rebase);
}

#[test]
fn catch_up_phrases_table() {
    let cases = [
        ("Always rebase onto origin/main.", Some(CatchUp::Rebase)),
        ("git rebase --onto origin/main sha branch", Some(CatchUp::Rebase)),
        ("Never rebase a pushed branch.", Some(CatchUp::Merge)),
        ("Don't rebase; merge instead.", Some(CatchUp::Merge)),
        ("Merge main into your branch to catch up.", Some(CatchUp::Merge)),
        ("git merge --no-edit origin/main", Some(CatchUp::Merge)),
        ("Never merge main into a feature branch, rebase.", Some(CatchUp::Rebase)),
        ("Do not use interactive rebase.", None),
        ("squash and merge the PR into main", None),
        ("Merge the pull request when checks pass.", None),
        ("keep the maintenance window short, merge maintenance fixes", None),
        ("Use a linear history.", Some(CatchUp::Rebase)),
        ("No force pushes.", Some(CatchUp::Merge)),
    ];
    for (line, want) in cases {
        let found = detect(&[repo("CLAUDE.md", line)], false);
        let got =
            matches!(found.catch_up.source, Source::Found { .. }).then_some(found.catch_up.value);
        assert_eq!(got, want, "{line:?}");
    }
}

#[test]
fn fixes_phrases_table() {
    let cases = [
        ("Use `git commit --fixup=<sha>` for review fixes.", Some(Fixes::Fixup)),
        ("Fold each fix into the commit it belongs to.", Some(Fixes::Fixup)),
        ("Review feedback is folded into its commit.", Some(Fixes::Fixup)),
        ("Never add standalone commits for a review.", Some(Fixes::Fixup)),
        ("Do not use fixup commits.", Some(Fixes::NewCommits)),
        ("Never use --fixup, push new commits for each review fix.", Some(Fixes::NewCommits)),
        ("Address review feedback in a new commit.", Some(Fixes::NewCommits)),
        ("Make a standalone commit per fix.", Some(Fixes::NewCommits)),
        ("Amend the last commit when you fix a typo.", Some(Fixes::Fixup)),
        ("Never amend pushed commits.", Some(Fixes::NewCommits)),
        ("Commit often.", None),
    ];
    for (line, want) in cases {
        let found = detect(&[repo("CLAUDE.md", line)], false);
        let got = matches!(found.fixes.source, Source::Found { .. }).then_some(found.fixes.value);
        assert_eq!(got, want, "{line:?}");
    }
}

#[test]
fn pull_request_phrases_table() {
    let cases = [
        ("Open a draft pull request.", Some(PullRequests::Draft)),
        ("Run `gh pr create --draft --base main`.", Some(PullRequests::Draft)),
        ("Keep the PR a draft through every fix round.", Some(PullRequests::Draft)),
        ("Open pull requests as drafts.", Some(PullRequests::Draft)),
        ("Never open a PR as a draft.", Some(PullRequests::Ready)),
        ("Open non-draft pull requests.", Some(PullRequests::Ready)),
        ("Create the PR without --draft.", Some(PullRequests::Ready)),
        ("Do not open drafts for pull requests.", Some(PullRequests::Ready)),
        ("Mark it ready for review when you are happy with it.", None),
        ("Draft a plan before you start.", None),
        ("Write drafts of the release notes.", None),
    ];
    for (line, want) in cases {
        let found = detect(&[repo("CLAUDE.md", line)], false);
        let got = matches!(found.pull_requests.source, Source::Found { .. })
            .then_some(found.pull_requests.value);
        assert_eq!(got, want, "{line:?}");
    }
}

#[test]
fn the_draft_guard_decides_the_pull_request_whatever_the_text_says() {
    let docs = [repo("CLAUDE.md", "Never open a PR as a draft.")];
    let found = detect(&docs, true);
    assert_eq!(found.pull_requests.value, PullRequests::Draft);
    assert_eq!(found.pull_requests.source.line(), "found: the Draft PRs guard is on for this repo");
}

#[test]
fn with_nothing_found_the_defaults_say_so() {
    let found = detect(&[], false);
    assert_eq!(found.workflow(), Workflow::default());
    for line in
        [found.catch_up.source.line(), found.fixes.source.line(), found.pull_requests.source.line()]
    {
        assert!(line.starts_with("nothing found: "), "{line}");
    }
}

#[test]
fn a_quote_is_cut_to_a_short_readable_stretch() {
    let long = format!(
        "{} then merge origin/main into your branch {}",
        "word ".repeat(30),
        "tail ".repeat(40)
    );
    let found = detect(&[repo("CLAUDE.md", &long)], false);
    let Source::Found { quote, .. } = found.catch_up.source else { panic!("no evidence") };
    assert!(quote.starts_with("merge origin/main") && quote.ends_with("..."), "{quote}");
    assert!(quote.chars().count() < 100, "{quote}");
}

#[test]
fn files_are_gathered_in_reading_order_with_short_labels() {
    let home = tempfile::tempdir().unwrap();
    let paths = TidewatchPaths::new(home.path());
    let repo_dir = home.path().join("web");
    std::fs::create_dir_all(repo_dir.join(".agents/rules")).unwrap();
    std::fs::write(repo_dir.join("CLAUDE.md"), "claude").unwrap();
    std::fs::write(repo_dir.join(".agents/rules/b.md"), "b").unwrap();
    std::fs::write(repo_dir.join(".agents/rules/a.md"), "a").unwrap();
    std::fs::write(repo_dir.join(".agents/rules/notes.txt"), "skipped").unwrap();
    std::fs::create_dir_all(paths.claude_dir()).unwrap();
    std::fs::write(paths.claude_dir().join("CLAUDE.md"), "owner").unwrap();
    let labels: Vec<(String, Scope)> =
        gather(&paths, &repo_dir).into_iter().map(|d| (d.label, d.scope)).collect();
    assert_eq!(
        labels,
        [
            ("CLAUDE.md".to_owned(), Scope::Repo),
            (".agents/rules/a.md".to_owned(), Scope::Repo),
            (".agents/rules/b.md".to_owned(), Scope::Repo),
            ("~/.claude/CLAUDE.md".to_owned(), Scope::Owner),
        ]
    );
}

#[test]
fn a_repo_with_no_rule_files_and_no_owner_file_proposes_the_defaults() {
    let home = tempfile::tempdir().unwrap();
    let paths = TidewatchPaths::new(home.path());
    let repo_dir = home.path().join("empty");
    std::fs::create_dir_all(&repo_dir).unwrap();
    assert_eq!(detect_repo(&paths, &repo_dir).workflow(), Workflow::default());
}

#[test]
fn the_launch_profile_is_the_confirmed_one_else_what_detection_proposes() {
    let home = tempfile::tempdir().unwrap();
    let paths = TidewatchPaths::new(home.path());
    let repo_dir = home.path().join("web");
    std::fs::create_dir_all(&repo_dir).unwrap();
    std::fs::write(repo_dir.join("CLAUDE.md"), "Merge origin/main into your branch.\n").unwrap();
    assert_eq!(profile_for(&paths, &repo_dir).catch_up, CatchUp::Merge);
    let confirmed = Workflow { catch_up: CatchUp::Rebase, ..Workflow::default() };
    Store::update(&paths, |store| store.set(&repo_dir, confirmed)).unwrap();
    assert_eq!(profile_for(&paths, &repo_dir), confirmed);
}
