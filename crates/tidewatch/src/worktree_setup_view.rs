//! The Worktree setup page of Settings: per repo, the files copied from the main checkout, the
//! folders linked to it, the shared Rust build folder and the generator command. Saved as you
//! change them (`worktree-setup.toml`, never a file in the repo).

use egui::{Key, Margin, RichText, Ui, vec2};
use tidewatch_core::setup_recipe::Recipe;

use crate::screen_kit::Size;
use crate::settings_ui::{
    self, Control, MUTED, TEXT, Tone, button, caption, divider, footer_note, note, page_header,
    row, row_with, section, switch_row, text_input,
};
use crate::settings_view::Action;
use crate::setup_editor::Editor;

const ENTRY_GAP: f32 = 10.0;

pub(crate) fn page(ui: &mut Ui, editor: &mut Editor, file: &str) -> Option<Action> {
    page_header(ui, "Worktree setup", "Saved as you change them");
    editor.poll(ui.ctx());
    if editor.repos().is_empty() {
        ui.add_space(16.0);
        caption(ui, "Open a repo to set up its worktrees.");
        footer_note(ui, file);
        return None;
    }
    repo_card(ui, editor);
    let mut saved = copy_card(ui, editor);
    saved = saved.or_else(|| link_card(ui, editor));
    saved = saved.or_else(|| build_card(ui, editor));
    saved = saved.or_else(|| generator_card(ui, editor));
    footer_note(ui, file);
    let repo = editor.repo()?.dir.clone();
    saved.map(|recipe| Action::WorktreeSetup { repo, recipe })
}

fn repo_card(ui: &mut Ui, editor: &mut Editor) {
    let names: Vec<&str> = editor.repos().iter().map(|repo| repo.name.as_str()).collect();
    let mut picked = None;
    section(ui, "Repo", |ui| {
        if names.len() > 1 {
            picked = pick_row(ui, &names, editor.selected());
        }
        if let Some(repo) = editor.repo() {
            row(ui, &repo.name, &repo.dir.display().to_string(), Control::NONE, |_| {});
        }
    });
    if let Some(index) = picked {
        editor.select(index);
    }
}

fn pick_row(ui: &mut Ui, names: &[&str], active: usize) -> Option<usize> {
    egui::Frame::new()
        .inner_margin(Margin::symmetric(18, 14))
        .show(ui, |ui| settings_ui::pick(ui, "setup-repo", names, Some(active)))
        .inner
}

enum Edit {
    Add(String),
    Remove(usize),
}

fn copy_card(ui: &mut Ui, editor: &mut Editor) -> Option<Recipe> {
    let mut edit = None;
    let entries = editor.recipe().copy.clone();
    let offered: Vec<String> = editor.offered().into_iter().map(str::to_owned).collect();
    section(ui, "Copy from the main checkout", |ui| {
        edit = entry_rows(ui, &entries, "Nothing is copied.").map(Edit::Remove);
        for name in &offered {
            if suggestion_row(ui, name) {
                edit = Some(Edit::Add(name.clone()));
            }
        }
        let mut draft = std::mem::take(&mut editor.copy_draft);
        if draft_row(ui, "copy", &mut draft, ".env.local or apps/web/.env", "Add") {
            edit = Some(Edit::Add(draft.clone()));
        }
        editor.copy_draft = draft;
    });
    let saved = match edit {
        Some(Edit::Add(name)) => editor.add_copy(&name),
        Some(Edit::Remove(at)) => editor.remove_copy(at),
        None => None,
    };
    caption_problem(ui, editor);
    saved
}

fn link_card(ui: &mut Ui, editor: &mut Editor) -> Option<Recipe> {
    let mut edit = None;
    let entries = editor.recipe().link.clone();
    section(ui, "Link to the main checkout", |ui| {
        edit = entry_rows(ui, &entries, "Nothing is linked.").map(Edit::Remove);
        let mut draft = std::mem::take(&mut editor.link_draft);
        if draft_row(ui, "link", &mut draft, "node_modules", "Add") {
            edit = Some(Edit::Add(draft.clone()));
        }
        editor.link_draft = draft;
    });
    ui.add_space(8.0);
    caption(
        ui,
        "An install in a worktree would write through a link into the main checkout. Tidewatch never \
         installs through one and keeps an install command out of the generator while one is linked.",
    );
    match edit {
        Some(Edit::Add(name)) => editor.add_link(&name),
        Some(Edit::Remove(at)) => editor.remove_link(at),
        None => None,
    }
}

fn build_card(ui: &mut Ui, editor: &mut Editor) -> Option<Recipe> {
    let on = editor.recipe().shared_build_dir;
    let mut toggled = None;
    section(ui, "Build", |ui| {
        if editor.has_cargo() {
            toggled = switch_row(
                ui,
                "Share the main checkout's target folder",
                "Sets CARGO_TARGET_DIR for the agent, so every worktree builds into the same folder and \
                 the firewall is asked once.",
                on,
            );
        } else {
            row(
                ui,
                "Share the main checkout's target folder",
                "Only for a Rust repo: this one has no Cargo.toml.",
                Control::NONE,
                |_| {},
            );
        }
    });
    editor.share_build_dir(toggled?)
}

fn generator_card(ui: &mut Ui, editor: &mut Editor) -> Option<Recipe> {
    let mut commit = false;
    section(ui, "Generator", |ui| {
        egui::Frame::new().inner_margin(Margin::symmetric(18, 14)).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.label(RichText::new("Command run in the new worktree").size(15.0).color(TEXT));
            ui.label(
                RichText::new(
                    "Runs before the agent starts, with its output in the terminal and a Worktree \
                     setup job in Progress. If it fails the agent starts anyway.",
                )
                .size(13.0)
                .color(MUTED),
            );
            let response = text_input(ui, &mut editor.generator, "npm run prepare");
            commit = response.lost_focus();
        });
    });
    caption_problem(ui, editor);
    commit.then(|| editor.commit_generator()).flatten()
}

fn caption_problem(ui: &mut Ui, editor: &Editor) {
    if let Some(problem) = editor.problem {
        ui.add_space(6.0);
        note(ui, problem, crate::brand::ORANGE);
    }
}

fn entry_rows(ui: &mut Ui, entries: &[String], empty: &str) -> Option<usize> {
    if entries.is_empty() {
        row(ui, empty, "", Control::NONE, |_| {});
    }
    let mut removed = None;
    for (at, entry) in entries.iter().enumerate() {
        let clicked = row_with(
            ui,
            Control::button(ui, "Remove", Size::Compact),
            |ui| {
                ui.label(RichText::new(entry).monospace().size(14.0).color(TEXT));
            },
            |ui| button(ui, "Remove", Tone::Quiet, Size::Compact).clicked(),
        );
        if clicked {
            removed = Some(at);
        }
    }
    removed
}

fn suggestion_row(ui: &mut Ui, name: &str) -> bool {
    row_with(
        ui,
        Control::button(ui, "Add", Size::Compact),
        |ui| {
            ui.spacing_mut().item_spacing.y = 3.0;
            ui.label(RichText::new(name).monospace().size(14.0).color(TEXT));
            ui.label(
                RichText::new("Ignored by git, found in the main checkout").size(13.0).color(MUTED),
            );
        },
        |ui| button(ui, "Add", Tone::Primary, Size::Compact).clicked(),
    )
}

fn draft_row(ui: &mut Ui, id: &str, draft: &mut String, hint: &str, label: &str) -> bool {
    divider(ui);
    egui::Frame::new()
        .inner_margin(Margin::symmetric(18, 12))
        .show(ui, |ui| {
            let button_w = settings_ui::button_width(ui, label, Size::Compact);
            let field_w = (ui.available_width() - button_w - ENTRY_GAP).max(120.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = ENTRY_GAP;
                let enter = ui
                    .allocate_ui(vec2(field_w, 38.0), |ui| {
                        ui.push_id(id, |ui| {
                            let response = text_input(ui, draft, hint);
                            response.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter))
                        })
                        .inner
                    })
                    .inner;
                let clicked = button(ui, label, Tone::Quiet, Size::Compact).clicked();
                (clicked || enter) && !draft.trim().is_empty()
            })
            .inner
        })
        .inner
}

#[cfg(test)]
mod tests;
