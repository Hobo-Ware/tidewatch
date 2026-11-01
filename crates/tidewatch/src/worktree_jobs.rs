//! The git work behind worktree tabs, run on `Jobs` threads. Each function starts one job and
//! returns at once; its `Done` arrives later through `Jobs::drain`.

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;

use tidewatch_core::clock::now_ms;
use tidewatch_core::listener::{Listener, TabRegistration};
use tidewatch_core::mcp::TabRole;
use tidewatch_core::meeseeks::Meeseeks;
use tidewatch_core::meeseeks_check;
use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::prepare::{PrepareInput, prepare};
use tidewatch_core::session_id;
use tidewatch_core::setup_recipe::{self, Recipe};
use tidewatch_core::start_base;
use tidewatch_core::tools::Tools;
use tidewatch_core::trust_apply::Grant;
use tidewatch_core::worktree::{self, BranchOutcome, Leftover, Terms, Worktree};
use tidewatch_core::worktree_safety::{Owner, Safety, Watch, with_machine};

use crate::jobs::Jobs;
use crate::new_tab::{AgentRequest, Placement};
use crate::pane::{OpenFailure, Opened, Pane, Started};

pub(crate) enum Done {
    Opened { tab: u64, result: Result<Opened, OpenFailure> },
    Inspected { tab: u64, worktree: Worktree, safety: Safety },
    Removed { tab: u64, result: Result<BranchOutcome, String> },
}

/// Where a new tab runs: a fresh worktree of the repo, or the folder itself in the Other group.
pub(crate) enum Target {
    Worktree { repo: PathBuf, placement: Placement },
    Folder { dir: PathBuf },
}

pub(crate) struct OpenSpec {
    pub(crate) paths: TidewatchPaths,
    pub(crate) target: Target,
    /// `None` opens a plain shell.
    pub(crate) agent: Option<Box<AgentRequest>>,
    pub(crate) path_var: OsString,
    pub(crate) tab: u64,
    /// Where the agent reports; `None` when the listener is down.
    pub(crate) listener: Option<Arc<Listener>>,
    /// Set after the user agreed to folder trust (D14).
    pub(crate) trust: Option<Grant>,
}

/// Makes the worktree (when asked) and gets the agent ready, off the UI thread.
pub(crate) fn open(jobs: &Jobs<Done>, ctx: &egui::Context, spec: OpenSpec) {
    jobs.spawn(ctx, move || Done::Opened { tab: spec.tab, result: run_open(&spec) });
}

fn run_open(spec: &OpenSpec) -> Result<Opened, OpenFailure> {
    let (worktree, dir, repo, base_note) = match &spec.target {
        Target::Worktree { repo, placement } => {
            let start =
                start_base::prepare(repo, placement.base.as_deref(), start_base::FETCH_TIMEOUT);
            let made = match &placement.name {
                Some(name) => worktree::add_free(&spec.paths, repo, name, start.base.as_deref()),
                None => worktree::add(&spec.paths, repo),
            }
            .map_err(|e| OpenFailure::Nothing(e.to_string()))?;
            (Some(made.clone()), made.path, repo.clone(), start.note)
        }
        Target::Folder { dir } => (None, dir.clone(), dir.clone(), None),
    };
    let (recipe, setup_notes) =
        worktree.as_ref().map_or_else(Default::default, |made| set_up(&spec.paths, made));
    let Some(request) = &spec.agent else {
        for note in &setup_notes {
            eprintln!("tidewatch: worktree setup: {note}");
        }
        return Ok(Opened { worktree, dir, started: None, base_note });
    };
    let tab_key = worktree
        .as_ref()
        .map_or_else(|| format!("tab-{}", spec.tab), |made| made.name().to_owned());
    let hooks = spec.listener.as_ref().map(|listener| {
        listener.register_tab(TabRegistration {
            tab_id: spec.tab,
            agent: request.agent.id.clone(),
            role: if request.meeseeks.is_some() { TabRole::Meeseeks } else { TabRole::Agent },
            repo: repo.clone(),
            tab: tab_key.clone(),
            generation: 0,
        })
    });
    let session_id = request.agent.session_id_flag.is_some().then(session_id::new);
    let meeseeks = request.meeseeks.map(|criteria| Meeseeks {
        start_head: worktree
            .as_ref()
            .and_then(|made| meeseeks_check::head(&Tools::system(), &made.path)),
        ..Meeseeks::summon(&request.task, criteria, now_ms())
    });
    let prepared = prepare(&PrepareInput {
        paths: &spec.paths,
        agent: &request.agent,
        profile: &request.profile,
        repo: &repo,
        tab: &tab_key,
        dir: &dir,
        worktree_tab: worktree.is_some(),
        task: &request.task,
        session_id: session_id.clone(),
        remote: request.remote,
        meeseeks: meeseeks.as_ref().map(|found| found.task.as_str()),
        path_var: Some(&spec.path_var),
        listener: hooks,
        trust: spec.trust.as_ref(),
    });
    if prepared.is_err()
        && let Some(listener) = &spec.listener
    {
        listener.unregister_tab(spec.tab);
    }
    let prepared = prepared.map(|mut prepared| {
        prepared.notes.extend(setup_notes);
        prepared.notes.extend(setup_recipe::install_through_links(&recipe).map(str::to_owned));
        prepared.setup = setup_recipe::with_generator(&recipe, &prepared.setup);
        prepared
    });
    match prepared {
        Ok(prepared) => Ok(Opened {
            worktree,
            dir,
            base_note,
            started: Some(Box::new(Started {
                profile_name: request.profile.name.clone(),
                profile_id: request.profile_id.clone(),
                level: request.profile.level,
                agent_id: request.agent.id.clone(),
                notes_file: Some(spec.paths.notes_file(&repo, &tab_key)),
                session_id,
                awaiting_prompt: request.task.trim().is_empty(),
                resumed: false,
                prepared,
                meeseeks,
            })),
        }),
        Err(e) => Err(match worktree {
            Some(worktree) => OpenFailure::Kept { worktree, message: e.to_string() },
            None => OpenFailure::Nothing(e.to_string()),
        }),
    }
}

/// Copies and links what the repo's recipe lists into the new worktree. Never fails the tab.
pub(crate) fn set_up(paths: &TidewatchPaths, made: &Worktree) -> (Recipe, Vec<String>) {
    let recipe = setup_recipe::recipe_for(paths, &made.repo);
    let outcome = setup_recipe::apply(&recipe, &made.repo, &made.path);
    (recipe, outcome.notes)
}

pub(crate) fn inspect(
    jobs: &Jobs<Done>,
    ctx: &egui::Context,
    paths: TidewatchPaths,
    tab: u64,
    worktree: Worktree,
) {
    jobs.spawn(ctx, move || {
        let safety = worktree::inspect(&paths, &worktree, Owner::ThisWindow);
        Done::Inspected { tab, worktree, safety }
    });
}

const EXIT_WAIT: std::time::Duration = std::time::Duration::from_secs(3);
const EXIT_POLL: std::time::Duration = std::time::Duration::from_millis(100);

/// Gives what the dropped shell left running a moment to exit, so the check inside `remove` does
/// not see a process that is already on its way out.
fn wait_for_exit(paths: &TidewatchPaths, worktree: &Worktree) {
    let deadline = std::time::Instant::now() + EXIT_WAIT;
    let still_running =
        |watch: &dyn Watch| watch.holders(&worktree.path).is_ok_and(|holders| !holders.is_empty());
    while std::time::Instant::now() < deadline
        && with_machine(paths, Owner::ThisWindow, still_running)
    {
        std::thread::sleep(EXIT_POLL);
    }
}

/// Stops the shell first (on this thread, not the UI's) so nothing is running inside the folder
/// git is about to delete. `remove` checks again before it deletes anything: what the dialog saw
/// may have changed since.
pub(crate) fn remove<T: Send + 'static>(
    jobs: &Jobs<Done>,
    ctx: &egui::Context,
    paths: TidewatchPaths,
    tab: u64,
    shell: Option<T>,
    worktree: Worktree,
    terms: Terms,
    trust: Option<Grant>,
) {
    jobs.spawn(ctx, move || {
        let had_shell = shell.is_some();
        drop(shell);
        if had_shell {
            wait_for_exit(&paths, &worktree);
        }
        let result = worktree::remove_with(&paths, &worktree, &terms).map_err(|e| e.to_string());
        if result.is_ok()
            && let Some(grant) = &trust
        {
            let _ = grant.revoke(Some(&worktree.path));
        }
        Done::Removed { tab, result }
    });
}

/// Leftovers scanned while a tab was being created can include that tab's own folder.
pub(crate) fn without_open<T>(
    leftovers: Vec<Leftover>,
    tabs: &crate::tabs::Tabs<Pane<T>>,
) -> Vec<Leftover> {
    leftovers
        .into_iter()
        .filter(|l| {
            !tabs.items().iter().any(|t| t.pane.worktree.as_ref().is_some_and(|w| w.path == l.path))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pane::{Body, ClosePolicy, finish_open};
    use crate::tabs::Tabs;
    use crate::test_support::git;
    use std::time::{Duration, Instant};
    use tidewatch_core::agents::Level;
    use tidewatch_core::profiles::{Profile, Profiles};
    use tidewatch_core::worktree::{BranchChoice, RemoveMode};
    use tidewatch_core::worktree_safety::Risk;

    fn leftover(path: &str) -> Leftover {
        Leftover {
            path: PathBuf::from(path),
            branch: None,
            repo: None,
            recorded: false,
            modified_ms: None,
        }
    }

    fn next(jobs: &Jobs<Done>) -> Done {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if let Some(done) = jobs.drain().into_iter().next() {
                return done;
            }
            assert!(Instant::now() < deadline, "the job never finished");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn spec(
        paths: &TidewatchPaths,
        repo: &std::path::Path,
        tab: u64,
        agent: Option<Box<AgentRequest>>,
    ) -> OpenSpec {
        OpenSpec {
            paths: paths.clone(),
            target: Target::Worktree { repo: repo.to_path_buf(), placement: Placement::default() },
            agent,
            path_var: OsString::from("/opt/homebrew/bin:/usr/bin:/bin"),
            tab,
            listener: None,
            trust: None,
        }
    }

    fn throwaway_repo(home: &std::path::Path) -> PathBuf {
        let repo = home.join("web");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["commit", "-q", "--allow-empty", "-m", "init"]);
        repo
    }

    fn request(level: Level, task: &str) -> Box<AgentRequest> {
        let profiles = Profiles::seeded(&["claude".to_owned()]);
        let agent = profiles.agent_table().get("claude").unwrap().clone();
        let profile = Profile { level, ..profiles.profile["builder"].clone() };
        Box::new(AgentRequest {
            profile_id: "builder".into(),
            profile,
            agent,
            task: task.into(),
            placement: Placement::default(),
            remote: false,
            meeseeks: None,
        })
    }

    #[test]
    fn an_agent_tab_gets_a_worktree_and_a_prepared_launch_with_the_guard() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().canonicalize().unwrap();
        let repo = throwaway_repo(&home);
        let paths = TidewatchPaths::new(&home);
        let jobs = Jobs::new();
        let plan = Some(request(Level::Edits, "write a haiku"));
        open(&jobs, &egui::Context::default(), spec(&paths, &repo, 4, plan));
        let Done::Opened { tab, result: Ok(opened) } = next(&jobs) else {
            panic!("expected Opened")
        };
        assert_eq!(tab, 4);
        let worktree = opened.worktree.expect("a worktree");
        assert_eq!(opened.dir, worktree.path);
        let started = opened.started.expect("an agent");
        assert_eq!(started.profile_name, "Builder");
        let line = started.prepared.launch.shell_line();
        assert!(line.starts_with("claude --permission-mode acceptEdits"), "{line}");
        assert!(line.ends_with("-- 'write a haiku'"), "{line}");
        let env = &started.prepared.env;
        assert!(env.iter().any(|(k, _)| k == "GIT_CONFIG_COUNT"));
        let path = &env.iter().find(|(k, _)| k == "PATH").unwrap().1;
        assert!(path.starts_with(&paths.guard_bin_dir().display().to_string()), "{path}");
        assert!(paths.house_style_file().is_file());
    }

    #[test]
    fn a_new_worktree_gets_the_repos_recipe_before_the_agent_and_a_missing_file_is_only_a_note() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().canonicalize().unwrap();
        let repo = throwaway_repo(&home);
        std::fs::write(repo.join(".env"), "TOKEN=1\n").unwrap();
        let paths = TidewatchPaths::new(&home);
        setup_recipe::Store::update(&paths, |store| {
            store.set(
                &repo,
                &Recipe {
                    copy: vec![".env".into(), ".env.local".into()],
                    generator: "echo hi".into(),
                    ..Recipe::default()
                },
            );
        })
        .unwrap();
        let jobs = Jobs::new();
        open(
            &jobs,
            &egui::Context::default(),
            spec(&paths, &repo, 8, Some(request(Level::Edits, ""))),
        );
        let Done::Opened { result: Ok(opened), .. } = next(&jobs) else {
            panic!("expected Opened")
        };
        assert_eq!(std::fs::read_to_string(opened.dir.join(".env")).unwrap(), "TOKEN=1\n");
        let prepared = opened.started.unwrap().prepared;
        assert_eq!(prepared.notes, [".env.local is not in the main checkout, skipped"]);
        assert!(prepared.setup.contains("echo hi"), "{}", prepared.setup);
    }

    #[test]
    fn with_consent_a_new_worktree_is_trusted_before_its_agent_and_untrusted_when_removed() {
        use tidewatch_core::trust::{TrustCli, is_trusted};
        use tidewatch_core::trust_apply::ConfigFiles;
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().canonicalize().unwrap();
        let repo = throwaway_repo(&home);
        let paths = TidewatchPaths::new(&home);
        let claude_json = home.join(".claude.json");
        std::fs::write(&claude_json, "{\n  \"numStartups\": 1\n}\n").unwrap();
        let grant = Grant::new(&paths, ConfigFiles::resolve(&home, |_| None));
        let jobs = Jobs::new();
        let ctx = egui::Context::default();
        let with_trust = OpenSpec {
            trust: Some(grant.clone()),
            ..spec(&paths, &repo, 6, Some(request(Level::Edits, "")))
        };
        open(&jobs, &ctx, with_trust);
        let Done::Opened { result: Ok(opened), .. } = next(&jobs) else {
            panic!("expected Opened")
        };
        let worktree = opened.worktree.expect("a worktree");
        let folder = worktree.path.canonicalize().unwrap().to_string_lossy().into_owned();
        let text = std::fs::read_to_string(&claude_json).unwrap();
        assert!(is_trusted(TrustCli::Claude, &text, &folder).unwrap(), "{text}");

        remove::<u8>(
            &jobs,
            &ctx,
            paths.clone(),
            6,
            None,
            worktree,
            Terms::new(BranchChoice::Delete, RemoveMode::Safe).owner(Owner::ThisWindow),
            Some(grant),
        );
        let Done::Removed { result, .. } = next(&jobs) else { panic!("expected Removed") };
        assert!(matches!(result, Ok(BranchOutcome::Deleted { .. })), "{result:?}");
        assert_eq!(std::fs::read_to_string(&claude_json).unwrap(), "{\n  \"numStartups\": 1\n}\n");
    }

    #[test]
    fn without_consent_no_cli_file_is_touched() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().canonicalize().unwrap();
        let repo = throwaway_repo(&home);
        let paths = TidewatchPaths::new(&home);
        let jobs = Jobs::new();
        open(
            &jobs,
            &egui::Context::default(),
            spec(&paths, &repo, 7, Some(request(Level::Edits, ""))),
        );
        let Done::Opened { result: Ok(_), .. } = next(&jobs) else { panic!("expected Opened") };
        assert!(!home.join(".claude.json").exists());
    }

    #[test]
    fn an_agent_in_a_plain_folder_gets_no_worktree_and_no_guard_and_never_yolo() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().canonicalize().unwrap();
        let folder = home.join("notes");
        std::fs::create_dir_all(&folder).unwrap();
        let paths = TidewatchPaths::new(&home);
        let jobs = Jobs::new();
        let ctx = egui::Context::default();
        let in_folder = |agent| OpenSpec {
            target: Target::Folder { dir: folder.clone() },
            ..spec(&paths, &folder, 5, Some(agent))
        };

        open(&jobs, &ctx, in_folder(request(Level::Auto, "")));
        let Done::Opened { result: Ok(opened), .. } = next(&jobs) else {
            panic!("expected Opened")
        };
        assert_eq!((opened.worktree, opened.dir.as_path()), (None, folder.as_path()));
        let started = opened.started.unwrap();
        assert!(!started.prepared.env.iter().any(|(k, _)| k == "GIT_CONFIG_COUNT"));
        assert!(!started.prepared.launch.args.iter().any(|a| a == "--"), "no task, no separator");

        open(&jobs, &ctx, in_folder(request(Level::Yolo, "")));
        let Done::Opened { result: Err(OpenFailure::Nothing(message)), .. } = next(&jobs) else {
            panic!("YOLO in Other must be refused")
        };
        assert!(message.starts_with("YOLO only runs"), "{message}");
        assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 0, "the folder stays untouched");
    }

    #[test]
    fn a_launch_that_cannot_be_built_keeps_the_worktree_for_the_tab_to_remove() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().canonicalize().unwrap();
        let repo = throwaway_repo(&home);
        let paths = TidewatchPaths::new(&home);
        let mut plan = request(Level::Edits, "");
        plan.agent = Profiles {
            agent: [(
                "aider".to_owned(),
                tidewatch_core::agents::AgentOverride {
                    bin: Some("aider".into()),
                    ..Default::default()
                },
            )]
            .into(),
            ..Profiles::default()
        }
        .agent_table()
        .get("aider")
        .unwrap()
        .clone();
        let jobs = Jobs::new();
        open(&jobs, &egui::Context::default(), spec(&paths, &repo, 6, Some(plan)));
        let Done::Opened { result: Err(OpenFailure::Kept { worktree, message }), .. } = next(&jobs)
        else {
            panic!("expected a kept worktree")
        };
        assert!(message.contains("no flags"), "{message}");
        assert!(worktree.path.is_dir());
    }

    #[test]
    fn leftovers_skip_folders_that_open_tabs_own() {
        let mut tabs: Tabs<Pane<u8>> = Tabs::new(Pane::shell(0));
        let owned = Worktree {
            repo: "/r".into(),
            path: "/root/r-1/brave-otter".into(),
            branch: "agent/brave-otter".into(),
        };
        tabs.push(
            "brave-otter".into(),
            Pane { body: Body::Shell(1), worktree: Some(owned), agent: None },
        );
        let kept = without_open(
            vec![leftover("/root/r-1/brave-otter"), leftover("/root/r-1/calm-comet")],
            &tabs,
        );
        assert_eq!(kept, [leftover("/root/r-1/calm-comet")]);
    }

    #[test]
    fn opening_and_closing_a_worktree_tab_leaves_no_folder_and_no_worktree() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().canonicalize().unwrap();
        let repo = home.join("web");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["commit", "-q", "--allow-empty", "-m", "init"]);
        let paths = TidewatchPaths::new(&home);
        let ctx = egui::Context::default();
        let jobs = Jobs::new();
        let mut tabs: Tabs<Pane<u8>> = Tabs::new(Pane::shell(0));

        let id = tabs.push("new worktree".into(), Pane::creating());
        open(&jobs, &ctx, spec(&paths, &repo, id, None));
        let Done::Opened { tab, result } = next(&jobs) else { panic!("expected Opened") };
        finish_open(&mut tabs, tab, result, 0.0, |_, _| 1);
        let worktree = tabs.get_mut(id).unwrap().pane.worktree.clone().expect("worktree tab");
        assert!(worktree.path.is_dir());
        assert!(git(&repo, &["worktree", "list"]).contains(worktree.name()));

        let policy = tabs.get_mut(id).unwrap().pane.close_policy();
        assert_eq!(policy, ClosePolicy::Confirm(worktree.clone()));
        inspect(&jobs, &ctx, paths.clone(), id, worktree.clone());
        let Done::Inspected { safety, .. } = next(&jobs) else { panic!("expected Inspected") };
        assert_eq!(safety, Safety::Safe);

        let (shell, to_remove) = tabs.get_mut(id).unwrap().pane.begin_remove().unwrap();
        remove(
            &jobs,
            &ctx,
            paths.clone(),
            id,
            shell,
            to_remove,
            Terms::new(BranchChoice::Delete, RemoveMode::Safe).owner(Owner::ThisWindow),
            None,
        );
        let Done::Removed { tab, result } = next(&jobs) else { panic!("expected Removed") };
        assert!(matches!(result, Ok(BranchOutcome::Deleted { .. })), "{result:?}");
        tabs.remove_id(tab);

        assert_eq!(tabs.items().len(), 1);
        assert!(!worktree.path.exists());
        assert!(!paths.repo_dir(&repo).exists());
        assert_eq!(git(&repo, &["worktree", "list"]).lines().count(), 1);
        assert_eq!(git(&repo, &["branch", "--list", "agent/*"]).trim(), "");
    }

    #[test]
    fn a_dirty_worktree_stays_until_discarded() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().canonicalize().unwrap();
        let repo = home.join("web");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["commit", "-q", "--allow-empty", "-m", "init"]);
        let paths = TidewatchPaths::new(&home);
        let ctx = egui::Context::default();
        let jobs = Jobs::new();

        open(&jobs, &ctx, spec(&paths, &repo, 1, None));
        let Done::Opened { result: Ok(Opened { worktree: Some(worktree), .. }), .. } = next(&jobs)
        else {
            panic!("expected a created worktree")
        };
        std::fs::write(worktree.path.join("wip.txt"), "x").unwrap();

        inspect(&jobs, &ctx, paths.clone(), 1, worktree.clone());
        let Done::Inspected { safety, .. } = next(&jobs) else { panic!("expected Inspected") };
        assert_eq!(safety, Safety::Unsafe(vec![Risk::Uncommitted { files: 1 }]));

        remove::<u8>(
            &jobs,
            &ctx,
            paths.clone(),
            1,
            None,
            worktree.clone(),
            Terms::new(BranchChoice::Keep, RemoveMode::Safe).owner(Owner::ThisWindow),
            None,
        );
        let Done::Removed { result, .. } = next(&jobs) else { panic!("expected Removed") };
        assert_eq!(result, Err("not removed: 1 file with uncommitted changes".to_owned()));
        assert!(worktree.path.join("wip.txt").is_file());

        remove::<u8>(
            &jobs,
            &ctx,
            paths,
            1,
            None,
            worktree.clone(),
            Terms::new(BranchChoice::Keep, RemoveMode::Discard).owner(Owner::ThisWindow),
            None,
        );
        let Done::Removed { result, .. } = next(&jobs) else { panic!("expected Removed") };
        assert_eq!(result, Ok(BranchOutcome::Kept));
        assert!(!worktree.path.exists());
    }

    #[test]
    fn a_placement_names_the_worktree_and_the_branch_it_starts_from() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().canonicalize().unwrap();
        let repo = throwaway_repo(&home);
        git(&repo, &["checkout", "-q", "-b", "release"]);
        git(&repo, &["commit", "-q", "--allow-empty", "-m", "on release"]);
        git(&repo, &["checkout", "-q", "main"]);
        let paths = TidewatchPaths::new(&home);
        let jobs = Jobs::new();
        let mut placed = spec(&paths, &repo, 1, None);
        placed.target = Target::Worktree {
            repo: repo.clone(),
            placement: Placement { name: Some("retry-gates".into()), base: Some("release".into()) },
        };
        open(&jobs, &egui::Context::default(), placed);
        let Done::Opened { result: Ok(opened), .. } = next(&jobs) else {
            panic!("expected Opened")
        };
        let worktree = opened.worktree.expect("a worktree");
        assert_eq!(worktree.name(), "retry-gates");
        assert_eq!(worktree.branch, "agent/retry-gates");
        let log = git(&worktree.path, &["log", "--format=%s", "-1"]);
        assert_eq!(log.trim(), "on release");
    }
}
