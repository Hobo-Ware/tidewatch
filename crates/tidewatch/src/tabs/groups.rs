//! Taking a whole group (a repo's tabs) out of the list and renumbering the groups after it.

use super::Tabs;

impl<T> Tabs<T> {
    /// Removes a group that has only its pinned main tab left and returns the ids of what went.
    /// The groups after it move down by one. When the group was on show, the one before it comes
    /// forward (the first, if it was the first). Returns nothing, and changes nothing, for the only
    /// group, a group that is not there, or one that still has tabs of its own.
    pub(crate) fn remove_group(&mut self, group: usize) -> Vec<u64> {
        let only_main = self.in_group(group).all(|(_, tab)| tab.pinned);
        if self.recent.len() < 2 || group >= self.recent.len() || !only_main {
            return Vec::new();
        }
        let active_id = self.items[self.active].id;
        let mut gone = Vec::new();
        self.items.retain(|tab| {
            let stays = tab.group != group;
            if !stays {
                gone.push(tab.id);
            }
            stays
        });
        for tab in &mut self.items {
            if tab.group > group {
                tab.group -= 1;
            }
        }
        self.recent.remove(group);
        if let Some(index) = self.index_of(active_id) {
            self.active = index;
        } else {
            let next = group.saturating_sub(1);
            let index = self.index_of(self.recent[next]).unwrap_or(0);
            self.select_index(index);
        }
        gone
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Action, MAIN_TITLE, Tabs};

    fn three_groups() -> Tabs<u32> {
        let mut tabs = Tabs::new(0);
        tabs.add_group(10);
        tabs.add_group(20);
        tabs.push_in(2, "c work".into(), 21);
        tabs
    }

    fn panes(tabs: &Tabs<u32>) -> Vec<(usize, u32)> {
        tabs.items().iter().map(|tab| (tab.group, tab.pane)).collect()
    }

    #[test]
    fn a_group_with_only_its_main_tab_goes_and_the_ones_after_it_move_down() {
        let mut tabs = three_groups();
        let main = tabs.main_id(1).unwrap();
        assert_eq!(tabs.remove_group(1), [main]);
        assert_eq!(panes(&tabs), [(0, 0), (1, 20), (1, 21)]);
        assert_eq!(tabs.in_group(1).next().unwrap().1.title, MAIN_TITLE);
        assert_eq!(tabs.active_group(), 0);
    }

    #[test]
    fn the_group_on_show_gives_way_to_the_one_before_it() {
        let mut tabs = three_groups();
        tabs.apply(Action::ShowGroup(1), || unreachable!());
        tabs.remove_group(1);
        assert_eq!((tabs.active_group(), *tabs.active_pane_mut()), (0, 0));
    }

    #[test]
    fn removing_the_first_group_brings_the_next_one_forward() {
        let mut tabs = three_groups();
        tabs.remove_group(0);
        assert_eq!(panes(&tabs), [(0, 10), (1, 20), (1, 21)]);
        assert_eq!((tabs.active_group(), *tabs.active_pane_mut()), (0, 10));
    }

    #[test]
    fn a_group_that_is_not_in_front_keeps_the_tab_that_is() {
        let mut tabs = three_groups();
        tabs.apply(Action::ShowGroup(2), || unreachable!());
        tabs.apply(Action::SelectNth(1), || unreachable!());
        tabs.remove_group(0);
        assert_eq!((tabs.active_group(), *tabs.active_pane_mut()), (1, 21));
        tabs.apply(Action::PrevGroup, || unreachable!());
        tabs.apply(Action::NextGroup, || unreachable!());
        assert_eq!(*tabs.active_pane_mut(), 21, "the group still remembers its tab");
    }

    #[test]
    fn a_group_with_tabs_of_its_own_the_last_group_and_a_missing_one_stay() {
        let mut tabs = three_groups();
        assert_eq!(tabs.remove_group(2), Vec::<u64>::new());
        assert_eq!(tabs.remove_group(7), Vec::<u64>::new());
        let mut single = Tabs::new(0);
        assert_eq!(single.remove_group(0), Vec::<u64>::new());
        assert_eq!(panes(&tabs).len(), 4);
        assert_eq!(panes(&single).len(), 1);
    }
}
