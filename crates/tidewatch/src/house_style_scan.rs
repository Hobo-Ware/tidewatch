//! The lines of the owner's `house-style.md` that name a tool Tidewatch no longer offers, read on
//! a `Jobs` thread while the House style page shows. Removing one is a click on that page.

use tidewatch_core::dead_tools::{self, DeadLine};
use tidewatch_core::paths::TidewatchPaths;

use crate::jobs::Jobs;

const REFRESH_SECS: f64 = 3.0;

enum Done {
    Read(Vec<DeadLine>),
    Removed(Result<bool, String>),
}

/// What a finished removal tells the owner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    Removed,
    FileChanged,
    Failed(String),
}

impl Outcome {
    pub(crate) fn toast(&self) -> String {
        match self {
            Self::Removed => "Removed the line from your house style".to_owned(),
            Self::FileChanged => {
                "Your house style changed since it was read, so nothing was removed".to_owned()
            }
            Self::Failed(why) => format!("Could not edit your house style: {why}"),
        }
    }
}

pub(crate) struct Host {
    jobs: Jobs<Done>,
    lines: Vec<DeadLine>,
    asked: Option<f64>,
}

impl Host {
    pub(crate) fn new() -> Self {
        Self { jobs: Jobs::new(), lines: Vec::new(), asked: None }
    }

    pub(crate) fn lines(&self) -> &[DeadLine] {
        &self.lines
    }

    /// Takes what finished and, while the page shows, asks for a fresh read when the last is old.
    pub(crate) fn tick(
        &mut self,
        paths: Option<&TidewatchPaths>,
        shown: bool,
        now: f64,
        ctx: &egui::Context,
    ) -> Option<Outcome> {
        let mut outcome = None;
        for done in self.jobs.drain() {
            match done {
                Done::Read(lines) => self.lines = lines,
                Done::Removed(result) => {
                    self.asked = None;
                    outcome = Some(match result {
                        Ok(true) => Outcome::Removed,
                        Ok(false) => Outcome::FileChanged,
                        Err(why) => Outcome::Failed(why),
                    });
                }
            }
        }
        if let Some(paths) = paths.filter(|_| shown)
            && self.asked.is_none_or(|at| now - at >= REFRESH_SECS)
        {
            self.asked = Some(now);
            let file = paths.house_style_file();
            self.jobs.spawn(ctx, move || {
                let text = std::fs::read_to_string(file).unwrap_or_default();
                Done::Read(dead_tools::scan(&text))
            });
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(REFRESH_SECS + 0.1));
        }
        outcome
    }

    /// Removes line `number` from the file when it still reads `text`.
    pub(crate) fn remove(
        &mut self,
        paths: &TidewatchPaths,
        number: usize,
        text: String,
        ctx: &egui::Context,
    ) {
        let file = paths.house_style_file();
        self.jobs.spawn(ctx, move || {
            Done::Removed(
                dead_tools::remove_from_file(&file, number, &text).map_err(|e| e.to_string()),
            )
        });
    }
}

#[cfg(test)]
mod tests;
