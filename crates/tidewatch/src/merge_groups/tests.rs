use super::*;
use crate::pane::Body;

fn main_at(root: &str, at_home: bool) -> Main {
    let repo = Repo { root: PathBuf::from(root), label: root.into() };
    Main { key: PathBuf::from(root), repo, at_home }
}

fn group(main: Option<Main>, main_busy: bool) -> Facts {
    Facts { main, main_busy }
}

fn home(root: &str) -> Facts {
    group(Some(main_at(root, true)), false)
}

fn linked(root: &str) -> Facts {
    group(Some(main_at(root, false)), false)
}

fn source(group: usize, keep_main: bool) -> Source {
    Source { group, keep_main }
}

fn repo(root: &str) -> Repo {
    Repo { root: PathBuf::from(root), label: root.into() }
}

#[test]
fn groups_of_one_repo_fold_into_the_one_at_its_root() {
    let cases: [(&str, Vec<Facts>, Vec<Merge>); 7] = [
        ("one group per repo", vec![home("/w/web"), home("/w/api")], vec![]),
        ("a lone group at its root", vec![home("/w/web")], vec![]),
        (
            "the linked worktrees of the open repo",
            vec![home("/w/web"), linked("/w/web"), linked("/w/web")],
            vec![Merge {
                home: Home::Existing(0),
                sources: vec![source(1, false), source(2, false)],
            }],
        ),
        (
            "the root after its worktrees",
            vec![linked("/w/web"), home("/w/web")],
            vec![Merge { home: Home::Existing(1), sources: vec![source(0, false)] }],
        ),
        (
            "worktrees of a repo that has no group of its own",
            vec![linked("/w/web"), home("/w/api"), linked("/w/web")],
            vec![Merge {
                home: Home::Create(repo("/w/web")),
                sources: vec![source(0, false), source(2, false)],
            }],
        ),
        (
            "a lone group that is a worktree",
            vec![linked("/w/web")],
            vec![Merge { home: Home::Create(repo("/w/web")), sources: vec![source(0, false)] }],
        ),
        (
            "a busy main shell is kept, an idle one is not",
            vec![home("/w/web"), group(Some(main_at("/w/web", false)), true), linked("/w/web")],
            vec![Merge {
                home: Home::Existing(0),
                sources: vec![source(1, true), source(2, false)],
            }],
        ),
    ];
    for (name, groups, want) in cases {
        assert_eq!(plan(&groups), want, "{name}");
    }
}

#[test]
fn the_other_group_and_folders_git_cannot_place_stay_as_they_are() {
    let groups = [home("/w/web"), group(None, false), group(None, true), linked("/w/web")];
    assert_eq!(plan(&groups), [Merge { home: Home::Existing(0), sources: vec![source(3, false)] }]);
}

#[test]
fn two_repos_fold_independently() {
    let groups = [home("/w/web"), home("/w/api"), linked("/w/api"), linked("/w/web")];
    assert_eq!(
        plan(&groups),
        [
            Merge { home: Home::Existing(0), sources: vec![source(3, false)] },
            Merge { home: Home::Existing(1), sources: vec![source(2, false)] },
        ]
    );
}

fn workspace(dir: &str) -> Workspace {
    Workspace::Repo(repo(dir))
}

fn titles(tabs: &Tabs<Pane<u32>>, group: usize) -> Vec<String> {
    tabs.in_group(group).map(|(_, tab)| tab.title.clone()).collect()
}

fn window() -> (Tabs<Pane<u32>>, Vec<Workspace>) {
    let mut tabs = Tabs::new(Pane::shell(100));
    let fix = tabs.add_group(Pane::shell(200));
    let nn = tabs.add_group(Pane::shell(300));
    tabs.push_in(0, "web work".into(), Pane::shell(101));
    tabs.push_in(fix, "fix work".into(), Pane::shell(201));
    tabs.push_in(nn, "nn work".into(), Pane::shell(301));
    let workspaces = vec![workspace("/w/web"), workspace("/w/web-fix"), workspace("/w/web-nn")];
    (tabs, workspaces)
}

fn busy_when(term: u32) -> impl Fn(&Pane<u32>) -> bool {
    move |pane| matches!(pane.body, Body::Shell(t) if t == term)
}

fn fold(
    tabs: &mut Tabs<Pane<u32>>,
    workspaces: &mut Vec<Workspace>,
    busy: impl Fn(&Pane<u32>) -> bool,
) -> Applied {
    let mains = vec![
        Some(main_at("/w/web", true)),
        Some(main_at("/w/web", false)),
        Some(main_at("/w/web", false)),
    ];
    let merges = plan(&facts(mains, tabs, busy));
    apply(tabs, workspaces, &merges, |_| 900)
}

fn terms(tabs: &Tabs<Pane<u32>>) -> Vec<(usize, String, u32)> {
    tabs.items()
        .iter()
        .map(|tab| match tab.pane.body {
            Body::Shell(term) => (tab.group, tab.title.clone(), term),
            _ => panic!("every tab has a shell"),
        })
        .collect()
}

#[test]
fn a_group_per_worktree_comes_back_as_one_group_with_every_tab_untouched() {
    let (mut tabs, mut workspaces) = window();
    let ids: Vec<u64> = tabs.items().iter().map(|tab| tab.id).collect();
    let applied = fold(&mut tabs, &mut workspaces, |_| false);

    assert_eq!(workspaces.iter().map(Workspace::dir).collect::<Vec<_>>(), [Path::new("/w/web")]);
    assert_eq!(titles(&tabs, 0), ["main", "web work", "fix work", "nn work"]);
    assert_eq!(
        terms(&tabs),
        [
            (0, "main".into(), 100),
            (0, "web work".into(), 101),
            (0, "fix work".into(), 201),
            (0, "nn work".into(), 301)
        ]
    );
    assert_eq!(applied.dropped, [ids[1], ids[2]]);
    let retired: Vec<&Path> = applied.retired.iter().map(|w| w.dir.as_path()).collect();
    assert_eq!(retired, [Path::new("/w/web-fix"), Path::new("/w/web-nn")]);
    assert!(applied.retired.iter().all(|w| w.tabs == [SavedTab::Main] && !w.other));
    assert_eq!(applied.created, Vec::<PathBuf>::new());
}

#[test]
fn a_main_shell_that_runs_a_command_stays_as_a_tab_with_its_pty() {
    let (mut tabs, mut workspaces) = window();
    let running = tabs.main_id(1).unwrap();
    let applied = fold(&mut tabs, &mut workspaces, busy_when(200));

    assert_eq!(titles(&tabs, 0), ["main", "web work", "web-fix", "fix work", "nn work"]);
    assert_eq!(applied.dropped.len(), 1, "only the idle main of the other worktree went");
    let tab = tabs.get_mut(running).unwrap();
    assert!(!tab.pinned);
    assert!(matches!(tab.pane.body, Body::Shell(200)));
    assert_eq!(tabs.in_group(0).filter(|(_, tab)| tab.pinned).count(), 1);
}

#[test]
fn the_active_tab_the_front_group_and_the_marks_survive() {
    let (mut tabs, mut workspaces) = window();
    let nn = tabs.in_group(2).nth(1).map(|(_, tab)| tab.id).unwrap();
    let fix = tabs.in_group(1).nth(1).map(|(_, tab)| tab.id).unwrap();
    tabs.get_mut(fix).unwrap().keep = true;
    tabs.apply(crate::tabs::Action::ShowGroup(2), || unreachable!());
    tabs.select_id(nn);
    fold(&mut tabs, &mut workspaces, |_| false);

    assert_eq!(tabs.items()[tabs.active()].id, nn);
    assert_eq!(tabs.active_group(), 0);
    assert!(tabs.get_mut(fix).unwrap().keep);
}

#[test]
fn a_repo_with_no_group_at_its_root_gets_one_made_after_the_others() {
    let (mut tabs, _) = window();
    let mut workspaces = vec![workspace("/w/api"), workspace("/w/web-fix"), workspace("/w/web-nn")];
    let mains = vec![
        Some(main_at("/w/api", true)),
        Some(main_at("/w/web", false)),
        Some(main_at("/w/web", false)),
    ];
    let merges = plan(&facts(mains, &tabs, |_| false));
    let applied = apply(&mut tabs, &mut workspaces, &merges, |_| 900);

    assert_eq!(applied.created, [PathBuf::from("/w/web")]);
    let dirs: Vec<&Path> = workspaces.iter().map(Workspace::dir).collect();
    assert_eq!(dirs, [Path::new("/w/api"), Path::new("/w/web")]);
    assert_eq!(titles(&tabs, 0), ["main", "web work"]);
    assert_eq!(titles(&tabs, 1), ["main", "fix work", "nn work"]);
    assert_eq!(
        terms(&tabs).iter().filter(|(group, _, term)| *group == 1 && *term == 900).count(),
        1
    );
}
