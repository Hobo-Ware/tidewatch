//! Emoji short names in the message box: `:` plus two letters opens a list, `:tada:` and a space
//! turns into the emoji. The names are GitHub's (the `emojis` crate), the same as Slack's. Pure.

use std::cmp::Reverse;
use std::collections::HashMap;
use std::ops::Range;

use emojis::{Emoji, UnicodeVersion};

pub(crate) const MAX_ROWS: usize = 6;
const MIN_TERM: usize = 2;
const OPENERS: &str = "([{\"'";

fn is_code_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '-')
}

fn starts_a_name(before_colon: &str) -> bool {
    before_colon.chars().next_back().is_none_or(|c| c.is_whitespace() || OPENERS.contains(c))
}

/// A short name being typed: the `:` at byte `start` and the letters after it up to the cursor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Query {
    pub(crate) start: usize,
    pub(crate) term: String,
}

/// The short name the cursor (a byte offset) is at the end of, when it has a `:` that starts a
/// word and at least two letters after it.
pub(crate) fn query_at(text: &str, cursor: usize) -> Option<Query> {
    let before = text.get(..cursor)?;
    let start = before.rfind(':')?;
    let term = &before[start + 1..];
    let named = term.chars().count() >= MIN_TERM && term.chars().all(is_code_char);
    (named && starts_a_name(&before[..start]))
        .then(|| Query { start, term: term.to_ascii_lowercase() })
}

/// How many times each emoji was picked from the list, by its character.
#[derive(Clone, Debug, Default)]
pub(crate) struct Usage(HashMap<&'static str, u32>);

impl Usage {
    pub(crate) fn bump(&mut self, emoji: &'static Emoji) {
        *self.0.entry(emoji.as_str()).or_default() += 1;
    }

    pub(crate) fn count(&self, emoji: &Emoji) -> u32 {
        self.0.get(emoji.as_str()).copied().unwrap_or(0)
    }
}

/// One row of the list: the emoji, the short name that matched and where in it the typed letters
/// are (the `:` counts as part of a prefix match).
#[derive(Clone, Debug)]
pub(crate) struct Hit {
    pub(crate) emoji: &'static Emoji,
    pub(crate) code: &'static str,
    pub(crate) lit: Range<usize>,
    pub(crate) prefix: bool,
}

fn best_code(emoji: &'static Emoji, term: &str) -> Option<(&'static str, usize)> {
    emoji
        .shortcodes()
        .filter_map(|code| code.find(term).map(|at| (code, at)))
        .min_by_key(|(code, at)| (*at != 0, code.len(), *code))
}

const FONT_UNICODE: UnicodeVersion = UnicodeVersion::new(15, 0);

/// Whether the bundled emoji font draws `emoji`: one code point (the font cannot join a sequence
/// such as a flag, a family or a skin tone) from a Unicode version it covers.
pub(crate) fn renders(emoji: &Emoji) -> bool {
    let marks = emoji.as_str().chars().filter(|c| !matches!(c, '\u{fe0f}')).count();
    marks == 1 && emoji.unicode_version() <= FONT_UNICODE
}

/// The emoji whose short name starts with `term`, then those that contain it, at most `limit`.
/// Within each group the most used come first, then the closest to what was typed. Emoji the font
/// cannot draw are left out.
pub(crate) fn find(term: &str, usage: &Usage, limit: usize) -> Vec<Hit> {
    let mut hits: Vec<Hit> = emojis::iter()
        .filter_map(|emoji| {
            let (code, at) = best_code(emoji, term)?;
            renders(emoji).then(|| Hit { emoji, code, lit: at..at + term.len(), prefix: at == 0 })
        })
        .collect();
    hits.sort_by_key(|hit| {
        (
            !hit.prefix,
            hit.code.len() != term.len(),
            Reverse(usage.count(hit.emoji)),
            hit.code.len(),
            hit.code,
        )
    });
    hits.truncate(limit);
    hits
}

/// `text` with the short name at `query` (up to byte `cursor`) replaced by `emoji`. Returns the new
/// cursor as a byte offset.
pub(crate) fn insert(text: &mut String, query: &Query, cursor: usize, emoji: &Emoji) -> usize {
    text.replace_range(query.start..cursor, emoji.as_str());
    query.start + emoji.as_str().len()
}

/// A finished `:name:` just before a space typed at `cursor`.
#[derive(Debug)]
pub(crate) struct Closed {
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) emoji: &'static Emoji,
}

/// The `:name:` that ends right before the white space the cursor (a byte offset) just passed.
pub(crate) fn closed_before(text: &str, cursor: usize) -> Option<Closed> {
    let before = text.get(..cursor)?;
    let end = before.trim_end_matches(char::is_whitespace).len();
    if end == before.len() {
        return None;
    }
    let named = before[..end].strip_suffix(':')?;
    let start = named.rfind(':')?;
    let code = &named[start + 1..];
    let valid =
        !code.is_empty() && code.chars().all(is_code_char) && starts_a_name(&named[..start]);
    let emoji = emojis::get_by_shortcode(&code.to_ascii_lowercase()).filter(|_| valid)?;
    Some(Closed { start, end, emoji })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Key {
    Up,
    Down,
    Accept,
    Close,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Pressed {
    Moved,
    Accepted(usize),
    Closed,
}

/// Which row of the list is picked, and which name was closed with Esc. A new name starts at the
/// first row and opens again.
#[derive(Clone, Debug, Default)]
pub(crate) struct Menu {
    picked: usize,
    seen: Option<Query>,
    closed: Option<Query>,
}

impl Menu {
    pub(crate) fn follow(&mut self, query: Option<&Query>) {
        if self.seen.as_ref() != query {
            self.picked = 0;
            self.seen = query.cloned();
        }
        if self.closed.as_ref() != query {
            self.closed = None;
        }
    }

    pub(crate) fn is_open(&self, query: Option<&Query>) -> bool {
        query.is_some() && self.closed.as_ref() != query
    }

    pub(crate) fn picked_in(&self, rows: usize) -> usize {
        self.picked.min(rows.saturating_sub(1))
    }

    pub(crate) fn press(&mut self, key: Key, rows: usize) -> Pressed {
        let at = self.picked_in(rows);
        let count = rows.max(1);
        match key {
            Key::Down => {
                self.picked = (at + 1) % count;
                Pressed::Moved
            }
            Key::Up => {
                self.picked = (at + count - 1) % count;
                Pressed::Moved
            }
            Key::Accept => Pressed::Accepted(at),
            Key::Close => {
                self.closed = self.seen.clone();
                Pressed::Closed
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codes(term: &str, usage: &Usage) -> Vec<&'static str> {
        find(term, usage, MAX_ROWS).iter().map(|hit| hit.code).collect()
    }

    fn glyph(emoji: &str) -> &'static Emoji {
        emojis::get(emoji).unwrap()
    }

    #[test]
    fn a_name_opens_after_a_colon_that_starts_a_word_and_two_letters() {
        let cases = [
            ("Nice :smi", 9, Some((5, "smi"))),
            (":sm", 3, Some((0, "sm"))),
            (":s", 2, None),
            (":", 1, None),
            ("a\n:ti", 5, Some((2, "ti"))),
            ("see (:ok", 8, Some((5, "ok"))),
            ("10:30", 5, None),
            ("http://x", 8, None),
            ("a:smi", 5, None),
            (":smile:", 7, None),
            (":SMI", 4, Some((0, "smi"))),
            (":+1", 3, Some((0, "+1"))),
            (":s m", 4, None),
            ("no colon", 8, None),
        ];
        for (text, cursor, want) in cases {
            let got = query_at(text, cursor).map(|q| (q.start, q.term));
            assert_eq!(got, want.map(|(start, term)| (start, term.to_owned())), "{text:?}");
        }
    }

    #[test]
    fn a_cursor_in_the_middle_reads_only_what_is_before_it() {
        assert_eq!(query_at(":smile and more", 4).map(|q| q.term), Some("smi".to_owned()));
        assert_eq!(query_at("é:smi", 3), None);
        assert_eq!(query_at("é :smi", 7).map(|q| q.start), Some(3));
    }

    #[test]
    fn prefix_matches_come_before_matches_that_only_contain() {
        let list = codes("smi", &Usage::default());
        assert_eq!(list.len(), MAX_ROWS);
        let first_contains = list.iter().position(|code| !code.starts_with("smi"));
        let last_prefix = list.iter().rposition(|code| code.starts_with("smi"));
        assert!(
            first_contains.is_none_or(|at| last_prefix.is_none_or(|last| last < at)),
            "{list:?}"
        );
        assert_eq!(list[0], "smile");
    }

    #[test]
    fn an_exact_name_comes_first_and_shorter_names_before_longer_ones() {
        assert_eq!(codes("tada", &Usage::default())[0], "tada");
        let list = codes("smile", &Usage::default());
        assert_eq!(list[0], "smile");
        let lens: Vec<usize> =
            list.iter().filter(|c| c.starts_with("smile")).map(|c| c.len()).collect();
        assert!(lens.windows(2).all(|pair| pair[0] <= pair[1]), "{list:?}");
    }

    #[test]
    fn a_name_that_only_contains_the_letters_is_found_after_the_prefixes() {
        let hits = find("miling", &Usage::default(), MAX_ROWS);
        assert!(!hits.is_empty());
        assert!(hits.iter().all(|hit| !hit.prefix));
        let hit = &hits[0];
        assert_eq!(&hit.code[hit.lit.clone()], "miling");
    }

    #[test]
    fn the_highlight_is_the_typed_letters_in_the_name() {
        let hit = find("smi", &Usage::default(), 1).remove(0);
        assert_eq!((hit.code, hit.lit.clone(), hit.prefix), ("smile", 0..3, true));
    }

    #[test]
    fn the_most_used_emoji_rise_within_their_group() {
        let mut usage = Usage::default();
        let before = codes("smi", &usage);
        let smirk = emojis::get_by_shortcode("smirk").unwrap();
        for _ in 0..3 {
            usage.bump(smirk);
        }
        let after = codes("smi", &usage);
        assert_eq!(before[0], "smile");
        assert_eq!(after[0], "smirk");
        assert_eq!(usage.count(smirk), 3);
        assert_eq!(usage.count(emojis::get_by_shortcode("smile").unwrap()), 0);
    }

    #[test]
    fn use_never_lifts_a_name_that_only_contains_over_a_prefix_match() {
        let mut usage = Usage::default();
        let slightly = emojis::get_by_shortcode("slightly_smiling_face").unwrap();
        for _ in 0..9 {
            usage.bump(slightly);
        }
        let list = codes("smi", &usage);
        assert_ne!(list[0], "slightly_smiling_face");
    }

    #[test]
    fn only_single_code_point_emoji_the_font_covers_are_offered() {
        let cases = [
            ("smile", true),
            ("heart", true),
            ("tada", true),
            ("hot_face", true),
            ("man_technologist", false),
            ("family_man_woman_girl", false),
            ("flag_netherlands", false),
            ("keycap_ten", true),
            ("1234", true),
        ];
        for (code, want) in cases {
            let emoji = emojis::get_by_shortcode(code);
            assert_eq!(emoji.is_some_and(renders), want, "{code}");
        }
        let all = find("a", &Usage::default(), usize::MAX);
        assert!(all.iter().all(|hit| renders(hit.emoji)));
    }

    #[test]
    fn no_match_is_an_empty_list() {
        assert!(find("zzzzqq", &Usage::default(), MAX_ROWS).is_empty());
    }

    #[test]
    fn picking_replaces_the_typed_name_and_returns_the_cursor_after_the_emoji() {
        let mut text = "Nice work :smi".to_owned();
        let query = query_at(&text, text.len()).unwrap();
        let cursor = insert(&mut text, &query, 14, glyph("😄"));
        assert_eq!(text, "Nice work 😄");
        assert_eq!(cursor, text.len());
    }

    #[test]
    fn a_whole_name_followed_by_a_space_turns_into_the_emoji() {
        let cases = [
            (":tada: ", Some((0, 6, "🎉"))),
            ("go :tada: ", Some((3, 9, "🎉"))),
            (":TADA:\n", Some((0, 6, "🎉"))),
            (":tada:", None),
            (":nopenotaname: ", None),
            (":: ", None),
            ("a:tada: ", None),
            (":tada", None),
            ("tada: ", None),
        ];
        for (text, want) in cases {
            let got = closed_before(text, text.len())
                .map(|closed| (closed.start, closed.end, closed.emoji.as_str()));
            assert_eq!(got, want, "{text:?}");
        }
    }

    #[test]
    fn the_plus_one_name_works_both_ways() {
        assert_eq!(codes("+1", &Usage::default())[0], "+1");
        assert_eq!(closed_before(":+1: ", 5).map(|c| c.emoji.as_str()), Some("👍"));
    }

    #[test]
    fn up_and_down_wrap_around_the_rows_and_esc_closes_only_that_name() {
        let ask = |term: &str| Query { start: 0, term: term.to_owned() };
        let mut menu = Menu::default();
        menu.follow(Some(&ask("smi")));
        assert_eq!(menu.press(Key::Up, 4), Pressed::Moved);
        assert_eq!(menu.picked_in(4), 3);
        assert_eq!(menu.press(Key::Down, 4), Pressed::Moved);
        assert_eq!(menu.picked_in(4), 0);
        assert_eq!(menu.press(Key::Down, 4), Pressed::Moved);
        assert_eq!(menu.press(Key::Accept, 4), Pressed::Accepted(1));
        assert!(menu.is_open(Some(&ask("smi"))));
        assert_eq!(menu.press(Key::Close, 4), Pressed::Closed);
        assert!(!menu.is_open(Some(&ask("smi"))));
        menu.follow(Some(&ask("smil")));
        assert!(menu.is_open(Some(&ask("smil"))));
        assert_eq!(menu.picked_in(4), 0);
        assert!(!menu.is_open(None));
    }

    #[test]
    fn the_pick_is_held_inside_a_list_that_got_shorter() {
        let mut menu = Menu::default();
        menu.follow(Some(&Query { start: 0, term: "sm".to_owned() }));
        menu.press(Key::Up, 6);
        assert_eq!(menu.picked_in(6), 5);
        assert_eq!(menu.picked_in(2), 1);
        assert_eq!(menu.picked_in(0), 0);
    }
}
