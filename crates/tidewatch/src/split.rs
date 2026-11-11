//! The main tab's split terminal, as stdusk does it: stdusk's `pane::Pane` tree holds the shape, each
//! pane has its own shell and one pane has the focus. Generic over the shell, so it is tested
//! without a pty. The tab's own shell stays in its body as pane [`MAIN`]; panes are named by id.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use egui::Rect;
use stdusk_core::pane::{self, Dir, Side, SplitDir};
use tidewatch_core::session::{SavedAxis, SavedSplit};

pub(crate) const MAIN: u32 = 0;
const MAX_PANES: usize = 16;
const MIN_PERMILLE: u16 = 50;
const MAX_PERMILLE: u16 = 950;

/// One pane as laid out in an area.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Place {
    pub(crate) id: u32,
    pub(crate) path: Vec<Side>,
    pub(crate) rect: Rect,
    pub(crate) focused: bool,
}

/// A divider between two panes: where to draw and grab it, and what it resizes.
pub(crate) struct Divider {
    pub(crate) path: Vec<Side>,
    pub(crate) dir: SplitDir,
    pub(crate) handle: Rect,
    pub(crate) parent: Rect,
}

/// What closing a pane did: the pane that went, and when it was [`MAIN`], the pane that took its
/// place (named by the id it had until now).
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Closed {
    pub(crate) removed: u32,
    pub(crate) promoted: Option<u32>,
}

pub(crate) struct Layout {
    root: pane::Pane<u32>,
    focused: u32,
    next: u32,
}

impl Layout {
    pub(crate) fn single() -> Self {
        Self { root: pane::Pane::leaf(MAIN), focused: MAIN, next: MAIN + 1 }
    }

    /// The layout a saved shape describes, focused on the main pane. `None` for a shape with more
    /// panes than a window will hold.
    pub(crate) fn from_saved(shape: &SavedSplit) -> Option<Self> {
        let mut next = MAIN;
        let root = build(shape, &mut next)?;
        Some(Self { root, focused: MAIN, next })
    }

    pub(crate) fn to_saved(&self) -> SavedSplit {
        saved(&self.root)
    }

    pub(crate) fn leaf_count(&self) -> usize {
        self.root.leaf_count()
    }

    pub(crate) fn is_single(&self) -> bool {
        self.leaf_count() < 2
    }

    pub(crate) fn can_split(&self) -> bool {
        self.leaf_count() < MAX_PANES
    }

    pub(crate) fn focused(&self) -> u32 {
        self.focused
    }

    pub(crate) fn ids(&self) -> Vec<u32> {
        self.root.leaves().into_iter().copied().collect()
    }

    fn path_of(&self, id: u32) -> Option<Vec<Side>> {
        let paths = self.root.leaf_paths();
        let at = self.root.leaves().iter().position(|leaf| **leaf == id)?;
        paths.into_iter().nth(at)
    }

    pub(crate) fn places(&self, area: Rect) -> Vec<Place> {
        self.root
            .layout(area)
            .into_iter()
            .filter_map(|(path, rect)| {
                let id = *self.root.leaf_at(&path)?;
                Some(Place { id, path, rect, focused: id == self.focused })
            })
            .collect()
    }

    pub(crate) fn dividers(&self, area: Rect) -> Vec<Divider> {
        self.root
            .splitters(area)
            .into_iter()
            .map(|(path, dir, handle, parent)| Divider { path, dir, handle, parent })
            .collect()
    }

    pub(crate) fn drag(&mut self, divider: &[Side], ratio: f32) {
        self.root.set_ratio(divider, ratio);
    }

    pub(crate) fn focus(&mut self, id: u32) -> bool {
        let known = self.root.leaves().iter().any(|leaf| **leaf == id);
        let changed = known && self.focused != id;
        if known {
            self.focused = id;
        }
        changed
    }

    /// Moves the focus to the pane nearest in `dir` in `area`, as stdusk's Cmd+Alt+arrows do.
    pub(crate) fn focus_toward(&mut self, area: Rect, dir: Dir) -> bool {
        let Some(from) = self.path_of(self.focused) else { return false };
        let layout = self.root.layout(area);
        let Some(target) = pane::neighbor(&layout, &from, dir) else { return false };
        let Some(id) = self.root.leaf_at(&target).copied() else { return false };
        self.focus(id)
    }

    /// Splits the focused pane in `dir`, the new pane after it, and focuses the new pane.
    pub(crate) fn split_focused(&mut self, dir: SplitDir) -> Option<u32> {
        if !self.can_split() {
            return None;
        }
        let path = self.path_of(self.focused)?;
        let id = self.next;
        let root = std::mem::replace(&mut self.root, pane::Pane::leaf(MAIN));
        let (root, focus) = root.split(&path, dir, id, false);
        self.root = root;
        focus?;
        self.next += 1;
        self.focused = id;
        Some(id)
    }

    /// Closes pane `id`, its sibling taking the space. The focus moves to the sibling when the
    /// closed pane had it. `None` for the last pane or an unknown id.
    pub(crate) fn close(&mut self, id: u32) -> Option<Closed> {
        if self.is_single() {
            return None;
        }
        let path = self.path_of(id)?;
        let root = std::mem::replace(&mut self.root, pane::Pane::leaf(MAIN));
        let (root, focus) = root.close(&path);
        self.root = root?;
        let sibling = self.root.leaf_at(&focus?).copied()?;
        let promoted = (id == MAIN).then_some(sibling);
        if let Some(old) = promoted {
            self.relabel(old, MAIN);
        }
        let landed = if promoted.is_some() { MAIN } else { sibling };
        if self.focused == id {
            self.focused = landed;
        } else if Some(self.focused) == promoted {
            self.focused = MAIN;
        }
        Some(Closed { removed: id, promoted })
    }

    fn relabel(&mut self, from: u32, to: u32) {
        let Some(path) = self.path_of(from) else { return };
        if let Some(leaf) = self.root.leaf_at_mut(&path) {
            *leaf = to;
        }
    }
}

fn build(shape: &SavedSplit, next: &mut u32) -> Option<pane::Pane<u32>> {
    match shape {
        SavedSplit::Pane => {
            let id = *next;
            *next += 1;
            (id < u32::try_from(MAX_PANES).ok()?).then(|| pane::Pane::leaf(id))
        }
        SavedSplit::Split { axis, permille, a, b } => {
            let a = build(a, next)?;
            let b = build(b, next)?;
            let ratio = f32::from((*permille).clamp(MIN_PERMILLE, MAX_PERMILLE)) / 1000.0;
            let dir = match axis {
                SavedAxis::Row => SplitDir::Row,
                SavedAxis::Column => SplitDir::Column,
            };
            Some(pane::Pane::Split { dir, ratio, a: Box::new(a), b: Box::new(b) })
        }
    }
}

fn permille_of(ratio: f32) -> u16 {
    (ratio * 1000.0).round().clamp(f32::from(MIN_PERMILLE), f32::from(MAX_PERMILLE)) as u16
}

fn saved(node: &pane::Pane<u32>) -> SavedSplit {
    match node {
        pane::Pane::Leaf(_) => SavedSplit::Pane,
        pane::Pane::Split { dir, ratio, a, b } => SavedSplit::Split {
            axis: match dir {
                SplitDir::Row => SavedAxis::Row,
                SplitDir::Column => SavedAxis::Column,
            },
            permille: permille_of(*ratio),
            a: Box::new(saved(a)),
            b: Box::new(saved(b)),
        },
    }
}

impl std::fmt::Debug for Layout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Layout")
            .field("panes", &self.leaf_count())
            .field("focused", &self.focused)
            .finish_non_exhaustive()
    }
}

impl Hash for Layout {
    fn hash<H: Hasher>(&self, state: &mut H) {
        fn walk<H: Hasher>(node: &pane::Pane<u32>, state: &mut H) {
            match node {
                pane::Pane::Leaf(_) => 0u8.hash(state),
                pane::Pane::Split { dir, ratio, a, b } => {
                    1u8.hash(state);
                    matches!(dir, SplitDir::Row).hash(state);
                    permille_of(*ratio).hash(state);
                    walk(a, state);
                    walk(b, state);
                }
            }
        }
        walk(&self.root, state);
    }
}

/// The shells of a split main tab. The tab's own shell is passed in wherever a pane is looked up,
/// because it stays in the tab's body.
pub(crate) struct Splits<T> {
    layout: Layout,
    terms: HashMap<u32, T>,
    area: Rect,
    refocus: bool,
}

impl<T> Splits<T> {
    pub(crate) fn single() -> Self {
        Self {
            layout: Layout::single(),
            terms: HashMap::new(),
            area: Rect::NOTHING,
            refocus: false,
        }
    }

    /// The splits a saved shape describes, a fresh shell from `spawn` in every pane but the main
    /// one. `None` for a shape this window will not hold.
    pub(crate) fn from_saved(shape: &SavedSplit, mut spawn: impl FnMut() -> T) -> Option<Self> {
        let layout = Layout::from_saved(shape)?;
        let terms =
            layout.ids().into_iter().filter(|id| *id != MAIN).map(|id| (id, spawn())).collect();
        Some(Self { layout, terms, area: Rect::NOTHING, refocus: true })
    }

    pub(crate) fn layout(&self) -> &Layout {
        &self.layout
    }

    pub(crate) fn layout_mut(&mut self) -> &mut Layout {
        &mut self.layout
    }

    pub(crate) fn is_single(&self) -> bool {
        self.layout.is_single()
    }

    pub(crate) fn can_split(&self) -> bool {
        self.layout.can_split()
    }

    pub(crate) fn set_area(&mut self, area: Rect) {
        self.area = area;
    }

    pub(crate) fn split(&mut self, dir: SplitDir, term: T) -> bool {
        let Some(id) = self.layout.split_focused(dir) else { return false };
        self.terms.insert(id, term);
        self.refocus = true;
        true
    }

    pub(crate) fn focus_toward(&mut self, dir: Dir) -> bool {
        let moved = self.layout.focus_toward(self.area, dir);
        self.refocus |= moved;
        moved
    }

    pub(crate) fn focus(&mut self, id: u32) {
        self.refocus |= self.layout.focus(id);
    }

    /// Whether the focus moved since the last call: the view then hands egui's keyboard focus to
    /// the pane that has it now.
    pub(crate) fn take_refocus(&mut self) -> bool {
        std::mem::take(&mut self.refocus)
    }

    pub(crate) fn close_focused(&mut self, main: &mut T) -> bool {
        self.close(self.layout.focused(), main)
    }

    /// Closes pane `id` and stops its shell. When that is the main pane, another pane's shell takes
    /// its place in `main`.
    pub(crate) fn close(&mut self, id: u32, main: &mut T) -> bool {
        let before = self.layout.focused();
        let Some(closed) = self.layout.close(id) else { return false };
        if closed.removed != MAIN {
            self.terms.remove(&closed.removed);
        } else if let Some(mut promoted) = closed.promoted.and_then(|old| self.terms.remove(&old)) {
            std::mem::swap(main, &mut promoted);
        }
        self.refocus |= self.layout.focused() != before;
        true
    }

    pub(crate) fn term_mut<'a>(&'a mut self, main: &'a mut T, id: u32) -> Option<&'a mut T> {
        if id == MAIN { Some(main) } else { self.terms.get_mut(&id) }
    }

    pub(crate) fn focused_term_mut<'a>(&'a mut self, main: &'a mut T) -> &'a mut T {
        let id = self.layout.focused();
        self.term_mut(main, id).unwrap_or_else(|| unreachable!("the focused pane has a shell"))
    }

    /// The first pane whose shell `is` true for, the main pane included.
    pub(crate) fn find(&self, main: &T, is: impl Fn(&T) -> bool) -> Option<u32> {
        self.layout
            .ids()
            .into_iter()
            .find(|id| if *id == MAIN { is(main) } else { self.terms.get(id).is_some_and(&is) })
    }

    pub(crate) fn places(&self) -> Vec<Place> {
        self.layout.places(self.area)
    }
}

#[cfg(test)]
mod tests {
    use egui::{pos2, vec2};
    use std::collections::hash_map::DefaultHasher;

    use super::*;

    fn area() -> Rect {
        Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 600.0))
    }

    fn splits() -> Splits<u32> {
        let mut splits = Splits::single();
        splits.set_area(area());
        splits
    }

    /// Main on the left; on the right, pane 1 above pane 2. Pane 2 has the focus.
    fn three() -> Splits<u32> {
        let mut splits = splits();
        assert!(splits.split(SplitDir::Row, 101));
        assert!(splits.split(SplitDir::Column, 102));
        splits
    }

    fn ids(splits: &Splits<u32>) -> Vec<u32> {
        splits.layout().ids()
    }

    fn hash_of(layout: &Layout) -> u64 {
        let mut hasher = DefaultHasher::new();
        layout.hash(&mut hasher);
        hasher.finish()
    }

    #[test]
    fn splitting_puts_the_new_pane_after_the_focused_one_and_focuses_it() {
        let cases = [
            (SplitDir::Row, vec2(497.0, 600.0), pos2(503.0, 0.0)),
            (SplitDir::Column, vec2(1000.0, 297.0), pos2(0.0, 303.0)),
        ];
        for (dir, size, second) in cases {
            let mut splits = splits();
            assert!(splits.split(dir, 7));
            let places = splits.places();
            assert_eq!(ids(&splits), [MAIN, 1], "{dir:?}");
            assert_eq!(places[0].rect, Rect::from_min_size(pos2(0.0, 0.0), size), "{dir:?}");
            assert_eq!(places[1].rect, Rect::from_min_size(second, size), "{dir:?}");
            assert_eq!(splits.layout().focused(), 1, "{dir:?}");
        }
    }

    #[test]
    fn closing_a_pane_gives_its_space_to_its_sibling_and_stops_its_shell() {
        let mut main = 100;
        let mut splits = three();
        assert!(splits.close(2, &mut main));
        assert_eq!(ids(&splits), [MAIN, 1]);
        assert_eq!(splits.layout().focused(), 1);
        assert_eq!(
            splits.places()[1].rect,
            Rect::from_min_size(pos2(503.0, 0.0), vec2(497.0, 600.0))
        );
        assert_eq!(splits.find(&main, |term| *term == 102), None);
        assert_eq!(main, 100);
    }

    #[test]
    fn closing_the_main_pane_hands_its_place_to_another_shell() {
        let mut main = 100;
        let mut splits = three();
        splits.focus(MAIN);
        assert!(splits.close_focused(&mut main));
        assert_eq!(main, 101);
        assert_eq!(ids(&splits), [MAIN, 2]);
        assert_eq!(splits.layout().focused(), MAIN);
        assert_eq!(splits.find(&main, |term| *term == 101), Some(MAIN));
        assert_eq!(splits.find(&main, |term| *term == 100), None);
    }

    #[test]
    fn the_last_pane_cannot_be_closed_and_a_closed_focus_follows_the_sibling() {
        let mut main = 100;
        let mut splits = splits();
        assert!(!splits.close_focused(&mut main));
        assert!(splits.split(SplitDir::Row, 101));
        assert!(splits.close_focused(&mut main));
        assert_eq!((ids(&splits), splits.layout().focused(), main), (vec![MAIN], MAIN, 100));
        assert!(splits.is_single());
    }

    #[test]
    fn closing_another_pane_leaves_the_focus_where_it_was() {
        let mut main = 100;
        let mut splits = three();
        splits.focus(MAIN);
        assert!(splits.close(1, &mut main));
        assert_eq!(splits.layout().focused(), MAIN);
        assert_eq!(ids(&splits), [MAIN, 2]);
    }

    #[test]
    fn focus_moves_to_the_nearest_pane_in_the_direction_of_the_arrow() {
        let cases = [
            (2, Dir::Left, Some(MAIN)),
            (2, Dir::Up, Some(1)),
            (2, Dir::Down, None),
            (2, Dir::Right, None),
            (1, Dir::Down, Some(2)),
            (MAIN, Dir::Right, Some(1)),
            (MAIN, Dir::Left, None),
        ];
        for (from, dir, to) in cases {
            let mut splits = three();
            splits.focus(from);
            assert_eq!(splits.focus_toward(dir), to.is_some(), "{from} {dir:?}");
            assert_eq!(splits.layout().focused(), to.unwrap_or(from), "{from} {dir:?}");
        }
    }

    #[test]
    fn a_focus_move_asks_the_view_to_hand_over_the_keyboard_once() {
        let mut splits = three();
        assert!(splits.take_refocus());
        assert!(!splits.take_refocus());
        splits.focus_toward(Dir::Left);
        assert!(splits.take_refocus());
        splits.focus_toward(Dir::Left);
        assert!(!splits.take_refocus());
    }

    #[test]
    fn a_window_holds_sixteen_panes() {
        let mut splits = splits();
        let mut made = 1;
        while splits.split(SplitDir::Row, made) {
            made += 1;
        }
        assert_eq!(splits.layout().leaf_count(), MAX_PANES);
        assert!(!splits.can_split());
    }

    #[test]
    fn the_saved_shape_round_trips_with_its_ratios() {
        let mut splits = three();
        let root = splits.layout().places(area());
        assert_eq!(root.len(), 3);
        let divider = splits.layout().dividers(area()).remove(0);
        splits.layout_mut().drag(&divider.path, 0.3);
        let shape = splits.layout().to_saved();
        let back = Layout::from_saved(&shape).unwrap();
        assert_eq!(back.to_saved(), shape);
        assert_eq!(hash_of(&back), hash_of(splits.layout()));
        assert_eq!(back.ids(), [MAIN, 1, 2]);
        assert_eq!(back.focused(), MAIN);
        assert_eq!(back.places(area())[0].rect.width(), 298.2);
    }

    #[test]
    fn the_signature_changes_with_the_shape_and_a_drag_but_not_with_the_focus() {
        let mut splits = three();
        let before = hash_of(splits.layout());
        splits.focus(MAIN);
        assert_eq!(hash_of(splits.layout()), before);
        let divider = splits.layout().dividers(area()).remove(0);
        splits.layout_mut().drag(&divider.path, 0.7);
        assert_ne!(hash_of(splits.layout()), before);
        assert_ne!(hash_of(&Layout::single()), before);
    }

    #[test]
    fn a_saved_shape_comes_back_with_a_fresh_shell_in_every_pane_but_the_main_one() {
        let shape = three().layout().to_saved();
        let mut made = 0;
        let back = Splits::from_saved(&shape, || {
            made += 1;
            made
        })
        .unwrap();
        assert_eq!(ids(&back), [MAIN, 1, 2]);
        assert_eq!(made, 2);
    }

    #[test]
    fn a_saved_shape_with_too_many_panes_is_not_taken() {
        let mut shape = SavedSplit::Pane;
        for _ in 0..MAX_PANES {
            shape = SavedSplit::Split {
                axis: SavedAxis::Row,
                permille: 500,
                a: Box::new(shape),
                b: Box::new(SavedSplit::Pane),
            };
        }
        assert!(Layout::from_saved(&shape).is_none());
    }

    #[test]
    fn a_saved_ratio_outside_the_dragging_range_is_pulled_in() {
        let shape = SavedSplit::Split {
            axis: SavedAxis::Row,
            permille: 0,
            a: Box::new(SavedSplit::Pane),
            b: Box::new(SavedSplit::Pane),
        };
        let SavedSplit::Split { permille, .. } = Layout::from_saved(&shape).unwrap().to_saved()
        else {
            unreachable!("two panes make a split")
        };
        assert_eq!(permille, MIN_PERMILLE);
    }
}
