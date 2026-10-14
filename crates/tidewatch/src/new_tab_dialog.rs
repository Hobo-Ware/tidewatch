//! Draws the new-tab dialog's frame and first screen.

use egui::{
    Align2, Color32, CornerRadius, Frame, Id, Key, Margin, Modifiers, Rect, Response, Shape,
    Stroke, Ui, Vec2, pos2, vec2,
};
use stdusk_egui::colors;

use crate::catalog::Catalog;
use crate::new_tab::{Group, Kind, NewTab, Request, Step};
use crate::new_tab_pick;
use crate::screen_kit::{ORANGE, body, card_stroke, clickable, heading, mono, paint_key_cap};

pub(super) const DIALOG_W: f32 = 720.0;
const SIDE_PAD: i8 = 28;
const CARD_H: f32 = 172.0;
const CARD_GAP: f32 = 14.0;
const ICON_BOX: f32 = 44.0;

pub(crate) enum Outcome {
    Start(Request),
    Cancel,
}

pub(crate) fn show(ctx: &egui::Context, state: &mut NewTab, catalog: &Catalog) -> Option<Outcome> {
    let mut outcome = keys(ctx, state, catalog);
    let top = if state.step() == Step::Choose { 150.0 } else { 84.0 };
    let id = Id::new("new_tab");
    let area = egui::Modal::default_area(id).anchor(Align2::CENTER_TOP, [0.0, top]);
    let frame = Frame::new()
        .fill(colors::elevated())
        .stroke(Stroke::new(1.0, colors::border()))
        .corner_radius(18)
        .shadow(egui::epaint::Shadow {
            offset: [0, 8],
            blur: 28,
            spread: 0,
            color: Color32::from_black_alpha(120),
        })
        .inner_margin(Margin::symmetric(SIDE_PAD, 24));
    let width = DIALOG_W.min(ctx.content_rect().width() - 40.0) - 2.0 * f32::from(SIDE_PAD);
    let modal = egui::Modal::new(id).area(area).frame(frame).show(ctx, |ui| {
        ui.set_width(width);
        ui.spacing_mut().item_spacing.y = 0.0;
        match state.step() {
            Step::Choose => choose(ui, state, catalog, &mut outcome),
            Step::Agent => new_tab_pick::agent(ui, state, catalog, &mut outcome),
            Step::Workflow => new_tab_pick::workflow(ui, state, catalog, &mut outcome),
        }
    });
    if outcome.is_none() && modal.backdrop_response.clicked() {
        outcome = Some(Outcome::Cancel);
    }
    outcome
}

fn pressed(ctx: &egui::Context, key: Key) -> bool {
    ctx.input_mut(|input| input.consume_key(Modifiers::NONE, key))
}

fn keys(ctx: &egui::Context, state: &mut NewTab, catalog: &Catalog) -> Option<Outcome> {
    if pressed(ctx, Key::Escape) {
        return if state.back() { None } else { Some(Outcome::Cancel) };
    }
    if state.step() == Step::Choose {
        for (key, kind) in [(Key::A, Kind::Agent), (Key::W, Kind::Workflow), (Key::S, Kind::Shell)]
        {
            if pressed(ctx, key) {
                return pick(state, kind, catalog);
            }
        }
        let back = pressed(ctx, Key::ArrowLeft) || pressed(ctx, Key::ArrowUp);
        let forward = pressed(ctx, Key::ArrowRight) || pressed(ctx, Key::ArrowDown);
        if back || forward {
            state.move_highlight(forward);
        }
        return pressed(ctx, Key::Enter).then(|| pick(state, state.highlight(), catalog))?;
    }
    if pressed(ctx, Key::ArrowUp) {
        state.step_selection(false, catalog);
    }
    if pressed(ctx, Key::ArrowDown) {
        state.step_selection(true, catalog);
    }
    if state.step() == Step::Agent {
        if pressed(ctx, Key::ArrowLeft) {
            state.step_level(false, catalog);
        }
        if pressed(ctx, Key::ArrowRight) {
            state.step_level(true, catalog);
        }
    }
    pressed(ctx, Key::Enter).then(|| start(state, catalog))?
}

fn pick(state: &mut NewTab, kind: Kind, catalog: &Catalog) -> Option<Outcome> {
    state.choose(kind, catalog).map(Outcome::Start)
}

pub(super) fn start(state: &mut NewTab, catalog: &Catalog) -> Option<Outcome> {
    match state.request(catalog) {
        Ok(request) => Some(Outcome::Start(request)),
        Err(reason) => {
            state.error = Some(reason);
            None
        }
    }
}

fn choose(ui: &mut Ui, state: &mut NewTab, catalog: &Catalog, outcome: &mut Option<Outcome>) {
    heading_block(ui, state);
    ui.add_space(22.0);
    let width = ui.available_width();
    let (row, _) = ui.allocate_exact_size(vec2(width, CARD_H), egui::Sense::hover());
    let card_w = (width - CARD_GAP) / 2.0;
    let left = Rect::from_min_size(row.min, vec2(card_w, CARD_H));
    let right = Rect::from_min_size(pos2(row.min.x + card_w + CARD_GAP, row.min.y), left.size());
    for (rect, kind) in [(left, Kind::Agent), (right, Kind::Workflow)] {
        let on = state.offers(kind);
        let response = clickable(ui, rect, kind_salt(kind));
        let lit = on && (response.hovered() || state.highlight() == kind);
        card(ui, rect, kind, state, lit);
        if on && response.clicked() {
            *outcome = pick(state, kind, catalog);
        }
    }
    ui.add_space(22.0);
    ui.separator();
    shell_row(ui, state, catalog, outcome);
}

fn kind_salt(kind: Kind) -> &'static str {
    match kind {
        Kind::Agent => "card-agent",
        Kind::Workflow => "card-workflow",
        Kind::Shell => "card-shell",
    }
}

fn heading_block(ui: &mut Ui, state: &NewTab) {
    ui.label(egui::RichText::new("New tab").font(heading(24.0)).color(colors::fg()));
    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let dim = colors::dim();
        ui.label(egui::RichText::new("in ").size(14.0).color(dim));
        ui.label(egui::RichText::new(state.label()).size(14.0).color(colors::accent()));
        match state.group() {
            Group::Repo => {
                ui.label(egui::RichText::new(", on a fresh worktree from ").size(14.0).color(dim));
                let base = state.base().unwrap_or("HEAD");
                ui.label(egui::RichText::new(base).font(mono(13.0)).color(colors::fg()));
            }
            Group::Other => {
                ui.label(
                    egui::RichText::new(", no worktree, runs in the folder itself")
                        .size(14.0)
                        .color(dim),
                );
            }
        }
    });
}

fn card(ui: &Ui, rect: Rect, kind: Kind, state: &NewTab, lit: bool) {
    let on = state.offers(kind);
    let painter = ui.painter();
    let fill = if lit { colors::selection() } else { colors::bg() };
    painter.rect(rect, 14.0, fill, card_stroke(lit), egui::StrokeKind::Inside);
    let fade = if on { 1.0 } else { 0.4 };
    let inner = rect.shrink(22.0);
    let icon = Rect::from_min_size(inner.min, Vec2::splat(ICON_BOX));
    let icon_fill =
        if lit { colors::accent().gamma_multiply(0.3) } else { colors::hover_elevated() };
    painter.rect_filled(icon, 12.0, icon_fill.gamma_multiply(fade));
    match kind {
        Kind::Agent => paint_agent_icon(ui, icon, fade),
        _ => paint_flow_icon(ui, icon, fade),
    }
    let letter = if kind == Kind::Agent { "A" } else { "W" };
    paint_key_cap(painter, pos2(inner.right(), inner.top() + ICON_BOX / 2.0), letter, false);
    let (title, text) = match (kind, on) {
        (Kind::Agent, _) => ("Agent", "One agent in its own worktree. You talk to it in the tab."),
        (_, true) => (
            "Workflow",
            "Agents in stages: plan, build, review, test, hand off. You set the task in the tab.",
        ),
        (_, false) => ("Workflow", "Needs a git repo to run its stages in a worktree."),
    };
    let title_pos = pos2(inner.left(), inner.top() + ICON_BOX + 14.0);
    let title_rect = painter.text(
        title_pos,
        Align2::LEFT_TOP,
        title,
        heading(20.0),
        colors::fg().gamma_multiply(fade),
    );
    let galley = painter.layout(
        text.to_owned(),
        body(14.0),
        colors::fg().gamma_multiply(0.62 * fade),
        inner.width(),
    );
    painter.galley(pos2(inner.left(), title_rect.bottom() + 10.0), galley, colors::dim());
}

fn paint_agent_icon(ui: &Ui, box_: Rect, fade: f32) {
    let c = box_.center();
    let stroke = Stroke::new(2.4, colors::fg().gamma_multiply(fade));
    let chevron = vec![c + vec2(-10.0, -6.0), c + vec2(-4.0, -1.5), c + vec2(-10.0, 3.0)];
    ui.painter().add(Shape::line(chevron, stroke));
    let wave: Vec<_> = (0..=12)
        .map(|i| {
            let t = i as f32 / 12.0;
            c + vec2(-2.0 + t * 14.0, 7.0 + (t * std::f32::consts::TAU).sin() * 2.2)
        })
        .collect();
    ui.painter().add(Shape::line(wave, Stroke::new(2.4, colors::accent().gamma_multiply(fade))));
}

fn paint_flow_icon(ui: &Ui, box_: Rect, fade: f32) {
    let c = box_.center();
    let ink = colors::fg().gamma_multiply(fade);
    let stroke = Stroke::new(1.8, ink);
    for (dx, color) in [(-9.0, ink), (0.0, ink), (9.0, ORANGE.gamma_multiply(fade))] {
        ui.painter().circle_stroke(c + vec2(dx, 0.0), 3.4, Stroke::new(1.8, color));
    }
    for dx in [-4.5, 4.5] {
        ui.painter().line_segment([c + vec2(dx - 1.0, 0.0), c + vec2(dx + 1.0, 0.0)], stroke);
    }
}

fn shell_row(ui: &mut Ui, state: &mut NewTab, catalog: &Catalog, outcome: &mut Option<Outcome>) {
    let width = ui.available_width();
    let (row, _) = ui.allocate_exact_size(vec2(width, 44.0), egui::Sense::hover());
    let button = Rect::from_min_size(pos2(row.left(), row.top() + 12.0), vec2(width * 0.6, 32.0));
    let response: Response = clickable(ui, button, "shell-row");
    let lit = response.hovered() || state.highlight() == Kind::Shell;
    let painter = ui.painter();
    if lit {
        painter.rect_filled(button, CornerRadius::same(8), colors::hover_elevated());
    }
    paint_key_cap(painter, pos2(button.left() + 4.0, button.center().y), "S", true);
    let text = match state.group() {
        Group::Repo => "Plain shell in this repo",
        Group::Other => "Plain shell in this folder",
    };
    painter.text(
        pos2(button.left() + 44.0, button.center().y),
        Align2::LEFT_CENTER,
        text,
        body(14.0),
        colors::fg().gamma_multiply(0.85),
    );
    painter.text(
        pos2(row.right(), row.top() + 28.0),
        Align2::RIGHT_CENTER,
        "A, W or S to pick  -  Esc closes",
        body(13.0),
        colors::dim(),
    );
    if response.clicked() {
        *outcome = pick(state, Kind::Shell, catalog);
    }
}

#[cfg(test)]
mod tests;
