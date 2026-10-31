use super::*;

const LINK: &str = "https://github.com/trakt/trakt-workers/pull/2437";

fn number(task: &str) -> Option<(u64, bool, Option<String>)> {
    Target::of_task(task)
        .pull_request()
        .map(|pr| (pr.number, pr.certain, pr.repo.as_ref().map(GithubRepo::slug)))
}

#[test]
fn a_task_names_the_pull_request_it_works_on_or_none() {
    let linked = Some((2437, true, Some("trakt/trakt-workers".to_owned())));
    let named = |n| Some((n, true, None));
    let guessed = |n| Some((n, false, None));
    let cases = [
        (format!("Sensei {LINK} and fold the fix into it"), linked.clone()),
        (format!("review <{LINK}>."), linked.clone()),
        (format!("[the PR]({LINK}) needs a rebase"), linked),
        ("Sensei PR 2437".to_owned(), named(2437)),
        ("sensei pr #2437 please".to_owned(), named(2437)),
        ("Review the Pull Request #88".to_owned(), named(88)),
        ("Finish #2437".to_owned(), guessed(2437)),
        ("Finish (#2437)".to_owned(), guessed(2437)),
        ("Bump vite to 6.3".to_owned(), None),
        ("Fix the flaky retry test".to_owned(), None),
        ("see issue #2333".to_owned(), None),
        ("Fixes #12".to_owned(), None),
        ("closes #12 and more".to_owned(), None),
        ("the https://github.com/acme/web/issues/9 report".to_owned(), None),
        ("pr review tooling".to_owned(), None),
        ("sprint 12 and apr 5".to_owned(), None),
        ("PR 12abc".to_owned(), None),
        ("PR #0".to_owned(), None),
        ("see page#12 and a/b#7".to_owned(), None),
        ("PR 99999999999999".to_owned(), None),
    ];
    for (task, want) in cases {
        assert_eq!(number(&task), want, "{task:?}");
    }
}

#[test]
fn several_pull_requests_give_the_first_and_a_link_beats_a_bare_number() {
    let cases = [
        ("compare https://github.com/a/b/pull/7 with https://github.com/a/b/pull/9", 7),
        ("compare https://github.com/a/b/pull/9 with https://github.com/a/b/pull/7", 9),
        ("PR 5 then PR 6", 5),
        ("PR 5 then https://github.com/a/b/pull/6", 6),
        ("#8 and PR 3", 3),
        ("#8 and #3", 8),
    ];
    for (task, want) in cases {
        assert_eq!(number(task).map(|found| found.0), Some(want), "{task:?}");
    }
}

#[test]
fn a_link_to_another_repo_keeps_its_repo_and_is_looked_up_by_its_address() {
    let pr =
        Target::of_task("update https://github.com/other/place/pull/31").pull_request().cloned();
    let pr = pr.expect("a pull request");
    assert_eq!(pr.repo.as_ref().map(GithubRepo::slug).as_deref(), Some("other/place"));
    assert_eq!(pr.lookup(), "https://github.com/other/place/pull/31");
    assert_eq!(pr.label(), "PR #31");
    let bare = Target::of_task("PR 31").pull_request().cloned().expect("a pull request");
    assert_eq!(bare.lookup(), "31");
}

#[test]
fn a_summary_names_a_pull_request_only_by_its_link() {
    assert_eq!(
        in_summary("Pushed to https://github.com/a/b/pull/12").map(|pr| pr.number),
        Some(12)
    );
    assert_eq!(in_summary("Pushed to PR 12"), None);
    assert_eq!(in_summary("fixed it"), None);
}
