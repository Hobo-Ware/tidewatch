use std::collections::HashSet;

use egui::{Event, RawInput, Rect, pos2, vec2};

use super::*;

const PLATFORMS: [Platform; 2] = [Platform::Mac, Platform::Linux];
const TAB_SHORTCUTS: usize = 9;

fn bound_to_a_chord(action: Action) -> bool {
    match action {
        Action::New
        | Action::CloseActive
        | Action::SelectNth(_)
        | Action::NextGroup
        | Action::PrevGroup => true,
        Action::Close(_) | Action::Select(_) | Action::ShowGroup(_) => false,
    }
}

fn press(platform: Platform, chord: Chord) -> Vec<Event> {
    vec![Event::Key {
        key: chord.key,
        physical_key: Some(chord.key),
        pressed: true,
        repeat: false,
        modifiers: chord.held(platform),
    }]
}

fn pressed_with(
    platform: Platform,
    events: Vec<Event>,
    wanted: impl Fn(Command) -> bool,
) -> Vec<Command> {
    let ctx = egui::Context::default();
    let raw = RawInput {
        screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0))),
        events,
        ..Default::default()
    };
    let mut commands = Vec::new();
    let _ = ctx.run_ui(raw, |ui| commands = consume(ui.ctx(), platform, &wanted));
    commands
}

#[test]
fn no_two_commands_share_an_id_or_a_title() {
    let ids: HashSet<_> = TABLE.iter().map(|entry| entry.id).collect();
    let titles: HashSet<_> = TABLE.iter().map(|entry| entry.title).collect();
    assert_eq!(ids.len(), TABLE.len());
    assert_eq!(titles.len(), TABLE.len());
}

#[test]
fn every_command_has_its_own_chord_on_each_platform() {
    for platform in PLATFORMS {
        let chords: HashSet<_> = TABLE.iter().map(|entry| entry.chord(platform)).collect();
        assert_eq!(chords.len(), TABLE.len(), "{platform:?}");
        let labels: HashSet<_> = TABLE.iter().map(|entry| entry.chord(platform).label()).collect();
        assert_eq!(labels.len(), TABLE.len(), "{platform:?}");
    }
}

#[test]
fn a_press_runs_exactly_its_own_command_even_when_a_shorter_chord_shares_the_key() {
    for platform in PLATFORMS {
        for entry in &TABLE {
            let got = pressed_with(platform, press(platform, entry.chord(platform)), |_| true);
            assert_eq!(got, [entry.command], "{platform:?} {}", entry.chord(platform).label());
        }
    }
}

#[test]
fn every_action_that_has_a_chord_is_in_the_table() {
    let actions =
        [Action::New, Action::CloseActive, Action::Close(0), Action::Select(0), Action::NextGroup]
            .into_iter()
            .chain([Action::PrevGroup])
            .chain((0..TAB_SHORTCUTS).map(Action::SelectNth));
    for action in actions {
        let listed = TABLE.iter().any(|entry| entry.command.action() == Some(action));
        assert_eq!(listed, bound_to_a_chord(action), "{action:?}");
    }
    let tabs = TABLE.iter().filter(|entry| matches!(entry.command, Command::SelectTab(_))).count();
    assert_eq!(tabs, TAB_SHORTCUTS);
}

#[test]
fn macos_chords_are_the_ones_the_app_shipped_with() {
    let shipped = [
        ("new-tab", "Cmd+T"),
        ("close-tab", "Cmd+W"),
        ("tab-1", "Cmd+1"),
        ("tab-9", "Cmd+9"),
        ("next-repo", "Cmd+Ctrl+]"),
        ("prev-repo", "Cmd+Ctrl+["),
        ("overview", "Cmd+0"),
        ("focus", "Cmd+."),
        ("reader", "Cmd+Shift+T"),
        ("settings", "Cmd+,"),
        ("approve-delivery", "Cmd+Enter"),
    ];
    for (id, keys) in shipped {
        let entry = TABLE.iter().find(|entry| entry.id == id).unwrap();
        assert_eq!(entry.chord(Platform::Mac).label(), keys, "{id}");
    }
}

#[test]
fn linux_never_binds_a_plain_ctrl_letter_or_digit_the_shell_needs() {
    let no_control_code = [Key::Comma, Key::Period, Key::Enter];
    for entry in &TABLE {
        let chord = entry.chord(Platform::Linux);
        assert!(!chord.has(CMD), "{}: Cmd does not exist on Linux", entry.id);
        if chord.mods == CTRL {
            assert!(no_control_code.contains(&chord.key), "{}: {}", entry.id, chord.label());
        }
    }
}

#[test]
fn a_real_ctrl_press_on_macos_is_not_a_command_chord() {
    let ctrl_t = ctrl(Key::T);
    assert_eq!(
        pressed_with(Platform::Mac, press(Platform::Mac, ctrl_t), |_| true),
        Vec::<Command>::new()
    );
}

#[test]
fn a_filtered_command_leaves_its_press_for_whoever_reads_it_next() {
    let enter = TABLE.iter().find(|entry| entry.command == Command::ApproveDelivery).unwrap();
    for platform in PLATFORMS {
        let events = press(platform, enter.chord(platform));
        assert_eq!(pressed_with(platform, events.clone(), Command::global), Vec::<Command>::new());
        let only = |command| command == Command::ApproveDelivery;
        assert_eq!(pressed_with(platform, events, only), [Command::ApproveDelivery]);
    }
}

#[test]
fn labels_follow_the_platform() {
    let reader = TABLE.iter().find(|entry| entry.command == Command::Reader).unwrap();
    assert_eq!(reader.chord(Platform::Mac).label(), "Cmd+Shift+T");
    assert_eq!(reader.chord(Platform::Linux).label(), "Ctrl+Shift+R");
    assert_eq!(Command::Reader.keys(), reader.chord(Platform::CURRENT).label());
}

#[test]
fn the_nine_tab_chords_make_one_row() {
    let mac = rows(Platform::Mac);
    let tabs: Vec<_> = mac.iter().filter(|row| row.title.starts_with("Switch")).collect();
    assert_eq!(tabs.len(), 1);
    assert_eq!(tabs[0].keys, "Cmd+1 to 9");
    let linux = rows(Platform::Linux);
    assert_eq!(
        linux.iter().find(|row| row.title.starts_with("Switch")).unwrap().keys,
        "Alt+1 to 9"
    );
    assert_eq!(mac.len(), linux.len());
}

#[test]
fn the_palette_lists_what_a_person_would_look_for() {
    let listed: Vec<_> =
        TABLE.iter().filter(|entry| entry.command.in_palette()).map(|entry| entry.id).collect();
    assert_eq!(
        listed,
        [
            "new-tab",
            "close-tab",
            "next-repo",
            "prev-repo",
            "overview",
            "focus",
            "reader",
            "settings"
        ]
    );
}

#[test]
fn the_palette_chord_is_cmd_shift_p_and_ctrl_shift_p() {
    assert_eq!(Command::Palette.keys_on(Platform::Mac), "Cmd+Shift+P");
    assert_eq!(Command::Palette.keys_on(Platform::Linux), "Ctrl+Shift+P");
}

#[test]
fn every_command_knows_its_id() {
    for entry in &TABLE {
        assert_eq!(entry.command.id(), entry.id);
    }
}

#[test]
fn shortcuts_doc() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/shortcuts.md");
    let wanted = markdown();
    if std::env::var_os("TIDEWATCH_BLESS").is_some() {
        std::fs::write(&path, &wanted).unwrap();
    }
    let found = std::fs::read_to_string(&path).unwrap_or_default();
    assert_eq!(found, wanted, "docs/shortcuts.md is out of date: run with TIDEWATCH_BLESS=1");
}

mod gate {
    use super::*;

    const NONE: Gate =
        Gate { dialog: Dialog::None, settings: false, overview: false, palette: false };

    fn allowed(gate: Gate) -> Vec<Command> {
        TABLE.iter().map(|entry| entry.command).filter(|command| gate.allows(*command)).collect()
    }

    #[test]
    fn an_open_workspace_takes_every_global_chord() {
        let all: Vec<_> = TABLE.iter().map(|e| e.command).filter(|c| c.global()).collect();
        assert_eq!(allowed(NONE), all);
    }

    #[test]
    fn the_overview_keeps_its_tab_chords_but_not_close() {
        let got = allowed(Gate { overview: true, ..NONE });
        assert!(got.contains(&Command::NewTab));
        assert!(!got.contains(&Command::CloseTab));
        assert!(!got.contains(&Command::Reader));
    }

    #[test]
    fn settings_can_close_itself_and_reach_the_palette() {
        let got = allowed(Gate { settings: true, ..NONE });
        assert!(got.contains(&Command::Settings));
        assert!(got.contains(&Command::Palette));
        assert!(!got.contains(&Command::NewTab));
    }

    #[test]
    fn the_new_tab_dialog_and_the_close_dialog_take_only_their_own_keys() {
        let got = allowed(Gate { dialog: Dialog::TabChoice, ..NONE });
        assert_eq!(got, Vec::<Command>::new());
    }

    #[test]
    fn an_open_palette_takes_nothing_but_its_own_chord() {
        let open = Gate { palette: true, ..NONE };
        assert_eq!(allowed(open), [Command::Palette]);
        assert!(open.modal_open());
    }

    #[test]
    fn a_dialog_keeps_the_palette_closed() {
        let got = allowed(Gate { dialog: Dialog::Other, ..NONE });
        assert!(!got.contains(&Command::Palette));
    }
}
