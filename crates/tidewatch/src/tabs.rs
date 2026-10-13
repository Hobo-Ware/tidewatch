//! Tab list and its reducer. Generic over the pane payload so the transitions are testable
//! without a pty. The UI collects `Action`s during the frame and applies them afterwards.
//!
//! Every tab belongs to a group, one per repo (or folder) the window holds, each with its own
//! pinned main tab. One list keeps tab ids unique across the window; the visible group is the one
//! the active tab is in, so selecting a tab of another group switches to it.

pub(crate) const MAIN_TITLE: &str = "main";

pub(crate) struct Tab<T> {
    pub(crate) id: u64,
    pub(crate) group: usize,
    pub(crate) title: String,
    pub(crate) pinned: bool,
    pub(crate) pane: T,
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
        self.items.push(Tab { id, group, title: MAIN_TITLE.into(), pinned: true, pane: main_pane });
        self.recent.push(id);
        group
    }

    /// The tabs of one group with their positions in the whole list, in order.
    pub(crate) fn in_group(&self, group: usize) -> impl Iterator<Item = (usize, &Tab<T>)> {
        self.items.iter().enumerate().filter(move |(_, tab)| tab.group == group)
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
            .or_else(|| self.in_group(group).next().map(|(i, _)| i))
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
        self.items.push(Tab { id, group, title, pinned: false, pane });
        id
    }

    /// Selects a tab. In a group that is not on show it only becomes what that group shows once it
    /// is, so a restored tab never takes the window from the one in front.
    pub(crate) fn select_id(&mut self, id: u64) {
        let Some(index) = self.index_of(id) else { return };
        if self.items[index].group == self.active_group() {
            self.select_index(index);
        } else {
            self.recent[self.items[index].group] = id;
        }
    }

    pub(crate) fn index_of(&self, id: u64) -> Option<usize> {
        self.items.iter().position(|t| t.id == id)
    }

    pub(crate) fn get_mut(&mut self, id: u64) -> Option<&mut Tab<T>> {
        self.items.iter_mut().find(|t| t.id == id)
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
                let found = self.in_group(self.active_group()).nth(n).map(|(i, _)| i);
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
        }
    }

    fn select_index(&mut self, index: usize) {
        self.active = index;
        self.recent[self.items[index].group] = self.items[index].id;
    }

    fn show_group(&mut self, group: usize) {
        let group = group % self.recent.len();
        self.select_index(self.selected_in(group));
    }

    /// The tab of a group at `from`, else the one before it. A group always has its main tab.
    fn neighbor(&self, group: usize, from: usize) -> usize {
        let after = self.items.iter().enumerate().skip(from).find(|(_, tab)| tab.group == group);
        let before =
            self.items[..from.min(self.items.len())].iter().rposition(|tab| tab.group == group);
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
}
