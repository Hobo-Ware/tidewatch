//! Tab list and its reducer. Generic over the pane payload so the transitions are testable
//! without a pty. The UI collects `Action`s during the frame and applies them afterwards.

pub(crate) const MAIN_TITLE: &str = "main";

pub(crate) struct Tab<T> {
    pub(crate) id: u64,
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
}

pub(crate) struct Tabs<T> {
    items: Vec<Tab<T>>,
    active: usize,
    next_id: u64,
    opened: usize,
}

impl<T> Tabs<T> {
    pub(crate) fn new(main_pane: T) -> Self {
        Self {
            items: vec![Tab { id: 0, title: MAIN_TITLE.into(), pinned: true, pane: main_pane }],
            active: 0,
            next_id: 1,
            opened: 0,
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

    /// Appends a tab, selects it and returns its id.
    pub(crate) fn push(&mut self, title: String, pane: T) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.items.push(Tab { id, title, pinned: false, pane });
        self.active = self.items.len() - 1;
        id
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
                    self.active = i;
                }
            }
        }
    }

    fn close(&mut self, i: usize) {
        if self.items.get(i).is_none_or(|t| t.pinned) {
            return;
        }
        self.items.remove(i);
        if i < self.active || self.active >= self.items.len() {
            self.active -= 1;
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
}
