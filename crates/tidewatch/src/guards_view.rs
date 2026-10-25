//! The Guards card of Settings, General: the outbound guard rules (the `[guards]` table of
//! `settings.toml`), each with its switch and the reason an agent is shown when it blocks, the list of
//! private repo names, and the push rule shown read-only. A repo can answer a rule for itself.

use std::path::PathBuf;

use egui::{Id, Margin, Ui};
use tidewatch_core::guards::{GuardPrefs, Rule, parse_private_repos};

use crate::settings_ui::{self, Control, note, row, section, switch_row, text_area};

const SCOPE: &str = "guards-scope";
const NAMES: &str = "guards-private-names";
const SEGMENT_H: f32 = 28.0;

/// The repo on show, which a rule can be set for on its own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Repo {
    pub(crate) path: PathBuf,
    pub(crate) label: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Change {
    /// A switch moved: for every repo (`repo` is `None`) or for one.
    Rule {
        repo: Option<PathBuf>,
        rule: Rule,
        on: bool,
    },
    PrivateRepos(Vec<String>),
}

impl Change {
    pub(crate) fn apply(&self, guards: &mut GuardPrefs) {
        match self {
            Self::Rule { repo, rule, on } => guards.set_rule(repo.as_deref(), *rule, *on),
            Self::PrivateRepos(names) => guards.private_repos.clone_from(names),
        }
    }
}

fn names_text(names: &[String]) -> String {
    names.join("\n")
}

/// The card. Returns what changed.
pub(crate) fn card(ui: &mut Ui, prefs: &GuardPrefs, repo: Option<&Repo>) -> Option<Change> {
    let mut change = None;
    section(ui, "Guards", |ui| {
        let scope_id = Id::new(SCOPE);
        let own =
            repo.is_some_and(|_| ui.data(|data| data.get_temp::<bool>(scope_id)).unwrap_or(false));
        if let Some(repo) = repo {
            let labels = ["All repos", repo.label.as_str()];
            let picked = row(
                ui,
                "Applies to",
                "Pick a repo to answer a rule for it alone. Other repos keep the default.",
                Control::new(settings_ui::inline_width(ui, &labels).min(360.0), SEGMENT_H),
                |ui| settings_ui::pick(ui, "guards-scope", &labels, Some(usize::from(own))),
            );
            if let Some(index) = picked {
                ui.data_mut(|data| data.insert_temp(scope_id, index == 1));
            }
        }
        let scope = repo.filter(|_| own);
        let rules = scope.map_or(prefs.rules, |repo| prefs.rules_for(&repo.path));
        for rule in Rule::ALL {
            if let Some(on) = switch_row(ui, rule.title(), rule.reason(), rules.is_on(rule)) {
                change = Some(Change::Rule { repo: scope.map(|r| r.path.clone()), rule, on });
            }
        }
        if let Some(names) = names_field(ui, &prefs.private_repos) {
            change = Some(Change::PrivateRepos(names));
        }
        row(
            ui,
            "Agents never push the default branch",
            "A profile setting, under Agents and profiles. Tidewatch opens the pull request itself.",
            Control::NONE,
            |_| {},
        );
    });
    change
}

fn names_field(ui: &mut Ui, saved: &[String]) -> Option<Vec<String>> {
    row(
        ui,
        "Private repos",
        "One per line, as owner/name or just the name. Text that names one is refused when the repo it goes to is public. Visibility comes from gh and is remembered for 6 hours.",
        Control::NONE,
        |_| {},
    );
    let id = Id::new(NAMES);
    let (mut draft, was_focused) = ui
        .data(|data| data.get_temp::<(String, bool)>(id))
        .filter(|(_, focused)| *focused)
        .unwrap_or_else(|| (names_text(saved), false));
    egui::Frame::new().inner_margin(Margin { left: 18, right: 18, top: 0, bottom: 16 }).show(
        ui,
        |ui| {
            text_area(ui, &mut draft, "acme/vault", 3);
            note(ui, "Saved when you leave the box.", settings_ui::FAINT);
        },
    );
    let focused = ui.ctx().text_edit_focused();
    ui.data_mut(|data| data.insert_temp(id, (draft.clone(), focused)));
    let names = parse_private_repos(&draft);
    (was_focused && !focused && names != saved).then_some(names)
}

#[cfg(test)]
mod tests;
