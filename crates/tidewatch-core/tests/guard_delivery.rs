//! Tidewatch's own delivery next to the agent guards: with the guard shims first on `PATH` it still
//! pushes and opens the pull request with exactly the text the user approved, dashes included. The
//! shims only act in an agent's environment (`TIDEWATCH_GUARD_CONFIG`), which delivery never has.

mod common;

use common::{FakeGh, commit_file, git_available, ok, remote_branches, reply, repos};
use tidewatch_core::delivery::{DeliveryPlan, Opens, deliver};
use tidewatch_core::guard_shim::shims;
use tidewatch_core::tools::Tools;

const BRANCH: &str = "agent/brave-otter";

#[test]
fn delivery_goes_through_with_the_guard_shims_on_the_path() {
    if !git_available() {
        return;
    }
    let r = repos();
    ok(&r.work, &["switch", "-q", "-c", BRANCH]);
    commit_file(&r.work, "auth.rs", "fn refresh() {}\n", "feat: add token refresh");
    let gh = FakeGh::new(&[reply("pr create", 0, "https://github.com/acme/web/pull/42\n", "")]);
    let shim_dir = tempfile::tempdir().unwrap();
    for (name, content) in shims() {
        common::write_executable_from_a_child(&shim_dir.path().join(name), &content);
    }
    let mut dirs = vec![shim_dir.path().to_path_buf()];
    dirs.extend(std::env::split_paths(&gh.path()));
    let tools = Tools::with_path(std::env::join_paths(dirs).unwrap());
    let plan = DeliveryPlan::build(&tools, &r.work, BRANCH, None).unwrap();

    let title = "Refresh tokens \u{2014} early";
    let body = "Body\n\nCo-Authored-By: Someone <a@b.c>";
    let delivered = deliver(&tools, &plan, title, body, Opens::Ready).unwrap();

    assert!(delivered.created);
    assert_eq!(remote_branches(&r.remote), [BRANCH, "main"]);
    assert_eq!(gh.calls_to("pr create")[0][7], title);
    assert_eq!(gh.body_file_text().as_deref(), Some(body));
}
