//! Taking a whole group (a repo's tabs) out of the list, or folding it into another, and renumbering
//! the groups after it.

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

    /// Folds group `source` into group `target`: its tabs follow the target's own, in order, with
    /// their ids, marks and panes untouched, and the group goes, the ones after it moving down by
    /// one. Its pinned main tab is dropped (its id is returned) unless `keep_main` gives it a
    /// title, in which case it stays as an ordinary tab, first of the ones that moved. The tab that
    /// was active stays active; if it was the dropped main, the target's selected tab is.
    /// Changes nothing for a group that is not there or `source == target`.
    pub(crate) fn merge_group(
        &mut self,
        source: usize,
        target: usize,
        keep_main: Option<&str>,
    ) -> Vec<u64> {
        let groups = self.recent.len();
        if source == target || source >= groups || target >= groups {
            return Vec::new();
        }
        let active_id = self.items[self.active].id;
        let (mut moved, mut rest): (Vec<_>, Vec<_>) =
            std::mem::take(&mut self.items).into_iter().partition(|tab| tab.group == source);
        let mut gone = Vec::new();
        moved.retain_mut(|tab| match (tab.pinned, &keep_main) {
            (true, Some(title)) => {
                tab.pinned = false;
                (*title).clone_into(&mut tab.title);
                true
            }
            (true, None) => {
                gone.push(tab.id);
                false
            }
            (false, _) => true,
        });
        let at = rest.iter().rposition(|tab| tab.group == target).map_or(rest.len(), |at| at + 1);
        for tab in &mut moved {
            tab.group = target;
        }
        rest.splice(at..at, moved);
        self.items = rest;
        for tab in &mut self.items {
            if tab.group > source {
                tab.group -= 1;
            }
        }
        self.recent.remove(source);
        if let Some(index) = self.index_of(active_id) {
            self.active = index;
        } else {
            let target = target - usize::from(target > source);
            let index = self.index_of(self.recent[target]).unwrap_or(0);
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

    fn titles(tabs: &Tabs<u32>, group: usize) -> Vec<String> {
        tabs.in_group(group).map(|(_, tab)| tab.title.clone()).collect()
    }

    #[test]
    fn a_folded_group_follows_the_targets_own_tabs_and_its_main_goes() {
        let mut tabs = Tabs::new(0);
        let linked = tabs.add_group(10);
        tabs.push_in(0, "a".into(), 1);
        tabs.push_in(linked, "b".into(), 11);
        tabs.push_in(0, "c".into(), 2);
        tabs.push_in(linked, "d".into(), 12);
        let main = tabs.main_id(linked).unwrap();
        let ids: Vec<u64> = tabs.in_group(linked).skip(1).map(|(_, tab)| tab.id).collect();
        assert_eq!(tabs.merge_group(linked, 0, None), [main]);
        assert_eq!(titles(&tabs, 0), ["main", "a", "c", "b", "d"]);
        assert_eq!(tabs.items().len(), 5);
        let moved: Vec<u64> = tabs.in_group(0).skip(3).map(|(_, tab)| tab.id).collect();
        assert_eq!(moved, ids, "the ids travel with the tabs");
        assert_eq!(panes(&tabs).iter().map(|(group, _)| *group).max(), Some(0));
    }

    #[test]
    fn a_main_that_is_kept_becomes_the_first_ordinary_tab_of_what_moved() {
        let mut tabs = three_groups();
        tabs.push_in(1, "b work".into(), 11);
        let kept = tabs.main_id(1).unwrap();
        assert_eq!(tabs.merge_group(1, 0, Some("web-fix")), Vec::<u64>::new());
        assert_eq!(titles(&tabs, 0), ["main", "web-fix", "b work"]);
        let tab = tabs.get_mut(kept).unwrap();
        assert!(!tab.pinned);
        assert_eq!(tab.pane, 10);
        assert_eq!(tabs.in_group(0).filter(|(_, tab)| tab.pinned).count(), 1);
        assert_eq!(panes(&tabs), [(0, 0), (0, 10), (0, 11), (1, 20), (1, 21)]);
    }

    #[test]
    fn the_active_tab_stays_active_wherever_the_folded_group_sat() {
        let mut tabs = three_groups();
        tabs.push_in(1, "b work".into(), 11);
        tabs.apply(Action::ShowGroup(1), || unreachable!());
        tabs.apply(Action::SelectNth(1), || unreachable!());
        assert_eq!(*tabs.active_pane_mut(), 11);
        tabs.merge_group(1, 2, None);
        assert_eq!((tabs.active_group(), *tabs.active_pane_mut()), (1, 11));
        let mut tabs = three_groups();
        tabs.apply(Action::ShowGroup(2), || unreachable!());
        tabs.apply(Action::SelectNth(1), || unreachable!());
        tabs.merge_group(0, 1, None);
        assert_eq!((tabs.active_group(), *tabs.active_pane_mut()), (1, 21));
    }

    #[test]
    fn a_dropped_main_that_was_active_gives_way_to_the_targets_selected_tab() {
        let mut tabs = three_groups();
        tabs.push_in(0, "a".into(), 1);
        tabs.apply(Action::SelectNth(1), || unreachable!());
        tabs.apply(Action::ShowGroup(1), || unreachable!());
        assert_eq!(*tabs.active_pane_mut(), 10);
        tabs.merge_group(1, 0, None);
        assert_eq!((tabs.active_group(), tabs.items()[tabs.active()].title.as_str()), (0, "a"));
    }

    #[test]
    fn marks_survive_the_fold() {
        let mut tabs = three_groups();
        let parked = tabs.push_in(1, "b work".into(), 11);
        let kept = tabs.push_in(1, "keep".into(), 12);
        tabs.get_mut(parked).unwrap().park =
            Some(crate::park::Park { why: crate::park::Why::Manual, since_ms: 5 });
        tabs.get_mut(kept).unwrap().keep = true;
        tabs.merge_group(1, 0, None);
        assert!(tabs.get_mut(parked).unwrap().park.is_some());
        assert!(tabs.get_mut(kept).unwrap().keep);
    }

    #[test]
    fn a_missing_group_or_the_same_group_changes_nothing() {
        let mut tabs = three_groups();
        assert_eq!(tabs.merge_group(1, 1, None), Vec::<u64>::new());
        assert_eq!(tabs.merge_group(7, 0, None), Vec::<u64>::new());
        assert_eq!(tabs.merge_group(0, 7, None), Vec::<u64>::new());
        assert_eq!(panes(&tabs).len(), 4);
    }
}
