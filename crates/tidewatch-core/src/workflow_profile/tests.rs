use super::*;

fn paths() -> (tempfile::TempDir, TidewatchPaths) {
    let home = tempfile::tempdir().unwrap();
    let paths = TidewatchPaths::new(home.path());
    (home, paths)
}

fn profile(catch_up: CatchUp, fixes: Fixes, pull_requests: PullRequests) -> Workflow {
    Workflow { catch_up, fixes, pull_requests, checks: Checks::default() }
}

#[test]
fn the_defaults_are_rebase_fixup_and_draft() {
    assert_eq!(Workflow::default(), profile(CatchUp::Rebase, Fixes::Fixup, PullRequests::Draft));
}

#[test]
fn the_catch_up_words_follow_the_profile() {
    let rebase = Workflow::default();
    let merge = Workflow { catch_up: CatchUp::Merge, ..rebase };
    assert_eq!(rebase.catch_up_clause("main"), "Rebase onto origin/main");
    assert_eq!(rebase.push_after_catch_up(), "push with --force-with-lease");
    assert_eq!(merge.catch_up_clause("develop"), "Merge origin/develop into this branch");
    assert_eq!(merge.push_after_catch_up(), "push");
}

#[test]
fn the_fix_words_follow_the_profile() {
    let fixup = Workflow::default();
    let new = Workflow { fixes: Fixes::NewCommits, ..fixup };
    let table = [
        (
            fixup,
            Fix::The,
            "fold the fix into the commit it belongs to and push with --force-with-lease",
        ),
        (fixup, Fix::Each, "fold each fix into its commit and push with --force-with-lease"),
        (fixup, Fix::It, "fold it into its commit and push with --force-with-lease"),
        (new, Fix::The, "commit the fix and push"),
        (new, Fix::Each, "commit each fix and push"),
        (new, Fix::It, "commit it and push"),
    ];
    for (workflow, fix, want) in table {
        assert_eq!(workflow.fix_and_push(fix), want, "{workflow:?} {fix:?}");
    }
}

#[test]
fn each_fix_follows_the_fixes_setting() {
    let fixup = Workflow::default();
    let new = Workflow { fixes: Fixes::NewCommits, ..fixup };
    assert_eq!(fixup.each_fix(), "fold each fix into its commit");
    assert_eq!(new.each_fix(), "commit each fix");
}

#[test]
fn unpushed_commits_need_the_lease_when_either_way_rewrites() {
    let table = [
        (CatchUp::Rebase, Fixes::Fixup, true),
        (CatchUp::Rebase, Fixes::NewCommits, true),
        (CatchUp::Merge, Fixes::Fixup, true),
        (CatchUp::Merge, Fixes::NewCommits, false),
    ];
    for (catch_up, fixes, want) in table {
        let workflow = profile(catch_up, fixes, PullRequests::Draft);
        assert_eq!(workflow.pushes_unpushed_with_lease(), want, "{catch_up:?} {fixes:?}");
    }
}

#[test]
fn a_new_pull_request_is_a_draft_or_says_it_is_not() {
    let draft = Workflow::default();
    let ready = Workflow { pull_requests: PullRequests::Ready, ..draft };
    assert_eq!(
        (draft.new_pull_request(), draft.draft_flag()),
        ("a draft pull request", " --draft")
    );
    assert_eq!(
        (ready.new_pull_request(), ready.draft_flag()),
        ("a pull request (not a draft)", "")
    );
}

#[test]
fn a_saved_profile_comes_back_for_its_repo_only() {
    let (_home, paths) = paths();
    let repo = Path::new("/work/web");
    let merge = profile(CatchUp::Merge, Fixes::NewCommits, PullRequests::Ready);
    assert_eq!(Store::load(&paths).get(repo), None);
    Store::update(&paths, |store| store.set(repo, merge)).unwrap();
    Store::update(&paths, |store| store.set(Path::new("/work/api"), Workflow::default())).unwrap();
    let store = Store::load(&paths);
    assert_eq!(store.get(repo), Some(merge));
    assert_eq!(store.get(Path::new("/work/api")), Some(Workflow::default()));
    assert_eq!(store.get(Path::new("/work/other")), None);
    assert_eq!(
        store.get(Path::new("/work/web/")),
        Some(merge),
        "a trailing slash is the same repo"
    );
}

#[test]
fn the_file_is_plain_words_keyed_by_repo() {
    let (_home, paths) = paths();
    let merge = profile(CatchUp::Merge, Fixes::NewCommits, PullRequests::Ready);
    Store::update(&paths, |store| store.set(Path::new("/work/web"), merge)).unwrap();
    let text = std::fs::read_to_string(paths.workflow_file()).unwrap();
    assert!(text.contains("[repo.\"/work/web\"]"), "{text}");
    assert!(text.contains("catch_up = \"merge\""), "{text}");
    assert!(text.contains("fixes = \"new-commits\""), "{text}");
    assert!(text.contains("pull_requests = \"ready\""), "{text}");
}

#[test]
fn a_missing_field_reads_as_its_default_and_a_broken_file_as_empty() {
    let (_home, paths) = paths();
    std::fs::create_dir_all(paths.config_dir()).unwrap();
    std::fs::write(paths.workflow_file(), "[repo.\"/a\"]\ncatch_up = \"merge\"\n").unwrap();
    let wanted = Workflow { catch_up: CatchUp::Merge, ..Workflow::default() };
    assert_eq!(Store::load(&paths).get(Path::new("/a")), Some(wanted));
    std::fs::write(paths.workflow_file(), "[repo.\"/a\"]\ncatch_up = \"squash\"\n").unwrap();
    assert_eq!(Store::load(&paths), Store::default());
    std::fs::write(paths.workflow_file(), "not toml [[[").unwrap();
    assert_eq!(Store::load(&paths), Store::default());
}

#[test]
fn the_launch_lines_follow_every_setting() {
    let table = [
        (
            Workflow::default(),
            [
                "Branches catch up by rebasing onto the default branch.",
                "Fixes fold into the commit they belong to: `git commit --fixup`, then autosquash.",
                "Push a rewritten branch with `--force-with-lease`.",
                "New pull requests open as drafts (`gh pr create --draft`).",
            ],
        ),
        (
            profile(CatchUp::Merge, Fixes::NewCommits, PullRequests::Ready),
            [
                "Branches catch up by merging the default branch into them.",
                "Fixes go in new commits.",
                "Push without force.",
                "New pull requests open ready for review (`gh pr create`, no `--draft`).",
            ],
        ),
        (
            profile(CatchUp::Merge, Fixes::Fixup, PullRequests::Draft),
            [
                "Branches catch up by merging the default branch into them.",
                "Fixes fold into the commit they belong to: `git commit --fixup`, then autosquash.",
                "Push a rewritten branch with `--force-with-lease`.",
                "New pull requests open as drafts (`gh pr create --draft`).",
            ],
        ),
    ];
    for (workflow, want) in table {
        assert_eq!(workflow.launch_lines(), want, "{workflow:?}");
    }
}

#[test]
fn tests_verified_and_scope_are_on_and_the_other_two_checks_off() {
    assert_eq!(
        Workflow::default().checks,
        Checks { tests: true, verified: true, scope: true, fix_test: false, reviewer: false }
    );
}

#[test]
fn a_profile_saved_before_the_checks_existed_reads_with_the_default_checks() {
    let (_home, paths) = paths();
    std::fs::create_dir_all(paths.config_dir()).unwrap();
    std::fs::write(paths.workflow_file(), "[repo.\"/a\"]\ncatch_up = \"merge\"\n").unwrap();
    let read = Store::load(&paths).get(Path::new("/a")).unwrap();
    assert_eq!(read.checks, Checks::default());
}

#[test]
fn a_switch_the_file_names_wins_and_the_ones_it_leaves_out_keep_their_default() {
    let (_home, paths) = paths();
    std::fs::create_dir_all(paths.config_dir()).unwrap();
    std::fs::write(paths.workflow_file(), "[repo.\"/a\".checks]\ntests = false\nreviewer = true\n")
        .unwrap();
    let read = Store::load(&paths).get(Path::new("/a")).unwrap();
    assert_eq!(
        read.checks,
        Checks { tests: false, verified: true, scope: true, fix_test: false, reviewer: true }
    );
}

#[test]
fn the_checks_are_saved_with_the_profile_and_come_back() {
    let (_home, paths) = paths();
    let checks = Checks { tests: false, fix_test: true, ..Checks::default() };
    let workflow = Workflow { checks, ..Workflow::default() };
    Store::update(&paths, |store| store.set(Path::new("/work/web"), workflow)).unwrap();
    let text = std::fs::read_to_string(paths.workflow_file()).unwrap();
    assert!(text.contains("[repo.\"/work/web\".checks]"), "{text}");
    assert_eq!(Store::load(&paths).get(Path::new("/work/web")), Some(workflow));
}

#[test]
fn where_a_fix_goes_follows_the_profile_without_the_push() {
    let fixup = Workflow::default();
    let new = Workflow { fixes: Fixes::NewCommits, ..fixup };
    let table = [
        (fixup, Fix::The, "fold the fix into the commit it belongs to"),
        (fixup, Fix::Each, "fold each fix into its commit"),
        (fixup, Fix::It, "fold it into its commit"),
        (new, Fix::The, "commit the fix"),
        (new, Fix::Each, "commit each fix"),
        (new, Fix::It, "commit it"),
    ];
    for (workflow, fix, want) in table {
        assert_eq!(workflow.fold(fix), want, "{workflow:?} {fix:?}");
    }
}
