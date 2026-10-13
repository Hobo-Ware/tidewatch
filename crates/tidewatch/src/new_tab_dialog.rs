//! Draws the new-tab dialog: profile list, permission ladder, task, live command preview. All
//! decisions are in `new_tab`; Enter starts, Esc cancels.

use egui::{Align, Id, Key, Layout, Modifiers, RichText, Stroke};
use stdusk_egui::colors;
use stdusk_egui::widgets::{action_button, chip, overlay_frame, text_field};
use tidewatch_core::agents::Level;
use tidewatch_core::paths::TidewatchPaths;

use crate::catalog::{Availability, Catalog};
use crate::commands::Command;
use crate::list_row::{ROW_HEIGHT, Row, pick_row};
use crate::new_tab::{Group, NewTab, Pick, Request, level_refusal};
use crate::new_tab_workflow;

const DIALOG_WIDTH: f32 = 560.0;
const LIST_ROWS: f32 = 7.5;
const SECTION_GAP: f32 = 6.0;

pub(crate) enum Outcome {
    Start(Request),
    Cancel,
}

pub(crate) fn show(
    ctx: &egui::Context,
    state: &mut NewTab,
    catalog: &Catalog,
    paths: Option<&TidewatchPaths>,
) -> Option<Outcome> {
    let (up, down) = ctx.input_mut(|i| {
        (
            i.consume_key(Modifiers::NONE, Key::ArrowUp),
            i.consume_key(Modifiers::NONE, Key::ArrowDown),
        )
    });
    if up || down {
        state.step(down, catalog);
    }
    let focused = ctx.memory(egui::Memory::focused);
    let enter_is_ours = focused.is_none() || focused == state.task_field;
    let enter = enter_is_ours && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
    state.refresh_preview(catalog, paths);

    let mut outcome = None;
    let mut task_field = None;
    let modal = egui::Modal::new(Id::new("new_tab")).frame(overlay_frame()).show(ctx, |ui| {
        ui.set_width(DIALOG_WIDTH);
        ui.spacing_mut().item_spacing.y = SECTION_GAP;
        ui.label(
            RichText::new(format!("New tab in {}", state.label()))
                .strong()
                .size(15.0)
                .color(colors::fg()),
        );
        section_label(ui, "Start");
        profile_list(ui, state, catalog, &mut outcome);
        if state.selected_entry(catalog).is_some() {
            ladder(ui, state, catalog);
        }
        if state.takes_task() {
            ui.add_space(2.0);
            let (caption, hint) = if state.selected_workflow(catalog).is_some() {
                ("Task", "What should the workflow build or fix?")
            } else {
                ("Task (optional)", "What should the agent do first?")
            };
            section_label(ui, caption);
            let focus = std::mem::take(&mut state.focus_task);
            let mut task = std::mem::take(&mut state.task);
            let field = ui
                .push_id("new-tab-task", |ui| {
                    text_field(ui, &mut task, hint, DIALOG_WIDTH, colors::fg())
                })
                .inner;
            if field.changed() {
                state.task_edited();
            }
            if focus {
                field.request_focus();
            }
            if field.has_focus() {
                let keep_arrows = egui::EventFilter {
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    ..egui::EventFilter::default()
                };
                ui.memory_mut(|m| m.set_focus_lock_filter(field.id, keep_arrows));
            }
            state.task = task;
            task_field = Some(field.id);
        } else {
            state.focus_task = false;
        }
        preview_block(ui, state, catalog);
        if let Some(error) = &state.error {
            ui.label(RichText::new(error).color(colors::red()));
        }
        footer(ui, state, catalog, &mut outcome);
    });

    state.task_field = task_field;
    if outcome.is_none() && enter {
        outcome = start(state, catalog);
    }
    if outcome.is_none() && modal.should_close() {
        outcome = Some(Outcome::Cancel);
    }
    if state.refresh_preview(catalog, paths) {
        ctx.request_repaint();
    }
    outcome
}

fn start(state: &mut NewTab, catalog: &Catalog) -> Option<Outcome> {
    match state.request(catalog) {
        Ok(request) => Some(Outcome::Start(request)),
        Err(reason) => {
            state.error = Some(reason);
            None
        }
    }
}

fn section_label(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(12.0).color(colors::dim()));
}

fn profile_list(
    ui: &mut egui::Ui,
    state: &mut NewTab,
    catalog: &Catalog,
    outcome: &mut Option<Outcome>,
) {
    let mut clicked = None;
    let scroll_to_selected = std::mem::take(&mut state.scroll_to_selected);
    egui::ScrollArea::vertical()
        .max_height(ROW_HEIGHT * LIST_ROWS)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            for (index, pick) in state.picks().to_vec().iter().enumerate() {
                let row = match pick {
                    Pick::Profile(id) => {
                        let Some((_, profile, agent)) =
                            catalog.entries().into_iter().find(|(x, _, _)| x == id)
                        else {
                            continue;
                        };
                        let (detail, found) =
                            agent_detail(&agent.name, &catalog.availability(&agent.id));
                        Row {
                            name: profile.name.clone(),
                            detail,
                            detail_color: if found { colors::dim() } else { colors::yellow() },
                            tag: profile.level.label().to_owned(),
                            dimmed: !found,
                        }
                    }
                    Pick::Workflow(id) => {
                        let Some(workflow) = catalog.workflows().and_then(|w| w.workflow.get(id))
                        else {
                            continue;
                        };
                        Row {
                            name: workflow.name.clone(),
                            detail: workflow.description.clone(),
                            detail_color: colors::dim(),
                            tag: new_tab_workflow::tag(workflow),
                            dimmed: false,
                        }
                    }
                    Pick::Shell => Row {
                        name: "Shell".to_owned(),
                        detail: match state.group() {
                            Group::Repo => "a plain terminal in the new worktree".to_owned(),
                            Group::Other => "a plain terminal in this folder".to_owned(),
                        },
                        detail_color: colors::dim(),
                        tag: String::new(),
                        dimmed: false,
                    },
                };
                let selected = index == state.selected();
                let response = pick_row(ui, &row, selected);
                if selected && scroll_to_selected {
                    response.scroll_to_me(None);
                }
                if response.double_clicked() {
                    clicked = Some((index, true));
                } else if response.clicked() {
                    clicked = Some((index, false));
                }
            }
        });
    ui.spacing_mut().item_spacing.y = SECTION_GAP;
    if let Some((index, start_now)) = clicked {
        state.select(index, catalog);
        if start_now {
            *outcome = start(state, catalog);
        }
    }
    if catalog.profiles().is_some_and(|p| p.profile.is_empty()) {
        ui.label(
            RichText::new(format!(
                "No agent profiles yet. Add one in Settings ({}).",
                Command::Settings.keys()
            ))
            .size(12.0)
            .color(colors::dim()),
        );
    }
}

fn agent_detail(name: &str, availability: &Availability) -> (String, bool) {
    match availability {
        Availability::Probing => (format!("{name}  checking..."), true),
        Availability::Found(Some(version)) => (format!("{name} {version}"), true),
        Availability::Found(None) => (name.to_owned(), true),
        Availability::Missing => (format!("{name}  not found on PATH"), false),
    }
}

fn ladder(ui: &mut egui::Ui, state: &mut NewTab, catalog: &Catalog) {
    let Some((_, agent)) = state.selected_entry(catalog) else { return };
    let agent = agent.clone();
    ui.add_space(2.0);
    section_label(ui, "Permissions for this tab");
    let mut picked = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        for level in Level::ALL {
            let refusal = level_refusal(state.group(), &agent, level);
            let response = ui
                .add_enabled_ui(refusal.is_none(), |ui| {
                    chip(ui, level.label(), level == state.level())
                })
                .inner;
            if let Some(reason) = &refusal {
                response.on_disabled_hover_text(reason);
            } else if response.clicked() {
                picked = Some(level);
            }
        }
    });
    if let Some(level) = picked {
        state.set_level(level, catalog);
    }
    let flags = agent.level_args(state.level()).map(|args| args.join(" ")).unwrap_or_default();
    let adds = if flags.is_empty() { "Adds no flag.".to_owned() } else { format!("Adds {flags}") };
    ui.label(RichText::new(adds).monospace().size(11.5).color(colors::dim()));
    if let Some(reason) = level_refusal(state.group(), &agent, Level::Yolo)
        && state.group() == Group::Other
    {
        ui.label(RichText::new(reason).size(12.0).color(colors::red()));
    }
}

fn preview_block(ui: &mut egui::Ui, state: &NewTab, catalog: &Catalog) {
    ui.add_space(2.0);
    let preview = state.preview();
    egui::Frame::new()
        .fill(colors::bg())
        .stroke(Stroke::new(1.0, colors::border()))
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 3.0;
            for line in &preview.lines {
                ui.add(
                    egui::Label::new(
                        RichText::new(line).monospace().size(11.5).color(colors::dim()),
                    )
                    .wrap(),
                );
            }
        });
    if let Some((_, agent)) = state.selected_entry(catalog)
        && catalog.availability(&agent.id) == Availability::Missing
    {
        ui.label(
            RichText::new(format!(
                "{} was not found on PATH. Starting will still try your login shell.",
                agent.bin
            ))
            .size(12.0)
            .color(colors::yellow()),
        );
    }
    for warning in &preview.warnings {
        ui.label(RichText::new(warning).size(12.0).color(colors::yellow()));
    }
}

fn footer(ui: &mut egui::Ui, state: &mut NewTab, catalog: &Catalog, outcome: &mut Option<Outcome>) {
    ui.add_space(4.0);
    ui.label(
        RichText::new("Enter start  -  Esc cancel  -  Up/Down pick a profile or workflow")
            .size(12.0)
            .color(colors::dim()),
    );
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        if action_button(ui, "Start", true).clicked() {
            *outcome = start(state, catalog);
        }
        if action_button(ui, "Cancel", false).clicked() {
            *outcome = Some(Outcome::Cancel);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::new_tab::AgentRequest;
    use egui::{Event, RawInput, Rect, pos2, vec2};
    use std::ffi::OsString;
    use std::path::PathBuf;
    use tidewatch_core::profiles::Profiles;

    struct Harness {
        ctx: egui::Context,
        state: NewTab,
        catalog: Catalog,
    }

    fn key(key: Key) -> Event {
        Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }
    }

    impl Harness {
        fn new(group: Group, preselect: Option<&str>) -> Self {
            Self::with(group, preselect, false)
        }

        fn with(group: Group, preselect: Option<&str>, workflows: bool) -> Self {
            let mut catalog = Catalog::new(OsString::from("/usr/bin"));
            catalog.set_profiles(Profiles::seeded(&["claude".to_owned()]));
            if workflows {
                catalog.set_workflows(tidewatch_core::workflows::Workflows::defaults());
            }
            let state = NewTab::open(
                group,
                "web".into(),
                PathBuf::from("/work/web"),
                &catalog,
                Vec::new(),
                preselect,
            );
            let mut harness = Self { ctx: egui::Context::default(), state, catalog };
            harness.frame(Vec::new());
            harness.frame(Vec::new());
            harness
        }

        fn frame(&mut self, events: Vec<Event>) -> Option<Outcome> {
            let raw = RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 700.0))),
                events,
                focused: true,
                ..RawInput::default()
            };
            let (state, catalog) = (&mut self.state, &self.catalog);
            let mut outcome = None;
            let _ = self.ctx.run_ui(raw, |ui| {
                outcome = show(ui.ctx(), state, catalog, None);
            });
            outcome
        }

        fn agent_request(outcome: Option<Outcome>) -> AgentRequest {
            match outcome {
                Some(Outcome::Start(Request::Agent(request))) => *request,
                _ => panic!("expected an agent request"),
            }
        }
    }

    #[test]
    fn enter_starts_the_selected_profile_at_its_own_level() {
        let mut harness = Harness::new(Group::Repo, None);
        let request = Harness::agent_request(harness.frame(vec![key(Key::Enter)]));
        assert_eq!(request.profile_id, "builder");
        assert_eq!(request.profile.level, Level::Edits);
        assert_eq!(request.task, "");
    }

    #[test]
    fn escape_cancels() {
        let mut harness = Harness::new(Group::Repo, None);
        assert!(matches!(harness.frame(vec![key(Key::Escape)]), Some(Outcome::Cancel)));
    }

    #[test]
    fn nothing_happens_without_a_key() {
        let mut harness = Harness::new(Group::Repo, None);
        assert!(harness.frame(Vec::new()).is_none());
    }

    #[test]
    fn typed_text_becomes_the_task_and_enter_in_the_field_starts() {
        let mut harness = Harness::new(Group::Repo, None);
        assert!(harness.frame(vec![Event::Text("fix the flaky test".into())]).is_none());
        let request = Harness::agent_request(harness.frame(vec![key(Key::Enter)]));
        assert_eq!(request.task, "fix the flaky test");
    }

    #[test]
    fn arrow_keys_pick_the_next_profile_and_the_one_after_is_shell_at_the_end() {
        let mut harness = Harness::new(Group::Repo, None);
        assert!(harness.frame(vec![key(Key::ArrowDown)]).is_none());
        let request = Harness::agent_request(harness.frame(vec![key(Key::Enter)]));
        assert_eq!(request.profile_id, "builder-yolo");
        assert_eq!(request.profile.level, Level::Yolo);

        let mut harness = Harness::new(Group::Repo, None);
        for _ in 0..5 {
            harness.frame(vec![key(Key::ArrowDown)]);
        }
        assert!(matches!(
            harness.frame(vec![key(Key::Enter)]),
            Some(Outcome::Start(Request::Shell))
        ));
    }

    #[test]
    fn in_other_enter_never_starts_a_yolo_agent() {
        let mut harness = Harness::new(Group::Other, Some("builder-yolo"));
        let request = Harness::agent_request(harness.frame(vec![key(Key::Enter)]));
        assert_ne!(request.profile.level, Level::Yolo);
        assert_eq!(request.profile.level, Level::Auto);
    }

    #[test]
    fn a_workflow_asks_for_a_task_and_starts_with_it() {
        let mut harness = Harness::with(Group::Repo, None, true);
        for _ in 0..3 {
            harness.frame(vec![key(Key::ArrowDown)]);
        }
        assert!(harness.frame(vec![key(Key::Enter)]).is_none(), "no task, no start");
        assert_eq!(harness.state.error.as_deref(), Some("A workflow needs a task to work from."));
        assert!(harness.frame(vec![Event::Text("add a retry".into())]).is_none());
        assert_eq!(harness.state.error, None, "typing clears the complaint");
        let Some(Outcome::Start(Request::Workflow(request))) = harness.frame(vec![key(Key::Enter)])
        else {
            panic!("expected a workflow request")
        };
        assert_eq!(request.workflow.name, "Ship a feature");
        assert_eq!(request.task, "add a retry");
        assert_eq!(request.first_agent, "claude");
    }

    #[test]
    fn no_workflow_is_offered_in_other() {
        let harness = Harness::with(Group::Other, None, true);
        assert!(harness.state.picks().iter().all(|pick| !matches!(pick, Pick::Workflow(_))));
    }

    #[test]
    fn the_dialog_stays_open_until_a_key_or_button_answers_it() {
        let mut harness = Harness::new(Group::Repo, None);
        for _ in 0..3 {
            assert!(harness.frame(Vec::new()).is_none());
        }
        assert!(matches!(harness.frame(vec![key(Key::Escape)]), Some(Outcome::Cancel)));
    }
}
