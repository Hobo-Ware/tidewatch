//! The Links card of the Progress panel, and the Read first lines of a question: what the agent
//! shared with the user, newest first, each row a kind tag, a title that opens the link and a copy
//! button. The rows are pure (`link_model`); this only draws them.

use std::collections::HashSet;

use egui::{
    Align2, Color32, CornerRadius, FontId, OpenUrl, Pos2, Rect, Sense, Stroke, StrokeKind,
    TextEdit, Ui, UiBuilder, pos2, vec2,
};
use stdusk_egui::ui::icons;
use tidewatch_core::link_board::{Link, LinkBoard};

use crate::brand;
use crate::link_filter::{self, Bucket, Filter, Type};
use crate::link_model::{self, Tone, ink, listed};
use crate::reader_view::{ellipsized, paint_item};
use crate::screen_kit::{
    Size, TEXT_BODY, TEXT_SECONDARY, TEXT_TAG, TagFill, body, heading, icon_button, paint_tag,
    tag_width,
};
use crate::selectable;

const LINK_ICON: &str = "\u{E2E2}";
const HEAD_H: f32 = 36.0;
const ROW_H: f32 = 44.0;
const ROW_H_STACKED: f32 = 68.0;
const TAG_LINE_H: f32 = Size::Compact.height();
const MIN_STACK_TEXT_W: f32 = 90.0;
const FOLD_H: f32 = 36.0;
const EMPTY_H: f32 = 44.0;
const PAD_X: f32 = 12.0;
const GAP: f32 = 10.0;
const ICON_PX: f32 = 14.0;
const MIN_TEXT_W: f32 = 24.0;
const CHIP_H: f32 = Size::Compact.height();
const CHIP_GAP: f32 = 6.0;
const COUNT_GAP: f32 = 6.0;
const FILTER_PAD: f32 = 8.0;
const FILTER_FROM: usize = 4;
const SEARCH_HINT: &str = "Search links";
const SEARCH_TEXT_H: f32 = 18.0;
const MIN_TITLE_BESIDE_TAG: f32 = 80.0;
pub(crate) const READ_FIRST_H: f32 = 22.0;
pub(crate) const READ_FIRST_MAX: usize = 2;

/// Whether the rows put the tag and copy button on a line above the title: in a narrow panel, or
/// when the title would get under 90 px beside them.
fn stacks(painter: &egui::Painter, width: f32, stack: bool) -> bool {
    let beside =
        width - 2.0 * PAD_X - tag_width(painter, "Artifact") - Size::Compact.height() - 2.0 * GAP;
    stack || beside < MIN_STACK_TEXT_W
}

/// What the card keeps of one tab while the app runs: the chips and the search, and whether the
/// list is folded out.
#[derive(Clone, Debug, Default)]
pub(crate) struct LinkView {
    pub(crate) filter: Filter,
    pub(crate) all: bool,
}

impl LinkView {
    /// `TIDEWATCH_DEV_LINKS=<type>,<state>,<search>,all` for captures: the type chip (all, prs,
    /// issues, artifacts, docs), the state chip (open, needs, done, any), the search text and
    /// whether the list is folded out.
    pub(crate) fn from_dev(text: Option<&str>) -> Option<Self> {
        let mut parts = text?.split(',');
        let kind = match parts.next()? {
            "prs" => Type::Prs,
            "issues" => Type::Issues,
            "artifacts" => Type::Artifacts,
            "docs" => Type::Docs,
            _ => Type::All,
        };
        let bucket = match parts.next().unwrap_or_default() {
            "open" => Some(Bucket::Open),
            "needs" => Some(Bucket::NeedsAction),
            "done" => Some(Bucket::Done),
            _ => None,
        };
        let search = parts.next().unwrap_or_default().to_owned();
        let all = parts.next() == Some("all");
        Some(Self { filter: Filter { kind, bucket, search }, all })
    }
}

/// What the card draws.
pub(crate) struct Links<'a> {
    pub(crate) board: &'a LinkBoard,
    /// The repos (`owner/name` in lower case) a forwarder delivers for.
    pub(crate) live: &'a HashSet<String>,
    pub(crate) now_ms: i64,
    pub(crate) age: Option<&'a str>,
    pub(crate) stack: bool,
}

impl Links<'_> {
    fn is_live(&self, link: &Link) -> bool {
        link.repo().is_some_and(|repo| self.live.contains(&repo))
    }
}

/// Where each chip of widths `widths` goes in rows of at most `avail`, as offsets from the top
/// left, and the height the rows take.
pub(crate) fn flow(widths: &[f32], avail: f32) -> (Vec<Pos2>, f32) {
    let (mut x, mut y) = (0.0, 0.0);
    let mut placed = Vec::with_capacity(widths.len());
    for width in widths {
        if x > 0.0 && x + width > avail {
            x = 0.0;
            y += CHIP_H + CHIP_GAP;
        }
        placed.push(pos2(x, y));
        x += width + CHIP_GAP;
    }
    (placed, if widths.is_empty() { 0.0 } else { y + CHIP_H })
}

struct Chip {
    label: &'static str,
    count: usize,
    selected: bool,
}

fn chip_width(painter: &egui::Painter, chip: &Chip) -> f32 {
    let text = painter.layout_no_wrap(chip.label.to_owned(), body(TEXT_SECONDARY), brand::SOFT);
    let count = painter.layout_no_wrap(chip.count.to_string(), body(TEXT_SECONDARY), brand::FAINT);
    2.0 * Size::Compact.pad() + text.size().x + COUNT_GAP + count.size().x
}

fn chips_of(links: &Links<'_>, filter: &Filter) -> (Vec<Chip>, Vec<Chip>) {
    let counts = link_filter::counts(links.board, filter);
    let kinds = Type::ALL
        .map(|kind| Chip {
            label: kind.label(),
            count: counts.of_kind(kind),
            selected: filter.kind == kind,
        })
        .into();
    let buckets = Bucket::ALL
        .map(|bucket| Chip {
            label: bucket.label(),
            count: counts.of_bucket(bucket),
            selected: filter.bucket == Some(bucket),
        })
        .into();
    (kinds, buckets)
}

struct FilterLayout {
    kinds: (Vec<f32>, Vec<Pos2>, f32),
    buckets: (Vec<f32>, Vec<Pos2>, f32),
    height: f32,
}

fn filter_layout(
    painter: &egui::Painter,
    chips: &(Vec<Chip>, Vec<Chip>),
    avail: f32,
) -> FilterLayout {
    let group = |list: &[Chip]| {
        let widths: Vec<f32> = list.iter().map(|chip| chip_width(painter, chip)).collect();
        let (at, height) = flow(&widths, avail);
        (widths, at, height)
    };
    let (kinds, buckets) = (group(&chips.0), group(&chips.1));
    let height = FILTER_PAD + kinds.2 + CHIP_GAP + buckets.2 + CHIP_GAP + CHIP_H + FILTER_PAD;
    FilterLayout { kinds, buckets, height }
}

pub(crate) fn show(
    ui: &mut Ui,
    width: f32,
    links: &Links<'_>,
    view: &mut LinkView,
) -> Option<String> {
    let filtering = links.board.len() >= FILTER_FROM;
    if !filtering {
        view.filter = Filter::default();
    }
    let kept = link_filter::shown(links.board, &view.filter);
    let rows = listed(&kept, view.all);
    let fold = link_model::fold_label(kept.len(), view.all);
    let painter = ui.painter().clone();
    let chips = chips_of(links, &view.filter);
    let layout = filtering.then(|| filter_layout(&painter, &chips, width - 2.0 * PAD_X));
    let stacked = stacks(&painter, width, links.stack);
    let row_h = if stacked { ROW_H_STACKED } else { ROW_H };
    let (head_h, age_below) = head_height(&painter, width, links.board.len(), links.age);
    let height = head_h
        + layout.as_ref().map_or(0.0, |layout| layout.height)
        + row_h * rows.len() as f32
        + if kept.is_empty() { EMPTY_H } else { 0.0 }
        + if fold.is_some() { FOLD_H } else { 0.0 };
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    paint_item(&painter, rect, false);
    head(ui, rect, links.board.len(), links.age, age_below);
    let tag_col = tag_width(&painter, "Artifact");
    let mut top = rect.top() + head_h;
    if let Some(layout) = &layout {
        let area = Rect::from_min_size(pos2(rect.left(), top), vec2(width, layout.height));
        painter.hline(area.x_range(), area.top(), Stroke::new(1.0, brand::ITEM_LINE));
        filter_area(ui, area, (&chips, layout), view);
        top += layout.height;
    }
    let mut copied = None;
    for (at, link) in rows.iter().enumerate() {
        let row = Rect::from_min_size(pos2(rect.left(), top), vec2(width, row_h));
        painter.hline(row.x_range(), row.top(), Stroke::new(1.0, brand::ITEM_LINE));
        let live = links.is_live(link);
        if row_of(ui, row, (link, live), (at, tag_col), (links.now_ms, stacked)) {
            copied = Some(link.url.clone());
        }
        top += row_h;
    }
    if kept.is_empty() {
        let area = Rect::from_min_size(pos2(rect.left(), top), vec2(width, EMPTY_H));
        painter.hline(area.x_range(), area.top(), Stroke::new(1.0, brand::ITEM_LINE));
        empty_row(ui, area, view);
        top += EMPTY_H;
    }
    if let Some(label) = fold {
        let area = Rect::from_min_size(pos2(rect.left(), top), vec2(width, FOLD_H));
        painter.hline(area.x_range(), area.top(), Stroke::new(1.0, brand::ITEM_LINE));
        let galley = painter.layout_no_wrap(label, body(TEXT_SECONDARY), brand::TEAL_TEXT);
        let hit = Rect::from_min_size(
            pos2(area.left() + PAD_X, area.center().y - FOLD_H / 2.0),
            vec2(galley.size().x + 8.0, FOLD_H),
        );
        let response = ui.interact(hit, ui.id().with("links-fold"), Sense::click());
        let ink = if response.hovered() { brand::TEXT } else { brand::TEAL_TEXT };
        painter.galley(pos2(hit.left(), area.center().y - galley.size().y / 2.0), galley, ink);
        if response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            view.all = !view.all;
        }
    }
    copied
}

fn empty_row(ui: &mut Ui, area: Rect, view: &mut LinkView) {
    let painter = ui.painter();
    let line = link_filter::empty_line(&view.filter);
    let text =
        painter.layout_no_wrap(format!("{line} \u{b7} "), body(TEXT_SECONDARY), brand::MUTED);
    let action =
        painter.layout_no_wrap("Show all".to_owned(), body(TEXT_SECONDARY), brand::TEAL_TEXT);
    let left = area.left() + PAD_X;
    let mid = area.center().y;
    let hit = Rect::from_min_size(
        pos2(left + text.size().x, mid - EMPTY_H / 2.0),
        vec2(action.size().x + 8.0, EMPTY_H),
    );
    let response = ui.interact(hit, ui.id().with("links-show-all"), Sense::click());
    let ink = if response.hovered() { brand::TEXT } else { brand::TEAL_TEXT };
    let painter = ui.painter();
    painter.galley(pos2(left, mid - text.size().y / 2.0), text.clone(), brand::MUTED);
    painter.galley(pos2(hit.left(), mid - action.size().y / 2.0), action, ink);
    if response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
        view.filter = Filter::default();
        ui.ctx().request_repaint();
    }
}

fn filter_area(
    ui: &mut Ui,
    area: Rect,
    (chips, layout): (&(Vec<Chip>, Vec<Chip>), &FilterLayout),
    view: &mut LinkView,
) {
    let origin = pos2(area.left() + PAD_X, area.top() + FILTER_PAD);
    let mut clicked_kind = None;
    let mut clicked_bucket = None;
    for (at, chip) in chips.0.iter().enumerate() {
        let rect = chip_rect(origin, &layout.kinds, at);
        if paint_chip(ui, rect, chip, ("links-type", at)) {
            clicked_kind = Some(Type::ALL[at]);
        }
    }
    let buckets_origin = origin + vec2(0.0, layout.kinds.2 + CHIP_GAP);
    for (at, chip) in chips.1.iter().enumerate() {
        let rect = chip_rect(buckets_origin, &layout.buckets, at);
        if paint_chip(ui, rect, chip, ("links-bucket", at)) {
            clicked_bucket = Some(Bucket::ALL[at]);
        }
    }
    if let Some(kind) = clicked_kind {
        view.filter.kind = kind;
    }
    if let Some(bucket) = clicked_bucket {
        view.filter.bucket = if view.filter.bucket == Some(bucket) { None } else { Some(bucket) };
    }
    if clicked_kind.is_some() || clicked_bucket.is_some() {
        ui.ctx().request_repaint();
    }
    let search_top = buckets_origin.y + layout.buckets.2 + CHIP_GAP;
    let search =
        Rect::from_min_size(pos2(origin.x, search_top), vec2(area.width() - 2.0 * PAD_X, CHIP_H));
    search_field(ui, search, &mut view.filter.search);
}

fn chip_rect(origin: Pos2, group: &(Vec<f32>, Vec<Pos2>, f32), at: usize) -> Rect {
    Rect::from_min_size(origin + group.1[at].to_vec2(), vec2(group.0[at], CHIP_H))
}

fn paint_chip(ui: &mut Ui, rect: Rect, chip: &Chip, id: (&str, usize)) -> bool {
    let response = ui.interact(rect, ui.id().with(id), Sense::click());
    let (line, ink) = if chip.selected {
        (brand::TEAL, brand::TEAL_TEXT)
    } else if response.hovered() {
        (brand::FAINT, brand::TEXT)
    } else {
        (brand::ITEM_LINE, brand::SOFT)
    };
    let painter = ui.painter();
    painter.rect_stroke(
        rect,
        CornerRadius::same(Size::Compact.radius()),
        Stroke::new(1.0, line),
        StrokeKind::Inside,
    );
    let font = body(TEXT_SECONDARY);
    let text = painter.layout_no_wrap(chip.label.to_owned(), font.clone(), ink);
    let count = painter.layout_no_wrap(chip.count.to_string(), font, brand::FAINT);
    let left = rect.left() + Size::Compact.pad();
    let mid = rect.center().y;
    painter.galley(pos2(left, mid - text.size().y / 2.0), text.clone(), ink);
    painter.galley(
        pos2(left + text.size().x + COUNT_GAP, mid - count.size().y / 2.0),
        count,
        brand::FAINT,
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// The header's height, and whether the age goes on a line of its own because the label, count
/// and age do not fit side by side.
fn head_height(
    painter: &egui::Painter,
    width: f32,
    count: usize,
    age: Option<&str>,
) -> (f32, bool) {
    let Some(age) = age else { return (HEAD_H, false) };
    let label = painter.layout_no_wrap("LINKS".to_owned(), heading(TEXT_SECONDARY), brand::SOFT);
    let used =
        PAD_X + ICON_PX + 8.0 + label.size().x + 8.0 + tag_width(painter, &count.to_string());
    let age_w = crate::panel_blocks::age_width(painter, age);
    if used + 12.0 + age_w + PAD_X <= width { (HEAD_H, false) } else { (HEAD_H + 22.0, true) }
}

fn search_field(ui: &mut Ui, field: Rect, query: &mut String) {
    let id = ui.id().with("links-search");
    ui.painter().rect_filled(field, CornerRadius::same(Size::Compact.radius()), brand::SEGMENT);
    let inner = field.shrink2(vec2(Size::Compact.pad(), 0.0));
    let line = Rect::from_center_size(inner.center(), vec2(inner.width(), SEARCH_TEXT_H));
    let mut child = ui.new_child(UiBuilder::new().max_rect(line));
    child.add(
        TextEdit::singleline(query)
            .id(id)
            .frame(egui::Frame::NONE)
            .margin(egui::Margin::ZERO)
            .font(body(TEXT_SECONDARY))
            .text_color(brand::TEXT)
            .desired_width(inner.width()),
    );
    let focused = ui.memory(|memory| memory.has_focus(id));
    let line = if focused { brand::TEAL } else { brand::FIELD_LINE };
    let painter = ui.painter();
    painter.rect_stroke(
        field,
        CornerRadius::same(Size::Compact.radius()),
        Stroke::new(1.0, line),
        StrokeKind::Inside,
    );
    if query.is_empty() {
        painter.text(
            pos2(inner.left(), field.center().y),
            Align2::LEFT_CENTER,
            SEARCH_HINT,
            body(TEXT_SECONDARY),
            brand::FAINT,
        );
    }
}

fn tone_ink(tone: Tone) -> Color32 {
    match tone {
        Tone::Bad => brand::RED,
        Tone::Warn => brand::ORANGE,
        Tone::Good => brand::GREEN,
        Tone::Quiet => brand::MUTED,
        Tone::Live => brand::TEAL_TEXT,
    }
}

fn head(ui: &Ui, rect: Rect, count: usize, age: Option<&str>, age_below: bool) {
    let painter = ui.painter();
    let mid = rect.top() + HEAD_H / 2.0;
    painter.text(
        pos2(rect.left() + PAD_X, mid),
        Align2::LEFT_CENTER,
        LINK_ICON,
        FontId::proportional(ICON_PX),
        brand::MUTED,
    );
    let label = painter.layout_no_wrap("LINKS".to_owned(), heading(TEXT_SECONDARY), brand::SOFT);
    let left = rect.left() + PAD_X + ICON_PX + 8.0;
    painter.galley(pos2(left, mid - label.size().y / 2.0), label.clone(), brand::SOFT);
    paint_tag(
        painter,
        (left + label.size().x + 8.0, mid),
        &count.to_string(),
        brand::SOFT,
        TagFill::Tint(brand::ITEM_LINE),
    );
    if let Some(age) = age {
        if age_below {
            let galley = painter.layout_no_wrap(age.to_owned(), body(TEXT_TAG), brand::MUTED);
            painter.galley(
                pos2(rect.left() + PAD_X, mid + HEAD_H / 2.0 - 4.0),
                galley,
                brand::MUTED,
            );
        } else {
            crate::panel_blocks::paint_age(painter, rect.right() - PAD_X, mid, age);
        }
    }
}

fn row_of(
    ui: &mut Ui,
    row: Rect,
    (link, live): (&Link, bool),
    (index, tag_col): (usize, f32),
    (now_ms, stacked): (i64, bool),
) -> bool {
    let painter = ui.painter().clone();
    let kind = link.kind();
    let button = Size::Compact.height();
    let button_left = row.right() - PAD_X - button;
    let (mid, text_left, mut text_right, tag_mid) = if stacked {
        let line = row.top() + 6.0 + TAG_LINE_H / 2.0;
        (
            row.top() + 6.0 + TAG_LINE_H + 4.0 + (row.height() - TAG_LINE_H - 14.0) / 2.0,
            row.left() + PAD_X,
            row.right() - PAD_X,
            line,
        )
    } else {
        let mid = row.center().y;
        (mid, row.left() + PAD_X + tag_col + GAP, button_left - GAP, mid)
    };
    paint_tag(
        &painter,
        (row.left() + PAD_X, tag_mid),
        kind.word(),
        ink(kind),
        TagFill::Line(ink(kind)),
    );
    let pills = link_model::pills(link, live);
    let widths: Vec<f32> = pills.iter().map(|pill| tag_width(&painter, pill.text)).collect();
    let (pill_right, pill_floor, pill_mid) = if stacked {
        let tag_end = row.left() + PAD_X + tag_width(&painter, kind.word()) + GAP;
        (button_left - GAP, tag_end, tag_mid)
    } else {
        (text_right, text_left + MIN_TITLE_BESIDE_TAG, mid)
    };
    let mut fitting = 0;
    let mut taken = 0.0;
    for width in &widths {
        let next = taken + width + GAP;
        if pill_right - next < pill_floor {
            break;
        }
        taken = next;
        fitting += 1;
    }
    let mut left = pill_right - taken + GAP;
    for (pill, width) in pills.iter().zip(&widths).take(fitting) {
        let tint = tone_ink(pill.tone);
        paint_tag(&painter, (left, pill_mid), pill.text, tint, TagFill::Line(tint));
        left += width + GAP;
    }
    if fitting > 0 && !stacked {
        text_right -= taken;
    }
    let mut meta = link_model::meta(link, now_ms, live);
    if fitting < pills.len() {
        let hidden: Vec<&str> = pills[fitting..].iter().map(|pill| pill.text).collect();
        meta = format!("{} \u{b7} {meta}", hidden.join(", "));
    }
    let text_w = (text_right - text_left).max(MIN_TEXT_W);
    let meta = ellipsized(&painter, &meta, body(TEXT_TAG), brand::MUTED, text_w);
    let probe = ellipsized(&painter, &link.title, body(TEXT_BODY), brand::TEXT, text_w);
    let block = probe.size().y + meta.size().y + 1.0;
    let top = mid - block / 2.0;
    let hit = Rect::from_min_size(pos2(text_left, top), vec2(probe.size().x, probe.size().y));
    let response = ui.interact(hit, ui.id().with(("link-open", index, &link.url)), Sense::click());
    let ink_of_title = if response.hovered() { brand::TEAL_TEXT } else { brand::TEXT };
    let title = if response.hovered() {
        ellipsized(&painter, &link.title, body(TEXT_BODY), ink_of_title, text_w)
    } else {
        probe
    };
    painter.galley(pos2(text_left, top), title, ink_of_title);
    if response.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(&link.url).clicked() {
        ui.ctx().open_url(OpenUrl::new_tab(&link.url));
    }
    selectable::galley(ui, pos2(text_left, top + hit.height() + 1.0), meta, brand::MUTED);
    let button_mid = if stacked { tag_mid } else { mid };
    let at =
        Rect::from_min_size(pos2(button_left, button_mid - button / 2.0), vec2(button, button));
    let mut child = ui.new_child(UiBuilder::new().id_salt(("link-copy", index)).max_rect(at));
    icon_button(&mut child, icons::COPY, "Copy link", Size::Compact).clicked()
}

pub(crate) fn read_first_height(count: usize) -> f32 {
    READ_FIRST_H * count.min(READ_FIRST_MAX) as f32
}

/// The `Read first:` lines of a question, from `top_left` across `width`: the links sent for it,
/// at most two, each title opening its link.
pub(crate) fn paint_read_first(ui: &mut Ui, top_left: Pos2, width: f32, links: &[&Link]) {
    let painter = ui.painter().clone();
    for (index, link) in links.iter().take(READ_FIRST_MAX).enumerate() {
        let mid = top_left.y + READ_FIRST_H * index as f32 + READ_FIRST_H / 2.0;
        painter.text(
            pos2(top_left.x, mid),
            Align2::LEFT_CENTER,
            LINK_ICON,
            FontId::proportional(TEXT_SECONDARY),
            brand::MUTED,
        );
        let label =
            painter.layout_no_wrap("Read first:".to_owned(), body(TEXT_SECONDARY), brand::MUTED);
        let label_left = top_left.x + TEXT_SECONDARY + 6.0;
        painter.galley(pos2(label_left, mid - label.size().y / 2.0), label.clone(), brand::MUTED);
        let title_left = label_left + label.size().x + 6.0;
        let room = (top_left.x + width - title_left).max(MIN_TEXT_W);
        let probe = ellipsized(&painter, &link.title, body(TEXT_SECONDARY), brand::TEAL_TEXT, room);
        let hit = Rect::from_min_size(pos2(title_left, mid - probe.size().y / 2.0), probe.size());
        let response =
            ui.interact(hit, ui.id().with(("read-first", index, &link.url)), Sense::click());
        let underline =
            if response.hovered() { Stroke::new(1.0, brand::TEAL_TEXT) } else { Stroke::NONE };
        painter.galley_with_override_text_color(hit.min, probe, brand::TEAL_TEXT);
        painter.hline(hit.x_range(), hit.bottom(), underline);
        if response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(&link.url)
            .clicked()
        {
            ui.ctx().open_url(OpenUrl::new_tab(&link.url));
        }
    }
}

#[cfg(test)]
mod tests;
