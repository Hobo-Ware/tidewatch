//! The Open a repo dialog's state: the pasted GitHub link, the folder a clone goes in, what the link
//! comes to (a checkout already on the disk, a clone, a refusal) and the clone under way. Pure.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use tidewatch_core::github_clone::{self, Landing};
use tidewatch_core::github_link::GithubRepo;

pub(crate) type Key = (String, PathBuf);

pub(crate) struct Planned {
    pub(crate) key: Key,
    pub(crate) landing: Landing,
}

pub(crate) struct Question {
    pub(crate) key: Key,
    pub(crate) repo: GithubRepo,
    pub(crate) parent: PathBuf,
    pub(crate) known: Vec<PathBuf>,
}

#[derive(Clone)]
pub(crate) struct Cloning {
    pub(crate) repo: GithubRepo,
    pub(crate) target: PathBuf,
    pub(crate) line: Arc<Mutex<String>>,
    pub(crate) cancel: Arc<AtomicBool>,
}

pub(crate) enum Phase {
    Form,
    Cloning(Cloning),
    Failed(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Verdict {
    Idle,
    NotALink,
    Checking(GithubRepo),
    Ready(GithubRepo, Landing),
}

pub(crate) struct OpenLink {
    pub(crate) link: String,
    pub(crate) parent: String,
    pub(crate) phase: Phase,
    pub(crate) focus: bool,
    home: PathBuf,
    default_parent: PathBuf,
    known: Vec<PathBuf>,
    planned: Option<Planned>,
    asked: Option<Key>,
}

impl OpenLink {
    pub(crate) fn open(known: Vec<PathBuf>, last_repo: Option<&Path>, home: &Path) -> Self {
        let default_parent = github_clone::default_parent(last_repo, home);
        Self {
            link: String::new(),
            parent: default_parent.display().to_string(),
            phase: Phase::Form,
            focus: true,
            home: home.to_path_buf(),
            default_parent,
            known,
            planned: None,
            asked: None,
        }
    }

    pub(crate) fn repo(&self) -> Option<GithubRepo> {
        GithubRepo::parse(&self.link)
    }

    pub(crate) fn parent_path(&self) -> PathBuf {
        let text = self.parent.trim();
        match text {
            "" => self.default_parent.clone(),
            "~" => self.home.clone(),
            _ => text.strip_prefix("~/").map_or_else(|| PathBuf::from(text), |r| self.home.join(r)),
        }
    }

    fn key_of(&self, repo: &GithubRepo) -> Key {
        (repo.slug().to_lowercase(), self.parent_path())
    }

    pub(crate) fn verdict(&self) -> Verdict {
        if self.link.trim().is_empty() {
            return Verdict::Idle;
        }
        let Some(repo) = self.repo() else { return Verdict::NotALink };
        match &self.planned {
            Some(planned) if planned.key == self.key_of(&repo) => {
                Verdict::Ready(repo, planned.landing.clone())
            }
            _ => Verdict::Checking(repo),
        }
    }

    pub(crate) fn question(&mut self) -> Option<Question> {
        if !matches!(self.phase, Phase::Form) {
            return None;
        }
        let repo = self.repo()?;
        let key = self.key_of(&repo);
        if self.asked.as_ref() == Some(&key) {
            return None;
        }
        self.asked = Some(key.clone());
        Some(Question { key, parent: self.parent_path(), known: self.known.clone(), repo })
    }

    pub(crate) fn answered(&mut self, planned: Planned) {
        if self.asked.as_ref() == Some(&planned.key) {
            self.planned = Some(planned);
        }
    }

    pub(crate) fn retry(&mut self) {
        self.phase = Phase::Form;
        self.asked = None;
        self.planned = None;
    }

    pub(crate) fn start(&mut self, repo: GithubRepo, target: PathBuf) -> Cloning {
        let cloning = Cloning {
            repo,
            target,
            line: Arc::new(Mutex::new(String::new())),
            cancel: Arc::new(AtomicBool::new(false)),
        };
        self.phase = Phase::Cloning(cloning.clone());
        cloning
    }
}

/// The first pasted text that is a link to a GitHub repo.
pub(crate) fn pasted_link(events: &[egui::Event]) -> Option<String> {
    events.iter().find_map(|event| match event {
        egui::Event::Paste(text) => GithubRepo::parse(text).map(|_| text.trim().to_owned()),
        _ => None,
    })
}

/// The `42` of git's `Receiving objects:  42% (1234/2900)`, as a fraction. git counts the work in
/// phases (counting, compressing, receiving, resolving); the last line says which one is under way.
pub(crate) fn fraction_of(line: &str) -> Option<f32> {
    let percent = line.split('%').next()?.rsplit(|c: char| !c.is_ascii_digit()).next()?;
    let value: f32 = percent.parse().ok()?;
    line.contains('%').then_some((value / 100.0).clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dialog() -> OpenLink {
        OpenLink::open(
            vec![PathBuf::from("/code/api")],
            Some(Path::new("/code/api")),
            Path::new("/Users/me"),
        )
    }

    fn landing_for(open: &mut OpenLink, landing: Landing) {
        let question = open.question().unwrap();
        open.answered(Planned { key: question.key, landing });
    }

    #[test]
    fn the_folder_starts_next_to_the_last_repo_and_takes_a_tilde() {
        let mut open = dialog();
        assert_eq!(open.parent_path(), Path::new("/code"));
        open.parent = "~/work".into();
        assert_eq!(open.parent_path(), Path::new("/Users/me/work"));
        open.parent = "~".into();
        assert_eq!(open.parent_path(), Path::new("/Users/me"));
        open.parent = "  /tmp/x ".into();
        assert_eq!(open.parent_path(), Path::new("/tmp/x"));
        open.parent = "  ".into();
        assert_eq!(open.parent_path(), Path::new("/code"));
        assert_eq!(
            OpenLink::open(vec![], None, Path::new("/Users/me")).parent_path(),
            Path::new("/Users/me/Git")
        );
    }

    #[test]
    fn the_verdict_follows_the_text_and_waits_for_the_answer_about_this_link_and_folder() {
        let mut open = dialog();
        assert_eq!(open.verdict(), Verdict::Idle);
        open.link = "nonsense".into();
        assert_eq!(open.verdict(), Verdict::NotALink);
        assert!(open.question().is_none());
        open.link = "github.com/acme/web".into();
        let repo = open.repo().unwrap();
        assert_eq!(open.verdict(), Verdict::Checking(repo.clone()));
        let target = PathBuf::from("/code/web");
        landing_for(&mut open, Landing::Clone { target: target.clone() });
        assert_eq!(open.verdict(), Verdict::Ready(repo.clone(), Landing::Clone { target }));
        open.parent = "/elsewhere".into();
        assert_eq!(open.verdict(), Verdict::Checking(repo.clone()));
        open.link = "https://github.com/ACME/Web.git".into();
        open.parent = "/code".into();
        assert!(matches!(open.verdict(), Verdict::Ready(..)));
    }

    #[test]
    fn each_link_and_folder_is_asked_about_once() {
        let mut open = dialog();
        open.link = "github.com/acme/web".into();
        let first = open.question().unwrap();
        assert_eq!(first.parent, Path::new("/code"));
        assert_eq!(first.known, [PathBuf::from("/code/api")]);
        assert!(open.question().is_none());
        open.parent = "/other".into();
        assert!(open.question().is_some());
        open.link = "github.com/acme/api".into();
        assert!(open.question().is_some());
        assert!(open.question().is_none());
    }

    #[test]
    fn an_answer_to_an_earlier_question_is_dropped() {
        let mut open = dialog();
        open.link = "github.com/acme/web".into();
        let old = open.question().unwrap();
        open.link = "github.com/acme/api".into();
        open.question().unwrap();
        open.answered(Planned { key: old.key, landing: Landing::Open(PathBuf::from("/x")) });
        assert!(matches!(open.verdict(), Verdict::Checking(_)));
    }

    #[test]
    fn a_clone_that_failed_can_be_tried_again_from_the_form() {
        let mut open = dialog();
        open.link = "github.com/acme/web".into();
        landing_for(&mut open, Landing::Clone { target: PathBuf::from("/code/web") });
        open.start(open.repo().unwrap(), PathBuf::from("/code/web"));
        assert!(open.question().is_none());
        open.phase = Phase::Failed("no".into());
        open.retry();
        assert!(matches!(open.verdict(), Verdict::Checking(_)));
        assert!(open.question().is_some());
    }

    #[test]
    fn a_paste_of_a_repo_link_is_picked_out_of_the_events() {
        let events = [
            egui::Event::Paste("just some words".into()),
            egui::Event::Paste("  git@github.com:acme/web.git\n".into()),
            egui::Event::Paste("https://github.com/acme/api".into()),
        ];
        assert_eq!(pasted_link(&events).as_deref(), Some("git@github.com:acme/web.git"));
        assert_eq!(pasted_link(&events[..1]), None);
        assert_eq!(pasted_link(&[egui::Event::Copy]), None);
    }

    #[test]
    fn git_progress_lines_give_a_fraction_when_they_have_a_percentage() {
        let cases = [
            ("Receiving objects:  42% (1234/2900), 3.1 MiB | 2.0 MiB/s", Some(0.42)),
            ("Resolving deltas: 100% (80/80), done.", Some(1.0)),
            ("remote: Counting objects:   7% (1/14)", Some(0.07)),
            ("Cloning into 'web'...", None),
            ("", None),
            ("%", None),
        ];
        for (line, want) in cases {
            assert_eq!(fraction_of(line), want, "{line:?}");
        }
    }
}
