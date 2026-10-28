//! Live check of the webhook forwarder on a real repo: a checkout, then optionally the word kill.

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tidewatch_core::github_events::{Host, recorded_hook};
use tidewatch_core::github_forwarder::forwarder_hook_ids;
use tidewatch_core::listener::Listener;
use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::tools::Tools;

const LIMIT: Duration = Duration::from_secs(60);

fn hooks(slug: &str) -> String {
    let out = Command::new("gh").args(["api", &format!("repos/{slug}/hooks")]).output();
    out.map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default()
}

fn run_until_live(host: &mut Host, repos: &[PathBuf], listener: &Listener) -> bool {
    let start = Instant::now();
    while start.elapsed() < LIMIT {
        host.drive(repos, Some(listener));
        if !host.live_repos().is_empty() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

fn main() {
    let mut args = std::env::args().skip(1);
    let checkout = PathBuf::from(args.next().expect("a checkout of the repo"));
    let hard_kill = args.next().as_deref() == Some("kill");
    let slug = tidewatch_core::github_clone::origin_of(&Tools::system(), &checkout)
        .expect("the checkout has a GitHub origin")
        .slug();
    let home = tempfile::tempdir().unwrap();
    let paths = TidewatchPaths::new(home.path());
    let repos = vec![checkout];
    let (listener, _inbox) = Listener::start(paths.clone(), Box::new(|| {})).unwrap();
    println!("listener on {}", listener.base_url());
    let port = listener.port();

    let mut host = Host::new(Tools::system(), Some(paths.clone()), Arc::new(|| {}), true);
    let live = run_until_live(&mut host, &repos, &listener);
    println!("live after the first ping: {live}");
    std::thread::sleep(Duration::from_millis(500));
    host.drive(&repos, Some(&listener));
    println!("recorded hook id: {:?}", recorded_hook(&paths, &slug));
    let listed = hooks(&slug);
    println!("webhooks now: {:?}", forwarder_hook_ids(&listed));
    println!("hooks json: {listed}");

    if hard_kill {
        let pattern = format!("webhook forward --repo={slug}.*--url=http://127.0.0.1:{port}/");
        let _ = Command::new("pkill").args(["-KILL", "-f", &pattern]).status();
        std::thread::sleep(Duration::from_secs(2));
        println!("after the hard kill: {:?}", forwarder_hook_ids(&hooks(&slug)));
        drop(host);
        let mut second = Host::new(Tools::system(), Some(paths.clone()), Arc::new(|| {}), true);
        let live = run_until_live(&mut second, &repos, &listener);
        println!("live again after clearing the stale hook: {live}");
        println!("webhooks now: {:?}", forwarder_hook_ids(&hooks(&slug)));
        second.shutdown(Some(&listener));
    } else {
        host.shutdown(Some(&listener));
    }
    std::thread::sleep(Duration::from_secs(1));
    println!("webhooks after shutdown: {:?}", forwarder_hook_ids(&hooks(&slug)));
}
