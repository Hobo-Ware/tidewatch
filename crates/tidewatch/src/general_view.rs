//! The General page: the Startup card (restore, open at login), the Window, Terminal and Updates
//! sections.

use egui::Ui;
use tidewatch_core::guards::GuardPrefs;
use tidewatch_core::prefs::{
    ChatPrefs, CloseEnter, DoneCheckPrefs, FETCH_MINUTES, GitPrefs, GithubPrefs, KeepAwakePrefs,
    NewTabPrefs, ParkAfter, ResumeAgents, TabsPrefs,
};

use crate::font_view;
use crate::github_view;
use crate::guards_view;
use crate::login_item;
use crate::macos::LoginItem;
use crate::new_tab_prefs_view::{self, ProfileOption};
use crate::overview_checklist::Checklist;
use crate::panel_blocks::OnTime;
use crate::screen_kit::{Size, quiet_button};
use crate::segmented;
use crate::settings_ui::{self, Control, footer_note, page_header, section, switch_row};
use crate::settings_view::Action;
use crate::sync_view;
use crate::touch_id_view::{self, TouchId};
use crate::updates::Updates;
use crate::updates_view;
use crate::window_settings;
use crate::window_view;
use crate::workflow_view;

pub(crate) struct General {
    pub(crate) restore: bool,
    pub(crate) resume: ResumeAgents,
    pub(crate) skipped_by_flag: bool,
    pub(crate) login_offered: bool,
    pub(crate) window: window_view::View,
    pub(crate) github: GithubPrefs,
    pub(crate) new_tab: NewTabPrefs,
    pub(crate) tabs: TabsPrefs,
    pub(crate) git: GitPrefs,
    pub(crate) done_check: DoneCheckPrefs,
    pub(crate) chat: ChatPrefs,
    pub(crate) keep_awake: KeepAwakePrefs,
    pub(crate) budget: BudgetState,
    /// `None` where Amphetamine has no lid mode (Linux): the row is not offered.
    pub(crate) touch_id: Option<TouchId>,
    pub(crate) guards: GuardPrefs,
    /// The repo on show, when it is one: a guard rule can be set for it alone.
    pub(crate) guards_repo: Option<guards_view::Repo>,
    pub(crate) profiles: Vec<ProfileOption>,
    /// The active repo's remote default branch, for the Start from label.
    pub(crate) default_branch: Option<String>,
    pub(crate) terminal: font_view::View,
    pub(crate) sync: sync_view::View,
    /// The must-do list of the repo on show, once it is read.
    pub(crate) checklist: Option<Checklist>,
    /// The workflow profile of the repo on show, when it is one.
    pub(crate) workflow: Option<workflow_view::View>,
    pub(crate) on_time: OnTime,
}

pub(crate) fn page(
    ui: &mut Ui,
    general: &General,
    window: &mut window_settings::Editor,
    font: &mut font_view::Picker,
    updates: &mut Updates,
    login: &mut login_item::Row,
    sync: &mut sync_view::Editor,
    read_login: impl FnOnce() -> LoginItem,
    file: &str,
) -> Option<Action> {
    page_header(ui, "General", "Saved as you change them");
    let mut restore = None;
    let mut resume = None;
    let mut open_at_login = None;
    section(ui, "Startup", |ui| {
        restore = switch_row(
            ui,
            "Restore the last session",
            "Tabs reopen where they were and agents pick up their conversation.",
            general.restore,
        );
        resume = resume_row(ui, general.resume, general.restore);
        if general.login_offered {
            let now = ui.ctx().input(|input| input.time);
            open_at_login = login_item::show(ui, login, now, read_login);
        }
    });
    if general.skipped_by_flag {
        ui.add_space(8.0);
        crate::settings_ui::caption(
            ui,
            "This run was started with --no-restore, so nothing was restored.",
        );
    }
    let window = window_view::section(ui, window, &general.window).map(Action::Window);
    let git = git_card(ui, general.git).map(Action::Git);
    let terminal = font_view::section(ui, font, &general.terminal).map(|change| match change {
        font_view::Change::Font(choice) => Action::TerminalFont(choice),
        font_view::Change::CopyOnSelect(on) => Action::TerminalCopyOnSelect(on),
    });
    let new_tab = new_tab_prefs_view::card(
        ui,
        &general.new_tab,
        &general.profiles,
        general.default_branch.as_deref(),
    )
    .map(Action::NewTab);
    let tabs = tabs_card(ui, general.tabs).map(Action::Tabs);
    let done_check = done_check_card(ui, general.done_check.enabled).map(Action::DoneCheck);
    let chat = chat_card(ui, general.chat.animate_emoji).map(Action::AnimateEmoji);
    let keep_awake = keep_awake_card(ui, general.keep_awake.enabled, general.touch_id).map(
        |change| match change {
            KeepAwakeChange::Switch(on) => Action::KeepAwake(on),
            KeepAwakeChange::TouchId(click) => Action::TouchId(click),
        },
    );
    let budget = budget_card(ui, general.budget).map(Action::SaveMode);
    let checklist = checklist_card(ui, general.checklist.as_ref());
    let github = github_view::card(ui, general.github).map(Action::Github);
    let workflow =
        workflow_view::settings_card(ui, general.workflow.as_ref()).and_then(|workflow| {
            let repo = general.workflow.as_ref()?.path.clone();
            Some(Action::Workflow { repo, workflow })
        });
    let guards =
        guards_view::card(ui, &general.guards, general.guards_repo.as_ref()).map(Action::Guards);
    let sync = sync_view::card(ui, sync, &general.sync).map(Action::Sync);
    let updates = updates_view::section(ui, updates).map(Action::Updates);
    about_card(ui, &general.on_time);
    footer_note(ui, file);
    restore
        .map(Action::Restore)
        .or(resume.map(Action::ResumeAgents))
        .or(open_at_login.map(Action::LoginItem))
        .or(window)
        .or(git)
        .or(terminal)
        .or(new_tab)
        .or(tabs)
        .or(done_check)
        .or(chat)
        .or(checklist)
        .or(keep_awake)
        .or(budget)
        .or(github)
        .or(workflow)
        .or(guards)
        .or(sync)
        .or(updates)
}

pub(crate) const EMOJI_CREDIT: &str =
    "Emoji pictures are Google Noto Emoji, used under the Apache 2.0 license.";
pub(crate) const ICON_CREDIT: &str =
    "Tab icons agents pick are Lucide, used under the ISC license.";

fn chat_card(ui: &mut Ui, on: bool) -> Option<bool> {
    let mut changed = None;
    section(ui, "Chat", |ui| {
        changed = switch_row(
            ui,
            "Play animated emoji",
            "A new message plays its emoji once, then they rest. Point at one to play it again. It stays still when macOS asks for reduced motion.",
            on,
        );
    });
    changed
}

fn about_card(ui: &mut Ui, on_time: &OnTime) {
    section(ui, "About", |ui| {
        settings_ui::row(ui, "Emoji", EMOJI_CREDIT, Control::NONE, |_| {});
        settings_ui::row(ui, "Icons", ICON_CREDIT, Control::NONE, |_| {});
        let value = match on_time.late.as_deref() {
            Some(late) => format!("{}. {late}", on_time.line),
            None => on_time.line.clone(),
        };
        settings_ui::row(ui, "Panel on time", &value, Control::NONE, |_| {});
    });
}

fn git_card(ui: &mut Ui, current: GitPrefs) -> Option<GitPrefs> {
    let mut changed = None;
    section(ui, "Git", |ui| {
        if let Some(on) = switch_row(
            ui,
            "Fetch in the background",
            "Keeps behind and ahead counts current without touching your files.",
            current.background_fetch,
        ) {
            changed = Some(GitPrefs { background_fetch: on, ..current });
        }
        if let Some(minutes) = fetch_every_row(ui, current) {
            changed = Some(GitPrefs { fetch_minutes: minutes, ..current });
        }
    });
    changed
}

fn fetch_every_row(ui: &mut Ui, current: GitPrefs) -> Option<u32> {
    let labels: Vec<String> =
        FETCH_MINUTES.iter().map(|minutes| format!("{minutes} min")).collect();
    let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
    let active = FETCH_MINUTES.iter().position(|minutes| *minutes == current.fetch_minutes);
    settings_ui::row(
        ui,
        "Every",
        "How often each open repo is fetched.",
        Control::new(settings_ui::inline_width(ui, &labels), 28.0),
        |ui| {
            ui.add_enabled_ui(current.background_fetch, |ui| {
                let id = ui.id().with("fetch-every");
                segmented::inline(ui, id, &labels, active.unwrap_or(usize::MAX))
                    .map(|index| FETCH_MINUTES[index])
            })
            .inner
        },
    )
}

fn tabs_card(ui: &mut Ui, current: TabsPrefs) -> Option<TabsPrefs> {
    let labels = ParkAfter::ALL.map(ParkAfter::label);
    let active = ParkAfter::ALL.iter().position(|after| *after == current.park_after);
    let enter_labels = CloseEnter::ALL.map(CloseEnter::label);
    let enter_active = CloseEnter::ALL.iter().position(|enter| *enter == current.close_enter);
    let mut picked = None;
    let mut enter = None;
    section(ui, "Tabs", |ui| {
        picked = settings_ui::row(
            ui,
            "Park quiet tabs after",
            "A tab nobody touched for this long leaves the strip and waits under Parked. It keeps running.",
            Control::new(settings_ui::inline_width(ui, &labels), 28.0),
            |ui| settings_ui::pick(ui, "park-after", &labels, active),
        );
        enter = settings_ui::row(
            ui,
            "When closing a tab, Enter",
            "The close dialog always opens. This picks the button Enter presses in it, unless the worktree has uncommitted changes.",
            Control::new(settings_ui::inline_width(ui, &enter_labels), 28.0),
            |ui| settings_ui::pick(ui, "close-enter", &enter_labels, enter_active),
        );
    });
    let park_after = picked.map_or(current.park_after, |index| ParkAfter::ALL[index]);
    let close_enter = enter.map_or(current.close_enter, |index| CloseEnter::ALL[index]);
    (picked.is_some() || enter.is_some()).then_some(TabsPrefs { park_after, close_enter })
}

fn checklist_card(ui: &mut Ui, list: Option<&Checklist>) -> Option<Action> {
    let desc = list.map_or_else(
        || "Reading the list.".to_owned(),
        |list| {
            let steps: Vec<&str> = list.chips.iter().map(|chip| chip.title.as_str()).collect();
            format!(
                "{}: {}. Agents keep these as steps in their progress report.",
                list.source.caption(),
                steps.join(", ")
            )
        },
    );
    let mut edit = false;
    if std::env::var_os("TIDEWATCH_DEV_SCROLL_CHECKLIST").is_some() {
        ui.scroll_to_cursor(Some(egui::Align::TOP));
    }
    section(ui, "Checklist", |ui| {
        edit = settings_ui::row(
            ui,
            "Must-do steps",
            &desc,
            Control::button(ui, "Edit", Size::Compact),
            |ui| {
                let response = quiet_button(ui, "Edit", list.is_some(), Size::Compact);
                if let Some(list) = list {
                    response.clone().on_hover_text(list.file.display().to_string());
                }
                response.clicked()
            },
        );
    });
    edit.then_some(Action::EditChecklist)
}

pub(crate) fn done_check_card(ui: &mut Ui, on: bool) -> Option<bool> {
    let mut changed = None;
    section(ui, "Done check", |ui| {
        changed = switch_row(
            ui,
            "Check the work when an agent says it is done",
            "Looks at whether the work is committed, pushed and its pull request is green, and shows Done, Blocked or Not verified on the tab. It never changes anything.",
            on,
        );
    });
    changed
}

/// What the Token budget card shows: the save mode switch, and whether any plan limit reading
/// exists (without one the card says save mode has nothing to watch).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BudgetState {
    pub(crate) save_mode: bool,
    pub(crate) limits_known: bool,
}

pub(crate) fn budget_card(ui: &mut Ui, state: BudgetState) -> Option<bool> {
    let mut changed = None;
    if std::env::var_os("TIDEWATCH_DEV_SCROLL_BUDGET").is_some() {
        ui.scroll_to_cursor(Some(egui::Align::TOP));
    }
    section(ui, "Token budget", |ui| {
        changed = switch_row(
            ui,
            "Save mode near a plan limit",
            "When a plan limit passes 80%, Tidewatch holds back what it sends on its own: Auto fixes, reminders and checks, and it starts no new Meeseeks. What you send and the agents already working are not touched. It tells you once when it starts.",
            state.save_mode,
        );
        let caption = if state.limits_known {
            "The tokens of each tab show on its tooltip and on the Overview."
        } else {
            "Claude Code does not give its plan limits to other programs yet, so save mode has nothing to watch. The tokens of each tab show on its tooltip and on the Overview."
        };
        settings_ui::card_caption(ui, caption);
    });
    changed
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum KeepAwakeChange {
    Switch(bool),
    TouchId(touch_id_view::Click),
}

pub(crate) fn keep_awake_card(
    ui: &mut Ui,
    on: bool,
    touch_id: Option<TouchId>,
) -> Option<KeepAwakeChange> {
    let mut changed = None;
    if std::env::var_os("TIDEWATCH_DEV_SCROLL_KEEP_AWAKE").is_some() {
        ui.scroll_to_cursor(Some(egui::Align::TOP));
    }
    section(ui, "Keep awake", |ui| {
        changed = switch_row(
            ui,
            "Keep the computer awake while agents work",
            "Stops it from going to sleep on its own while any agent is working, so a long turn is not frozen. The screen can still turn off and lock. It lets go two minutes after the last agent stops or waits for you.",
            on,
        )
        .map(KeepAwakeChange::Switch);
        if let Some(state) = touch_id {
            changed = touch_id_view::show(ui, state).map(KeepAwakeChange::TouchId).or(changed);
        }
        settings_ui::card_caption(
            ui,
            "Amphetamine, the button at the right end of the tab strip, keeps it awake for a set time, lid closed included on a Mac.",
        );
    });
    changed
}

fn resume_row(ui: &mut Ui, current: ResumeAgents, enabled: bool) -> Option<ResumeAgents> {
    let options = [
        (ResumeAgents::Auto, "Resume"),
        (ResumeAgents::Prefill, "Type only"),
        (ResumeAgents::Off, "Off"),
    ];
    let labels = options.map(|(_, label)| label);
    let active = options.iter().position(|(mode, _)| *mode == current);
    settings_ui::row(
        ui,
        "Resume agents",
        "Resume runs the agent's command. Type only leaves Enter to you. After repeated crashes it is always typed.",
        Control::new(settings_ui::inline_width(ui, &labels), 28.0),
        |ui| {
            ui.add_enabled_ui(enabled, |ui| {
                settings_ui::pick(ui, "resume", &labels, active).map(|index| options[index].0)
            })
            .inner
        },
    )
}

#[cfg(test)]
mod tests {
    use egui::{Event, Modifiers, PointerButton, RawInput, Rect, Vec2, pos2};

    use super::*;

    fn frame(ctx: &egui::Context, git: GitPrefs, events: Vec<Event>) -> Option<GitPrefs> {
        let mut answer = None;
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), Vec2::new(900.0, 400.0))),
            focused: true,
            events,
            ..RawInput::default()
        };
        let _ = ctx.run_ui(raw, |ui| answer = git_card(ui, git));
        answer
    }

    fn press(at: egui::Pos2, pressed: bool) -> Event {
        Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        }
    }

    fn every_click(git: GitPrefs) -> Vec<GitPrefs> {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::fonts::definitions());
        frame(&ctx, git, vec![]);
        let mut found = Vec::new();
        for y in (0..220).step_by(6) {
            for x in (0..900).step_by(6) {
                let at = pos2(x as f32, y as f32);
                frame(&ctx, git, vec![Event::PointerMoved(at)]);
                frame(&ctx, git, vec![press(at, true)]);
                found.extend(frame(&ctx, git, vec![press(at, false)]));
            }
        }
        found
    }

    #[test]
    fn the_git_card_flips_the_switch_and_picks_an_interval() {
        let on = GitPrefs::default();
        let found = every_click(on);
        assert!(found.contains(&GitPrefs { background_fetch: false, ..on }));
        for minutes in FETCH_MINUTES.into_iter().filter(|minutes| *minutes != on.fetch_minutes) {
            assert!(found.contains(&GitPrefs { fetch_minutes: minutes, ..on }), "{minutes}");
        }
    }

    #[test]
    fn the_interval_cannot_be_picked_while_the_switch_is_off() {
        let off = GitPrefs { background_fetch: false, ..GitPrefs::default() };
        let found = every_click(off);
        assert!(found.contains(&GitPrefs { background_fetch: true, ..off }));
        assert!(found.iter().all(|git| git.fetch_minutes == off.fetch_minutes), "{found:?}");
    }

    #[test]
    fn the_tabs_card_picks_what_enter_does_on_close_and_keeps_the_park_time() {
        let current =
            TabsPrefs { park_after: ParkAfter::Day, close_enter: CloseEnter::DeleteBranch };
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::fonts::definitions());
        let frame = |events: Vec<Event>| {
            let mut answer = None;
            let raw = RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), Vec2::new(900.0, 400.0))),
                focused: true,
                events,
                ..RawInput::default()
            };
            let _ = ctx.run_ui(raw, |ui| answer = tabs_card(ui, current));
            answer
        };
        frame(vec![]);
        let mut found = Vec::new();
        for y in (0..260).step_by(6) {
            for x in (0..900).step_by(6) {
                let at = pos2(x as f32, y as f32);
                frame(vec![Event::PointerMoved(at)]);
                frame(vec![press(at, true)]);
                found.extend(frame(vec![press(at, false)]));
            }
        }
        let keep = TabsPrefs { close_enter: CloseEnter::KeepBranch, ..current };
        assert!(found.contains(&keep), "{found:?}");
    }
}

#[cfg(test)]
mod checklist_tests {
    use super::*;
    use crate::overview_checklist::{Chip, Source};
    use egui::{Event, PointerButton, RawInput, Rect, pos2, vec2};

    fn walk(shape: &egui::Shape, found: &mut Vec<(String, Rect)>) {
        match shape {
            egui::Shape::Text(text) if !text.galley.text().trim().is_empty() => found.push((
                text.galley.text().to_owned(),
                text.galley.rect.translate(text.pos.to_vec2()),
            )),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| walk(shape, found)),
            _ => {}
        }
    }

    fn frame(
        ctx: &egui::Context,
        list: Option<&Checklist>,
        events: Vec<Event>,
    ) -> (Option<Action>, Vec<(String, Rect)>) {
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 500.0))),
            events,
            focused: true,
            ..RawInput::default()
        };
        let mut action = None;
        let output = ctx.run_ui(raw, |ui| action = checklist_card(ui, list));
        let mut found = Vec::new();
        for clipped in &output.shapes {
            walk(&clipped.shape, &mut found);
        }
        (action, found)
    }

    #[test]
    fn the_checklist_card_names_the_steps_and_edit_asks_to_open_the_file() {
        let ctx = crate::fonts::test_context();
        let list = Checklist {
            chips: vec![
                Chip { number: Some(1), title: "Understand".into() },
                Chip { number: Some(2), title: "Build".into() },
            ],
            source: Source::Default,
            file: "/cfg/checklist.md".into(),
        };
        frame(&ctx, Some(&list), vec![]);
        let found = frame(&ctx, Some(&list), vec![]).1;
        let has = |want: &str| found.iter().any(|(text, _)| text == want);
        assert!(has("Checklist") && has("Must-do steps") && has("Edit"));
        assert!(found.iter().any(|(text, _)| text.starts_with("Four stages \u{b7} default: Understand, Build.")));
        let at = found.iter().find(|(text, _)| text == "Edit").unwrap().1.center();
        let button = |pressed| Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(&ctx, Some(&list), vec![Event::PointerMoved(at)]);
        frame(&ctx, Some(&list), vec![button(true)]);
        let (action, _) = frame(&ctx, Some(&list), vec![button(false)]);
        assert!(matches!(action, Some(Action::EditChecklist)));
    }

    #[test]
    fn before_the_list_is_read_the_card_says_so_and_offers_no_edit() {
        let ctx = crate::fonts::test_context();
        frame(&ctx, None, vec![]);
        let (action, found) = frame(&ctx, None, vec![]);
        assert!(action.is_none());
        assert!(found.iter().any(|(text, _)| text == "Reading the list."));
    }
}

#[cfg(test)]
mod chat_tests {
    use egui::{Event, Modifiers, PointerButton, RawInput, Rect, pos2, vec2};

    use super::*;

    fn frame(ctx: &egui::Context, on: bool, events: Vec<Event>) -> Option<bool> {
        let mut answer = None;
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 300.0))),
            focused: true,
            events,
            ..RawInput::default()
        };
        let _ = ctx.run_ui(raw, |ui| answer = chat_card(ui, on));
        answer
    }

    fn press(at: egui::Pos2, pressed: bool) -> Event {
        Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        }
    }

    fn clicks(on: bool) -> Vec<bool> {
        let ctx = crate::fonts::test_context();
        frame(&ctx, on, vec![]);
        let mut found = Vec::new();
        for y in (0..160).step_by(6) {
            for x in (0..900).step_by(6) {
                let at = pos2(x as f32, y as f32);
                frame(&ctx, on, vec![Event::PointerMoved(at)]);
                frame(&ctx, on, vec![press(at, true)]);
                found.extend(frame(&ctx, on, vec![press(at, false)]));
            }
        }
        found
    }

    #[test]
    fn the_chat_card_turns_animated_emoji_off_and_back_on() {
        let off = clicks(true);
        assert!(!off.is_empty() && off.iter().all(|value| !value), "{off:?}");
        let on = clicks(false);
        assert!(!on.is_empty() && on.iter().all(|value| *value), "{on:?}");
    }
}

#[cfg(test)]
mod about_tests {
    use egui::{RawInput, Rect, pos2, vec2};

    use super::*;

    fn walk(shape: &egui::Shape, texts: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(text) => texts.push(text.galley.text().to_owned()),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| walk(shape, texts)),
            _ => {}
        }
    }

    #[test]
    fn the_about_card_credits_google_noto_emoji_and_its_license() {
        let ctx = crate::fonts::test_context();
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 400.0))),
            ..RawInput::default()
        };
        let on_time = OnTime::none();
        let _ = ctx.run_ui(raw.clone(), |ui| about_card(ui, &on_time));
        let output = ctx.run_ui(raw, |ui| about_card(ui, &on_time));
        let mut texts = Vec::new();
        for clipped in &output.shapes {
            walk(&clipped.shape, &mut texts);
        }
        assert!(texts.iter().any(|text| text == "About"), "{texts:?}");
        assert!(
            texts.iter().any(|text| text == EMOJI_CREDIT
                && text.contains("Google Noto Emoji")
                && text.contains("Apache 2.0")),
            "{texts:?}"
        );
        assert!(
            texts
                .iter()
                .any(|text| text == ICON_CREDIT && text.contains("Lucide") && text.contains("ISC")),
            "{texts:?}"
        );
    }

    fn about_texts(on_time: &OnTime) -> Vec<String> {
        let ctx = crate::fonts::test_context();
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 400.0))),
            ..RawInput::default()
        };
        let _ = ctx.run_ui(raw.clone(), |ui| about_card(ui, on_time));
        let output = ctx.run_ui(raw, |ui| about_card(ui, on_time));
        let mut texts = Vec::new();
        for clipped in &output.shapes {
            walk(&clipped.shape, &mut texts);
        }
        texts
    }

    #[test]
    fn the_about_card_says_how_much_of_the_week_the_panel_was_on_time() {
        let late = OnTime::fake(99.96);
        let texts = about_texts(&late);
        assert!(texts.contains(&"Panel on time".to_owned()), "{texts:?}");
        assert!(texts.contains(&"99.96% this week. Late: hooks 3".to_owned()), "{texts:?}");
    }

    #[test]
    fn the_about_card_has_no_late_line_when_nothing_was_late_and_says_so_with_no_changes() {
        let perfect = about_texts(&OnTime::fake(100.0));
        assert!(perfect.contains(&"Panel on time".to_owned()), "{perfect:?}");
        assert!(perfect.contains(&"100% this week".to_owned()), "{perfect:?}");
        assert!(!perfect.iter().any(|text| text.starts_with("Late:")), "{perfect:?}");
        let none = about_texts(&OnTime::none());
        assert!(none.contains(&"no changes yet this week".to_owned()), "{none:?}");
        assert!(!none.iter().any(|text| text.starts_with("Late:")), "{none:?}");
    }
}
