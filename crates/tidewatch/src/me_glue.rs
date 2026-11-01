//! Loads the owner's face on `Jobs` threads and asks gh again when the picture is a day old. A
//! screenshot or background run never calls gh; the dev switch `TIDEWATCH_FAKE_ME` names an image
//! or `letter:X` and fetches nothing.

use std::path::PathBuf;

use tidewatch_core::clock::now_secs;
use tidewatch_core::me_face as core;
use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::tools::Tools;

use crate::jobs::Jobs;
use crate::me_face::{self, Me};

const FAKE_LETTER: &str = "letter:";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Source {
    Fake(Fake),
    Local,
    Live,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Fake {
    Image(PathBuf),
    Letter(String),
}

pub(crate) fn source(fake: Option<&str>, background: bool) -> Source {
    match fake.filter(|value| !value.is_empty()) {
        Some(value) => Source::Fake(value.strip_prefix(FAKE_LETTER).map_or_else(
            || Fake::Image(PathBuf::from(value)),
            |text| Fake::Letter(core::letter_of(text)),
        )),
        None if background => Source::Local,
        None => Source::Live,
    }
}

pub(crate) enum Done {
    Loaded { me: Me, refresh: bool },
    Refreshed(Option<Me>),
}

pub(crate) struct Host {
    jobs: Jobs<Done>,
    source: Source,
    paths: Option<TidewatchPaths>,
    tools: Tools,
}

fn letter(tools: &Tools) -> Me {
    let name = core::git_name(tools, &std::env::temp_dir()).unwrap_or_default();
    Me::Letter(core::letter_of(&name))
}

fn load(paths: Option<&TidewatchPaths>, tools: &Tools) -> (Me, bool) {
    let Some(paths) = paths else { return (letter(tools), false) };
    let picture = std::fs::read(paths.me_avatar_file()).ok().and_then(|bytes| Me::decode(&bytes));
    let due = picture.is_none() || core::refresh_due(core::read_stamp(paths), now_secs());
    (picture.unwrap_or_else(|| letter(tools)), due)
}

fn load_fake(fake: &Fake) -> Me {
    match fake {
        Fake::Letter(text) => Me::Letter(text.clone()),
        Fake::Image(path) => std::fs::read(path)
            .ok()
            .and_then(|bytes| Me::decode(&bytes))
            .unwrap_or_else(|| Me::Letter(core::letter_of(""))),
    }
}

impl Host {
    pub(crate) fn start(
        ctx: &egui::Context,
        paths: Option<&TidewatchPaths>,
        tools: Tools,
        background: bool,
    ) -> Self {
        let fake = std::env::var("TIDEWATCH_FAKE_ME").ok();
        Self::begin(ctx, paths, tools, source(fake.as_deref(), background))
    }

    fn begin(
        ctx: &egui::Context,
        paths: Option<&TidewatchPaths>,
        tools: Tools,
        source: Source,
    ) -> Self {
        let host = Self { jobs: Jobs::new(), source, paths: paths.cloned(), tools };
        let (paths, tools) = (host.paths.clone(), host.tools.clone());
        let fake = match &host.source {
            Source::Fake(fake) => Some(fake.clone()),
            Source::Local | Source::Live => None,
        };
        host.jobs.spawn(ctx, move || {
            let (me, refresh) =
                fake.map_or_else(|| load(paths.as_ref(), &tools), |fake| (load_fake(&fake), false));
            Done::Loaded { me, refresh }
        });
        host
    }

    pub(crate) fn tick(&self, ctx: &egui::Context) {
        for done in self.jobs.drain() {
            match done {
                Done::Loaded { me, refresh } => {
                    me_face::set(ctx, me);
                    if refresh && self.source == Source::Live {
                        self.refresh(ctx);
                    }
                }
                Done::Refreshed(me) => {
                    if let Some(me) = me {
                        me_face::set(ctx, me);
                    }
                }
            }
        }
    }

    fn refresh(&self, ctx: &egui::Context) {
        let Some(paths) = self.paths.clone() else { return };
        let tools = self.tools.clone();
        self.jobs.spawn(ctx, move || match core::refresh(&tools, &paths, now_secs()) {
            Ok(()) => Done::Refreshed(Some(load(Some(&paths), &tools).0)),
            Err(reason) => {
                eprintln!("tidewatch: your picture was not refreshed ({reason})");
                Done::Refreshed(None)
            }
        });
    }
}

#[cfg(test)]
mod tests;
