use egui::{Context, RawInput, Rect, pos2, vec2};

use super::*;

fn frame(ctx: &Context, prefs: &GuardPrefs, repo: Option<&Repo>) -> Option<Change> {
    let raw = RawInput {
        screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 900.0))),
        ..RawInput::default()
    };
    let mut changed = None;
    let _ = ctx.run_ui(raw, |ui| changed = card(ui, prefs, repo));
    changed
}

fn context() -> Context {
    let ctx = Context::default();
    crate::fonts::install(&ctx);
    ctx
}

fn repo() -> Repo {
    Repo { path: "/work/web".into(), label: "web".into() }
}

#[test]
fn the_card_draws_headless_for_every_state_and_changes_nothing_by_itself() {
    let mut busy = GuardPrefs::default();
    busy.rules.set(Rule::Dashes, false);
    busy.rules.set(Rule::DraftPrs, true);
    busy.private_repos = vec!["acme/vault".into(), "ledger-api".into()];
    busy.set_rule(Some(std::path::Path::new("/work/web")), Rule::RerunFailed, false);
    for prefs in [GuardPrefs::default(), busy] {
        for repo in [None, Some(repo())] {
            let ctx = context();
            assert_eq!(frame(&ctx, &prefs, repo.as_ref()), None);
            assert_eq!(frame(&ctx, &prefs, repo.as_ref()), None);
        }
    }
}

#[test]
fn the_card_lists_every_rule_with_its_title_and_its_reason() {
    let ctx = context();
    let raw = RawInput {
        screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 900.0))),
        ..RawInput::default()
    };
    let output = ctx.run_ui(raw, |ui| {
        let _ = card(ui, &GuardPrefs::default(), None);
    });
    let shown: String = output
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            egui::epaint::Shape::Text(text) => Some(text.galley.text().to_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    for rule in Rule::ALL {
        assert!(shown.contains(rule.title()), "{} missing", rule.title());
        assert!(shown.contains(rule.reason()), "{} missing", rule.reason());
    }
    assert!(shown.contains("Agents push only their own branch"));
}

#[test]
fn a_change_sets_the_default_a_repo_answer_or_the_private_list() {
    let mut guards = GuardPrefs::default();
    Change::Rule { repo: None, rule: Rule::DraftPrs, on: true }.apply(&mut guards);
    Change::Rule { repo: Some("/work/web".into()), rule: Rule::Dashes, on: false }
        .apply(&mut guards);
    Change::PrivateRepos(vec!["acme/vault".into()]).apply(&mut guards);
    assert!(guards.rules.draft_prs && guards.rules.dashes);
    assert!(!guards.rules_for(std::path::Path::new("/work/web")).dashes);
    assert_eq!(guards.private_repos, ["acme/vault"]);
}
