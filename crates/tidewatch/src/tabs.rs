//! Tab list and its reducer. Generic over the pane payload so the transitions are testable
//! without a pty. The UI collects `Action`s during the frame and applies them afterwards.
//!
//! Every tab belongs to a group, one per repo (or folder) the window holds, each with its own
//! pinned main tab. One list keeps tab ids unique across the window; the visible group is the one
//! the active tab is in, so selecting a tab of another group switches to it.

mod groups;

use crate::park::{Park, Why};

pub(crate) const MAIN_TITLE: &str = "main";

pub(crate) struct Tab<T> {
    pub(crate) id: u64,
    pub(crate) group: usize,
    pub(crate) title: String,
    pub(crate) pinned: bool,
    pub(crate) pane: T,
    pub(crate) park: Option<Park>,
    pub(crate) keep: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Step {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    New,
    CloseActive,
    Close(usize),
    Select(usize),
    /// The n-th tab (from zero) of the visible group.
    SelectNth(usize),
    NextGroup,
    PrevGroup,
    ShowGroup(usize),
    /// Swaps a tab with its neighbour in the same group, never past the pinned main tab.
    Move(usize, Step),
    MoveActive(Step),
    Park(usize),
    ParkActive,
    ToggleKeep(usize),
    /// Opens the Change this tab popover for a tab.
    Change(usize),
}

pub(crate) struct Tabs<T> {
    items: Vec<Tab<T>>,
    active: usize,
    next_id: u64,
    opened: usize,
    /// Per group, the id of the tab selected last in it: what shows when the group comes back.
    recent: Vec<u64>,
}

impl<T> Tabs<T> {
    pub(crate) fn new(main_pane: T) -> Self {
        Self {
            items: vec![Tab {
                id: 0,
                group: 0,
                title: MAIN_TITLE.into(),
                pinned: true,
                pane: main_pane,
                park: None,
                keep: false,
            }],
            active: 0,
            next_id: 1,
            opened: 0,
            recent: vec![0],
        }
    }

    pub(crate) fn items(&self) -> &[Tab<T>] {
        &self.items
    }

    pub(crate) fn items_mut(&mut self) -> &mut [Tab<T>] {
        &mut self.items
    }

    pub(crate) fn active(&self) -> usize {
        self.active
    }

    pub(crate) fn active_pane_mut(&mut self) -> &mut T {
        &mut self.items[self.active].pane
    }

    /// The group on show.
    pub(crate) fn active_group(&self) -> usize {
        self.items[self.active].group
    }

    /// A new group with its own pinned main tab. The visible group stays as it is.
    pub(crate) fn add_group(&mut self, main_pane: T) -> usize {
        let group = self.recent.len();
        let id = self.take_id();
        self.items.push(Tab {
            id,
            group,
            title: MAIN_TITLE.into(),
            pinned: true,
            pane: main_pane,
            park: None,
            keep: false,
        });
        self.recent.push(id);
        group
    }

    /// The tabs of one group with their positions in the whole list, in order.
    pub(crate) fn in_group(&self, group: usize) -> impl Iterator<Item = (usize, &Tab<T>)> {
        self.items.iter().enumerate().filter(move |(_, tab)| tab.group == group)
    }

    pub(crate) fn shown_in(&self, group: usize) -> impl Iterator<Item = (usize, &Tab<T>)> {
        self.in_group(group).filter(|(_, tab)| tab.park.is_none())
    }

    pub(crate) fn parked_in(&self, group: usize) -> impl Iterator<Item = (usize, &Tab<T>)> {
        self.in_group(group).filter(|(_, tab)| tab.park.is_some())
    }

    pub(crate) fn group_of(&self, id: u64) -> Option<usize> {
        self.items.iter().find(|tab| tab.id == id).map(|tab| tab.group)
    }

    pub(crate) fn main_id(&self, group: usize) -> Option<u64> {
        self.in_group(group).find(|(_, tab)| tab.pinned).map(|(_, tab)| tab.id)
    }

    /// The position of the tab a group has selected: the active one for the visible group.
    pub(crate) fn selected_in(&self, group: usize) -> usize {
        if group == self.active_group() {
            return self.active;
        }
        self.recent
            .get(group)
            .and_then(|id| self.index_of(*id))
            .filter(|i| self.items[*i].park.is_none())
            .or_else(|| self.shown_in(group).next().map(|(i, _)| i))
            .unwrap_or(self.active)
    }

    fn take_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Appends a tab to the visible group, selects it and returns its id.
    pub(crate) fn push(&mut self, title: String, pane: T) -> u64 {
        let id = self.push_in(self.active_group(), title, pane);
        self.select_index(self.items.len() - 1);
        id
    }

    /// Appends a tab to a group without selecting it, and returns its id.
    pub(crate) fn push_in(&mut self, group: usize, title: String, pane: T) -> u64 {
        let id = self.take_id();
        self.items.push(Tab { id, group, title, pinned: false, pane, park: None, keep: false });
        id
    }

    /// Selects a tab. In a group that is not on show it only becomes what that group shows once it
    /// is, so a restored tab never takes the window from the one in front.
    pub(crate) fn select_id(&mut self, id: u64) {
        let Some(index) = self.index_of(id) else { return };
        if self.items[index].group == self.active_group() {
            self.select_index(index);
        } else {
            self.items[index].park = None;
            self.recent[self.items[index].group] = id;
        }
    }

    pub(crate) fn index_of(&self, id: u64) -> Option<usize> {
        self.items.iter().position(|t| t.id == id)
    }

    pub(crate) fn get_mut(&mut self, id: u64) -> Option<&mut Tab<T>> {
        self.items.iter_mut().find(|t| t.id == id)
    }

    /// Moves a tab into another group; the pinned main tab never moves. A group left showing a tab
    /// that moved falls back to its main tab.
    pub(crate) fn move_to_group(&mut self, id: u64, group: usize) {
        let Some(index) = self.index_of(id) else { return };
        let old = self.items[index].group;
        if old == group || group >= self.recent.len() || self.items[index].pinned {
            return;
        }
        self.items[index].group = group;
        if self.recent[old] == id {
            self.recent[old] = self.main_id(old).unwrap_or_default();
        }
    }

    pub(crate) fn remove_id(&mut self, id: u64) {
        if let Some(i) = self.index_of(id) {
            self.close(i);
        }
    }

    pub(crate) fn apply(&mut self, action: Action, spawn: impl FnOnce() -> T) {
        match action {
            Action::New => {
                self.opened += 1;
                let title = format!("shell {}", self.opened);
                self.push(title, spawn());
            }
            Action::CloseActive => self.close(self.active),
            Action::Close(i) => self.close(i),
            Action::Select(i) => {
                if i < self.items.len() {
                    self.select_index(i);
                }
            }
            Action::SelectNth(n) => {
                let found = self.shown_in(self.active_group()).nth(n).map(|(i, _)| i);
                if let Some(i) = found {
                    self.select_index(i);
                }
            }
            Action::NextGroup => self.show_group(self.active_group() + 1),
            Action::PrevGroup => self.show_group(self.active_group() + self.recent.len() - 1),
            Action::ShowGroup(group) => {
                if group < self.recent.len() {
                    self.show_group(group);
                }
            }
            Action::Move(i, step) => self.move_tab(i, step),
            Action::MoveActive(step) => self.move_tab(self.active, step),
            Action::Park(i) => {
                self.park_index(i, Why::Manual, 0);
            }
            Action::ParkActive => {
                self.park_index(self.active, Why::Manual, 0);
            }
            Action::Change(_) => {}
            Action::ToggleKeep(i) => {
                if let Some(tab) = self.items.get_mut(i).filter(|tab| !tab.pinned) {
                    tab.keep = !tab.keep;
                    tab.park = tab.park.filter(|_| !tab.keep);
                }
            }
        }
    }

    pub(crate) fn park_index(&mut self, index: usize, why: Why, since_ms: u64) -> bool {
        let Some(tab) = self.items.get(index) else { return false };
        if tab.pinned || tab.park.is_some() {
            return false;
        }
        if index == self.active {
            let group = tab.group;
            let shown = |i: &usize| *i != index && self.items[*i].park.is_none();
            let after =
                (index + 1..self.items.len()).filter(|i| self.items[*i].group == group).find(shown);
            let before = (0..index).rev().filter(|i| self.items[*i].group == group).find(shown);
            let Some(next) = after.or(before) else { return false };
            self.select_index(next);
        }
        self.items[index].park = Some(Park { why, since_ms });
        true
    }

    pub(crate) fn set_park(&mut self, id: u64, park: Park) {
        let Some(index) = self.index_of(id) else { return };
        let tab = &mut self.items[index];
        if index != self.active && !tab.pinned && !tab.keep {
            tab.park = Some(park);
        }
    }

    pub(crate) fn unpark(&mut self, id: u64) {
        if let Some(tab) = self.get_mut(id) {
            tab.park = None;
        }
    }

    fn move_tab(&mut self, i: usize, step: Step) {
        let Some(tab) = self.items.get(i) else { return };
        let group = tab.group;
        let on_show = |t: &Tab<T>| t.group == group && t.park.is_none();
        let neighbor = match step {
            Step::Left => self.items[..i].iter().rposition(on_show),
            Step::Right => self.items[i + 1..].iter().position(on_show).map(|n| n + i + 1),
        };
        let Some(j) = neighbor else { return };
        if self.items[i].pinned || self.items[j].pinned {
            return;
        }
        self.items.swap(i, j);
        if self.active == i {
            self.active = j;
        } else if self.active == j {
            self.active = i;
        }
    }

    fn select_index(&mut self, index: usize) {
        self.active = index;
        self.items[index].park = None;
        self.recent[self.items[index].group] = self.items[index].id;
    }

    fn show_group(&mut self, group: usize) {
        let group = group % self.recent.len();
        self.select_index(self.selected_in(group));
    }

    /// The tab of a group at `from`, else the one before it. A group always has its main tab.
    fn neighbor(&self, group: usize, from: usize) -> usize {
        let shown = |tab: &Tab<T>| tab.group == group && tab.park.is_none();
        let after = self.items.iter().enumerate().skip(from).find(|(_, tab)| shown(tab));
        let before = self.items[..from.min(self.items.len())].iter().rposition(shown);
        after.map(|(i, _)| i).or(before).unwrap_or(0)
    }

    fn close(&mut self, i: usize) {
        if self.items.get(i).is_none_or(|t| t.pinned) {
            return;
        }
        let (group, id) = (self.items[i].group, self.items[i].id);
        let was_active = i == self.active;
        self.items.remove(i);
        if i < self.active {
            self.active -= 1;
        }
        if was_active {
            self.select_index(self.neighbor(group, i));
        } else if self.recent[group] == id {
            self.recent[group] = self.items[self.neighbor(group, i)].id;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tabs_with(extra: usize) -> Tabs<u32> {
        let mut tabs = Tabs::new(0);
        for n in 1..=extra {
            tabs.apply(Action::New, || n as u32);
        }
        tabs
    }

    fn panes(tabs: &Tabs<u32>) -> Vec<u32> {
        tabs.items().iter().map(|t| t.pane).collect()
    }

    #[test]
    fn a_tab_moves_to_another_group_and_the_pinned_main_tab_does_not() {
        let mut tabs = tabs_with(1);
        let other = tabs.add_group(9);
        let moved = tabs.items()[1].id;
        tabs.move_to_group(moved, other);
        assert_eq!(tabs.group_of(moved), Some(other));
        assert_eq!(tabs.in_group(0).count(), 1);
        tabs.move_to_group(0, other);
        assert_eq!(tabs.group_of(0), Some(0));
        tabs.move_to_group(moved, 7);
        assert_eq!(tabs.group_of(moved), Some(other), "a group that does not exist is ignored");
    }

    #[test]
    fn starts_with_one_pinned_main_tab() {
        let tabs = tabs_with(0);
        assert_eq!(tabs.items().len(), 1);
        assert!(tabs.items()[0].pinned);
        assert_eq!(tabs.items()[0].title, MAIN_TITLE);
        assert_eq!(tabs.active(), 0);
    }

    #[test]
    fn new_tab_is_appended_and_selected() {
        let tabs = tabs_with(2);
        assert_eq!(panes(&tabs), [0, 1, 2]);
        assert_eq!(tabs.active(), 2);
        assert_eq!(tabs.items()[2].title, "shell 2");
    }

    #[test]
    fn push_selects_the_new_tab_and_returns_its_id() {
        let mut tabs = tabs_with(1);
        let id = tabs.push("agent".into(), 7);
        assert_eq!(tabs.active(), 2);
        assert_eq!(tabs.index_of(id), Some(2));
        assert_eq!(tabs.get_mut(id).unwrap().title, "agent");
        assert_eq!(tabs.items()[2].pane, 7);
    }

    #[test]
    fn remove_by_id_follows_the_tab_when_others_move() {
        let mut tabs = tabs_with(1);
        let id = tabs.push("agent".into(), 7);
        tabs.apply(Action::Close(1), || unreachable!());
        assert_eq!(tabs.index_of(id), Some(1));
        tabs.remove_id(id);
        assert_eq!(panes(&tabs), [0]);
        tabs.remove_id(id);
        assert_eq!(panes(&tabs), [0]);
    }

    #[test]
    fn tab_ids_are_unique_after_closing() {
        let mut tabs = tabs_with(2);
        tabs.apply(Action::Close(1), || unreachable!());
        tabs.apply(Action::New, || 9);
        let mut ids: Vec<u64> = tabs.items().iter().map(|t| t.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), tabs.items().len());
    }

    #[test]
    fn pinned_tab_cannot_be_closed() {
        let mut tabs = tabs_with(1);
        tabs.apply(Action::Close(0), || unreachable!());
        tabs.apply(Action::Select(0), || unreachable!());
        tabs.apply(Action::CloseActive, || unreachable!());
        assert_eq!(panes(&tabs), [0, 1]);
    }

    #[test]
    fn closing_the_active_last_tab_selects_its_left_neighbor() {
        let mut tabs = tabs_with(2);
        tabs.apply(Action::CloseActive, || unreachable!());
        assert_eq!(panes(&tabs), [0, 1]);
        assert_eq!(tabs.active(), 1);
    }

    #[test]
    fn closing_a_tab_left_of_active_keeps_the_same_tab_active() {
        let mut tabs = tabs_with(3);
        tabs.apply(Action::Close(1), || unreachable!());
        assert_eq!(panes(&tabs), [0, 2, 3]);
        assert_eq!(*tabs.active_pane_mut(), 3);
    }

    #[test]
    fn closing_a_tab_right_of_active_keeps_the_same_tab_active() {
        let mut tabs = tabs_with(3);
        tabs.apply(Action::Select(1), || unreachable!());
        tabs.apply(Action::Close(3), || unreachable!());
        assert_eq!(*tabs.active_pane_mut(), 1);
    }

    #[test]
    fn closing_the_active_middle_tab_selects_the_next_one() {
        let mut tabs = tabs_with(3);
        tabs.apply(Action::Select(2), || unreachable!());
        tabs.apply(Action::CloseActive, || unreachable!());
        assert_eq!(panes(&tabs), [0, 1, 3]);
        assert_eq!(*tabs.active_pane_mut(), 3);
    }

    #[test]
    fn select_out_of_range_is_ignored() {
        let mut tabs = tabs_with(1);
        tabs.apply(Action::Select(7), || unreachable!());
        assert_eq!(tabs.active(), 1);
    }

    #[test]
    fn close_out_of_range_is_ignored() {
        let mut tabs = tabs_with(1);
        tabs.apply(Action::Close(7), || unreachable!());
        assert_eq!(panes(&tabs), [0, 1]);
    }

    fn two_groups() -> Tabs<u32> {
        let mut tabs = tabs_with(1);
        tabs.add_group(10);
        tabs.push_in(1, "api work".into(), 11);
        tabs
    }

    fn titles_in(tabs: &Tabs<u32>, group: usize) -> Vec<&str> {
        tabs.in_group(group).map(|(_, tab)| tab.title.as_str()).collect()
    }

    #[test]
    fn a_new_group_has_its_own_pinned_main_tab_and_leaves_the_visible_group_alone() {
        let mut tabs = tabs_with(1);
        let group = tabs.add_group(10);
        assert_eq!(group, 1);
        assert_eq!(tabs.active_group(), 0);
        assert_eq!(tabs.active(), 1);
        assert_eq!(tabs.main_id(0), Some(0));
        let main = tabs.main_id(1).unwrap();
        assert!(tabs.items()[tabs.index_of(main).unwrap()].pinned);
        assert_eq!(titles_in(&tabs, 1), [MAIN_TITLE]);
    }

    #[test]
    fn ids_stay_unique_across_groups() {
        let mut tabs = two_groups();
        tabs.apply(Action::New, || 12);
        let mut ids: Vec<u64> = tabs.items().iter().map(|t| t.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), tabs.items().len());
    }

    #[test]
    fn push_goes_to_the_visible_group_and_push_in_selects_nothing() {
        let mut tabs = two_groups();
        assert_eq!(tabs.active_group(), 0);
        assert_eq!(tabs.active(), 1, "push_in did not move the selection");
        tabs.push("here".into(), 5);
        assert_eq!(titles_in(&tabs, 0), [MAIN_TITLE, "shell 1", "here"]);
        assert_eq!(titles_in(&tabs, 1), [MAIN_TITLE, "api work"]);
    }

    #[test]
    fn selecting_a_tab_of_another_group_shows_that_group() {
        let mut tabs = two_groups();
        let api = tabs.in_group(1).last().unwrap().0;
        tabs.apply(Action::Select(api), || unreachable!());
        assert_eq!(tabs.active_group(), 1);
        assert_eq!(*tabs.active_pane_mut(), 11);
    }

    #[test]
    fn select_nth_counts_inside_the_visible_group() {
        let mut tabs = two_groups();
        tabs.apply(Action::NextGroup, || unreachable!());
        tabs.apply(Action::SelectNth(0), || unreachable!());
        assert_eq!(*tabs.active_pane_mut(), 10);
        tabs.apply(Action::SelectNth(1), || unreachable!());
        assert_eq!(*tabs.active_pane_mut(), 11);
        tabs.apply(Action::SelectNth(2), || unreachable!());
        assert_eq!(*tabs.active_pane_mut(), 11, "out of range is ignored");
    }

    #[test]
    fn next_and_previous_group_wrap_and_each_group_returns_to_the_tab_it_had() {
        let mut tabs = two_groups();
        tabs.apply(Action::NextGroup, || unreachable!());
        assert_eq!(tabs.active_group(), 1);
        assert_eq!(*tabs.active_pane_mut(), 10, "the group's main tab, nothing else was selected");
        tabs.apply(Action::SelectNth(1), || unreachable!());
        tabs.apply(Action::NextGroup, || unreachable!());
        assert_eq!(tabs.active_group(), 0);
        assert_eq!(*tabs.active_pane_mut(), 1, "the shell that was selected in the first group");
        tabs.apply(Action::PrevGroup, || unreachable!());
        assert_eq!((tabs.active_group(), *tabs.active_pane_mut()), (1, 11));
    }

    #[test]
    fn show_group_picks_a_group_by_number_and_ignores_one_that_is_not_there() {
        let mut tabs = two_groups();
        tabs.apply(Action::ShowGroup(1), || unreachable!());
        assert_eq!(tabs.active_group(), 1);
        tabs.apply(Action::ShowGroup(5), || unreachable!());
        assert_eq!(tabs.active_group(), 1);
        tabs.apply(Action::ShowGroup(0), || unreachable!());
        assert_eq!(tabs.active_group(), 0);
    }

    #[test]
    fn one_group_has_nowhere_to_step_to() {
        let mut tabs = tabs_with(2);
        tabs.apply(Action::NextGroup, || unreachable!());
        tabs.apply(Action::PrevGroup, || unreachable!());
        assert_eq!(tabs.active(), 2);
    }

    #[test]
    fn selecting_in_a_group_on_show_selects_and_in_another_only_remembers() {
        let mut tabs = two_groups();
        let api = tabs.in_group(1).last().unwrap().1.id;
        tabs.select_id(api);
        assert_eq!((tabs.active_group(), tabs.active()), (0, 1), "the window stays in front");
        assert_eq!(tabs.selected_in(1), tabs.index_of(api).unwrap());
        tabs.apply(Action::NextGroup, || unreachable!());
        assert_eq!(*tabs.active_pane_mut(), 11);
    }

    #[test]
    fn closing_the_active_tab_stays_inside_its_group() {
        let mut tabs = two_groups();
        tabs.apply(Action::NextGroup, || unreachable!());
        tabs.apply(Action::SelectNth(1), || unreachable!());
        tabs.apply(Action::CloseActive, || unreachable!());
        assert_eq!((tabs.active_group(), *tabs.active_pane_mut()), (1, 10));
        assert_eq!(titles_in(&tabs, 0), [MAIN_TITLE, "shell 1"]);
    }

    #[test]
    fn closing_a_tab_in_a_group_out_of_sight_moves_what_that_group_remembers() {
        let mut tabs = two_groups();
        let api = tabs.in_group(1).last().unwrap().1.id;
        tabs.select_id(api);
        tabs.remove_id(api);
        tabs.apply(Action::NextGroup, || unreachable!());
        assert_eq!((tabs.active_group(), *tabs.active_pane_mut()), (1, 10));
    }

    fn ids_in(tabs: &Tabs<u32>, group: usize) -> Vec<u32> {
        tabs.in_group(group).map(|(_, tab)| tab.pane).collect()
    }

    #[test]
    fn move_swaps_a_tab_with_its_neighbour_and_the_active_tab_follows_its_tab() {
        let table = [
            (1, Step::Right, 3, [0, 2, 1, 3], 3),
            (2, Step::Left, 3, [0, 2, 1, 3], 3),
            (1, Step::Right, 1, [0, 2, 1, 3], 2),
            (3, Step::Left, 1, [0, 1, 3, 2], 1),
            (1, Step::Right, 2, [0, 2, 1, 3], 1),
        ];
        for (from, step, active, order, now) in table {
            let mut tabs = tabs_with(3);
            tabs.apply(Action::Select(active), || unreachable!());
            let pane = tabs.items()[active].pane;
            tabs.apply(Action::Move(from, step), || unreachable!());
            assert_eq!(panes(&tabs), order, "{from} {step:?} with {active} active");
            assert_eq!(tabs.active(), now, "{from} {step:?} with {active} active");
            assert_eq!(*tabs.active_pane_mut(), pane, "the active tab stays the same tab");
        }
    }

    #[test]
    fn move_refuses_the_ends_the_pinned_tab_and_a_tab_that_is_not_there() {
        for (from, step) in [
            (3, Step::Right),
            (1, Step::Left),
            (0, Step::Right),
            (0, Step::Left),
            (9, Step::Left),
            (9, Step::Right),
        ] {
            let mut tabs = tabs_with(3);
            tabs.apply(Action::Select(2), || unreachable!());
            tabs.apply(Action::Move(from, step), || unreachable!());
            assert_eq!(panes(&tabs), [0, 1, 2, 3], "{from} {step:?}");
            assert_eq!(tabs.active(), 2, "{from} {step:?}");
        }
    }

    #[test]
    fn move_active_moves_the_selected_tab() {
        let mut tabs = tabs_with(3);
        tabs.apply(Action::MoveActive(Step::Left), || unreachable!());
        assert_eq!((panes(&tabs), tabs.active()), (vec![0, 1, 3, 2], 2));
        tabs.apply(Action::MoveActive(Step::Left), || unreachable!());
        tabs.apply(Action::MoveActive(Step::Left), || unreachable!());
        assert_eq!((panes(&tabs), tabs.active()), (vec![0, 3, 1, 2], 1), "stops at the main tab");
        tabs.apply(Action::SelectNth(0), || unreachable!());
        tabs.apply(Action::MoveActive(Step::Right), || unreachable!());
        assert_eq!(panes(&tabs), [0, 3, 1, 2], "the pinned tab stays put");
    }

    #[test]
    fn move_stays_in_the_group_and_leaves_the_others_where_they_are() {
        let mut tabs = tabs_with(1);
        tabs.add_group(10);
        tabs.push_in(0, "late".into(), 2);
        tabs.push_in(1, "api work".into(), 11);
        tabs.push_in(0, "later".into(), 3);
        assert_eq!(panes(&tabs), [0, 1, 10, 2, 11, 3]);
        let first = tabs.index_of(tabs.in_group(0).nth(1).unwrap().1.id).unwrap();
        tabs.apply(Action::Move(first, Step::Right), || unreachable!());
        assert_eq!(ids_in(&tabs, 0), [0, 2, 1, 3]);
        assert_eq!(ids_in(&tabs, 1), [10, 11]);
        assert_eq!(panes(&tabs), [0, 2, 10, 1, 11, 3]);
        let last = tabs.items().len() - 1;
        tabs.apply(Action::Move(last, Step::Right), || unreachable!());
        let api = tabs.index_of(tabs.in_group(1).last().unwrap().1.id).unwrap();
        tabs.apply(Action::Move(api, Step::Left), || unreachable!());
        assert_eq!(ids_in(&tabs, 1), [10, 11], "the pinned tab of that group is not passed");
        assert_eq!(ids_in(&tabs, 0), [0, 2, 1, 3]);
    }

    #[test]
    fn a_group_out_of_sight_keeps_the_tab_it_remembers_when_its_tabs_move() {
        let mut tabs = two_groups();
        let extra = tabs.push_in(1, "more".into(), 12);
        tabs.select_id(extra);
        let index = tabs.index_of(extra).unwrap();
        tabs.apply(Action::Move(index, Step::Left), || unreachable!());
        assert_eq!(tabs.active_group(), 0);
        assert_eq!(tabs.items()[tabs.selected_in(1)].id, extra);
        assert_eq!(ids_in(&tabs, 1), [10, 12, 11]);
    }

    fn parked_ids(tabs: &Tabs<u32>, group: usize) -> Vec<u32> {
        tabs.parked_in(group).map(|(_, tab)| tab.pane).collect()
    }

    fn shown(tabs: &Tabs<u32>, group: usize) -> Vec<u32> {
        tabs.shown_in(group).map(|(_, tab)| tab.pane).collect()
    }

    fn park(tabs: &mut Tabs<u32>, pane: u32) {
        let id = tabs.items().iter().find(|tab| tab.pane == pane).unwrap().id;
        tabs.set_park(id, Park { why: Why::Idle, since_ms: 1 });
    }

    #[test]
    fn a_parked_tab_leaves_the_shown_list_and_keeps_its_place_in_the_whole_one() {
        let mut tabs = tabs_with(4);
        tabs.apply(Action::Select(1), || unreachable!());
        park(&mut tabs, 3);
        park(&mut tabs, 2);
        assert_eq!(panes(&tabs), [0, 1, 2, 3, 4]);
        assert_eq!(shown(&tabs, 0), [0, 1, 4]);
        assert_eq!(parked_ids(&tabs, 0), [2, 3]);
    }

    #[test]
    fn the_number_chords_count_the_shown_tabs_only() {
        let mut tabs = tabs_with(4);
        tabs.apply(Action::Select(1), || unreachable!());
        park(&mut tabs, 2);
        park(&mut tabs, 3);
        for (n, want) in [(0, 0), (1, 1), (2, 4), (3, 4)] {
            tabs.apply(Action::SelectNth(n), || unreachable!());
            assert_eq!(*tabs.active_pane_mut(), want, "tab {n}");
        }
    }

    #[test]
    fn selecting_a_parked_tab_by_any_way_brings_it_back() {
        let mut tabs = tabs_with(3);
        tabs.apply(Action::Select(1), || unreachable!());
        park(&mut tabs, 2);
        park(&mut tabs, 3);
        tabs.apply(Action::Select(2), || unreachable!());
        assert_eq!(parked_ids(&tabs, 0), [3]);
        assert_eq!(*tabs.active_pane_mut(), 2);
        let id = tabs.items()[3].id;
        tabs.select_id(id);
        assert_eq!(parked_ids(&tabs, 0), Vec::<u32>::new());
        assert_eq!(*tabs.active_pane_mut(), 3);
    }

    #[test]
    fn selecting_a_parked_tab_of_another_group_brings_it_back_and_it_shows_with_the_group() {
        let mut tabs = two_groups();
        let extra = tabs.push_in(1, "more".into(), 12);
        tabs.set_park(extra, Park { why: Why::Manual, since_ms: 0 });
        assert_eq!(tabs.selected_in(1), tabs.main_id(1).and_then(|id| tabs.index_of(id)).unwrap());
        tabs.select_id(extra);
        assert_eq!(tabs.items()[tabs.index_of(extra).unwrap()].park, None);
        tabs.apply(Action::NextGroup, || unreachable!());
        assert_eq!(*tabs.active_pane_mut(), 12);
    }

    #[test]
    fn a_group_never_comes_back_to_a_parked_tab() {
        let mut tabs = two_groups();
        let api = tabs.in_group(1).last().unwrap().1.id;
        tabs.select_id(api);
        tabs.set_park(api, Park { why: Why::Idle, since_ms: 0 });
        tabs.apply(Action::NextGroup, || unreachable!());
        assert_eq!((tabs.active_group(), *tabs.active_pane_mut()), (1, 10), "its main tab");
        assert_eq!(parked_ids(&tabs, 1), [11]);
    }

    #[test]
    fn parking_the_selected_tab_hands_the_selection_to_the_shown_tab_after_it_else_before() {
        let mut tabs = tabs_with(3);
        tabs.apply(Action::Select(2), || unreachable!());
        park(&mut tabs, 3);
        tabs.apply(Action::ParkActive, || unreachable!());
        assert_eq!(*tabs.active_pane_mut(), 1, "the tab after it was parked, so the one before");
        assert_eq!(parked_ids(&tabs, 0), [2, 3]);

        let mut tabs = tabs_with(3);
        tabs.apply(Action::Select(1), || unreachable!());
        tabs.apply(Action::Park(1), || unreachable!());
        assert_eq!(*tabs.active_pane_mut(), 2);
        assert_eq!(parked_ids(&tabs, 0), [1]);
        assert_eq!(tabs.items()[1].park.map(|park| park.why), Some(Why::Manual));
    }

    #[test]
    fn nothing_parks_the_pinned_tab_a_tab_that_is_not_there_or_one_with_nowhere_to_go() {
        let mut tabs = tabs_with(1);
        tabs.apply(Action::Park(0), || unreachable!());
        tabs.apply(Action::Park(9), || unreachable!());
        assert_eq!(parked_ids(&tabs, 0), Vec::<u32>::new());
        tabs.apply(Action::ParkActive, || unreachable!());
        assert_eq!(parked_ids(&tabs, 0), [1], "the main tab takes the selection");
        assert_eq!(tabs.active(), 0);
        assert!(!tabs.park_index(1, Why::Manual, 0), "already parked");
    }

    #[test]
    fn the_rules_never_park_the_selected_the_pinned_or_a_kept_tab() {
        let mut tabs = tabs_with(2);
        let (main, selected, other) = (tabs.items()[0].id, tabs.items()[2].id, tabs.items()[1].id);
        let park = Park { why: Why::Idle, since_ms: 0 };
        for id in [main, selected] {
            tabs.set_park(id, park);
        }
        tabs.apply(Action::ToggleKeep(1), || unreachable!());
        tabs.set_park(other, park);
        assert_eq!(parked_ids(&tabs, 0), Vec::<u32>::new());
        tabs.apply(Action::ToggleKeep(1), || unreachable!());
        tabs.set_park(other, park);
        assert_eq!(parked_ids(&tabs, 0), [1]);
    }

    #[test]
    fn never_park_unparks_a_tab_and_flips_back_and_the_pinned_tab_has_no_such_mark() {
        let mut tabs = tabs_with(2);
        tabs.apply(Action::Select(2), || unreachable!());
        park(&mut tabs, 1);
        tabs.apply(Action::ToggleKeep(1), || unreachable!());
        assert!(tabs.items()[1].keep);
        assert_eq!(tabs.items()[1].park, None);
        tabs.apply(Action::ToggleKeep(1), || unreachable!());
        assert!(!tabs.items()[1].keep);
        tabs.apply(Action::ToggleKeep(0), || unreachable!());
        assert!(!tabs.items()[0].keep);
    }

    #[test]
    fn moving_a_tab_steps_over_parked_neighbours_and_leaves_them_where_they_are() {
        let mut tabs = tabs_with(4);
        tabs.apply(Action::Select(1), || unreachable!());
        park(&mut tabs, 2);
        park(&mut tabs, 3);
        tabs.apply(Action::Move(1, Step::Right), || unreachable!());
        assert_eq!(
            panes(&tabs),
            [0, 4, 2, 3, 1],
            "swapped with the next shown tab, not a parked one"
        );
        assert_eq!(*tabs.active_pane_mut(), 1, "the selected tab is still the same tab");
        let at = tabs.items().iter().position(|tab| tab.pane == 1).unwrap();
        tabs.apply(Action::Move(at, Step::Left), || unreachable!());
        assert_eq!(panes(&tabs), [0, 1, 2, 3, 4]);
        assert_eq!(*tabs.active_pane_mut(), 1);
    }

    #[test]
    fn closing_the_selected_tab_selects_a_shown_neighbour_not_a_parked_one() {
        let mut tabs = tabs_with(3);
        tabs.apply(Action::Select(1), || unreachable!());
        park(&mut tabs, 2);
        tabs.apply(Action::CloseActive, || unreachable!());
        assert_eq!(panes(&tabs), [0, 2, 3]);
        assert_eq!(*tabs.active_pane_mut(), 3);
        assert_eq!(parked_ids(&tabs, 0), [2]);
    }
}
