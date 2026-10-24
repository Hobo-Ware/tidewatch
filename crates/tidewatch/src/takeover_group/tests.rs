use super::*;

fn ident(main: &str, origin: Option<&str>) -> Ident {
    Ident { main: PathBuf::from(main), origin: origin.map(normalize_origin) }
}

fn roots(open: &[Ident], movers: &[Option<Ident>]) -> Vec<Option<String>> {
    group_roots(open, movers)
        .into_iter()
        .map(|root| root.map(|root| root.to_string_lossy().into_owned()))
        .collect()
}

#[test]
fn every_spelling_of_a_remote_is_one() {
    let same = [
        "git@github.com:Acme/Web.git",
        "https://github.com/acme/web",
        "https://github.com/acme/web.git/",
        "ssh://git@github.com/Acme/Web.git",
        "https://user:token@github.com/acme/web.git",
    ];
    for url in same {
        assert_eq!(normalize_origin(url), "github.com/acme/web", "{url}");
    }
    assert_eq!(normalize_origin("/srv/git/web.git"), "/srv/git/web");
    assert_eq!(normalize_origin("file:///srv/git/web.git"), "/srv/git/web");
    assert_ne!(normalize_origin("git@github.com:acme/api.git"), normalize_origin(same[0]));
}

#[test]
fn tree_folders_are_the_ones_named_with_the_mark() {
    let cases = [
        ("/w/web_TREE", true),
        ("/w/web_TREE_2", true),
        ("/w/web_TREE_12", true),
        ("/w/web", false),
        ("/w/web_TREES", false),
        ("/w/web_TREE_x", false),
        ("/w/web-tree", false),
    ];
    for (dir, want) in cases {
        assert_eq!(is_tree_folder(Path::new(dir)), want, "{dir}");
    }
}

#[test]
fn linked_worktrees_of_one_repo_share_the_main_repo_whatever_the_remote() {
    let linked = [Some(ident("/w/web", None)), Some(ident("/w/web", None))];
    assert_eq!(roots(&[], &linked), [Some("/w/web".to_owned()), Some("/w/web".to_owned())]);
}

#[test]
fn a_folder_in_no_repo_has_no_group_root() {
    assert_eq!(roots(&[], &[None, Some(ident("/w/web", None))]), [None, Some("/w/web".to_owned())]);
}

#[test]
fn clones_of_one_remote_gather_under_the_one_that_is_not_a_tree_folder() {
    let remote = Some("git@github.com:acme/web.git");
    let movers = [
        Some(ident("/w/web_TREE", remote)),
        Some(ident("/w/web", remote)),
        Some(ident("/w/web_TREE_2", remote)),
    ];
    assert_eq!(roots(&[], &movers), vec![Some("/w/web".to_owned()); 3]);
}

#[test]
fn clones_of_one_remote_with_only_tree_folders_gather_under_the_first() {
    let remote = Some("https://github.com/acme/web");
    let movers = [Some(ident("/w/web_TREE_2", remote)), Some(ident("/w/web_TREE", remote))];
    assert_eq!(roots(&[], &movers), vec![Some("/w/web_TREE_2".to_owned()); 2]);
}

#[test]
fn a_clone_joins_the_open_repo_of_its_remote() {
    let open = [ident("/w/web_TREE", Some("git@github.com:acme/web.git"))];
    let movers = [Some(ident("/w/web", Some("https://github.com/acme/web")))];
    assert_eq!(roots(&open, &movers), [Some("/w/web_TREE".to_owned())]);
}

#[test]
fn a_folder_of_a_repo_that_is_open_stays_in_it() {
    let remote = Some("git@github.com:acme/web.git");
    let open = [ident("/w/web", remote), ident("/w/web_TREE", remote)];
    let movers = [Some(ident("/w/web_TREE", remote))];
    assert_eq!(roots(&open, &movers), [Some("/w/web_TREE".to_owned())]);
}

#[test]
fn clones_of_different_remotes_and_clones_with_no_remote_stay_apart() {
    let movers = [
        Some(ident("/w/web", Some("git@github.com:acme/web.git"))),
        Some(ident("/w/web_TREE", Some("git@github.com:acme/api.git"))),
        Some(ident("/w/scratch", None)),
        Some(ident("/w/scratch_TREE", None)),
    ];
    assert_eq!(
        roots(&[], &movers),
        ["/w/web", "/w/web_TREE", "/w/scratch", "/w/scratch_TREE"].map(|dir| Some(dir.to_owned()))
    );
}
