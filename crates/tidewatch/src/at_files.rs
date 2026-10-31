//! The `@` file list of a message box: which word is being typed, which of the repo's files match it,
//! and the token a pick leaves in the text (pure, table-tested).

use std::path::Path;

use crate::emoji_pick::Query;
use crate::palette::fuzzy;
use crate::tokens::{self, Target};

pub(crate) const MAX_ROWS: usize = 6;
const NAME_BONUS: i32 = 30;

#[derive(Clone, Copy)]
pub(crate) struct Source<'a> {
    pub(crate) root: &'a Path,
    pub(crate) paths: &'a [String],
}

/// The word the cursor (a byte offset) is at the end of, when it is an `@` that starts a word.
pub(crate) fn query_at(text: &str, cursor: usize) -> Option<Query> {
    let before = text.get(..cursor)?;
    let start = before.rfind('@')?;
    let term = &before[start + 1..];
    let word = !term.contains(char::is_whitespace) && !term.contains([tokens::OPEN, tokens::CLOSE]);
    let starts_word = before[..start].chars().next_back().is_none_or(char::is_whitespace);
    (word && starts_word).then(|| Query { start, term: term.to_owned() })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Hit {
    pub(crate) path: String,
    pub(crate) lit: Vec<usize>,
}

impl Hit {
    pub(crate) fn name_start(&self) -> usize {
        self.path.rfind('/').map_or(0, |slash| slash + 1)
    }
}

/// The best `limit` paths for `term`, a match on the file name before one only in its folders.
pub(crate) fn find(term: &str, paths: &[String], limit: usize) -> Vec<Hit> {
    if term.is_empty() {
        let hit = |path: &String| Hit { path: path.clone(), lit: Vec::new() };
        return paths.iter().take(limit).map(hit).collect();
    }
    let mut scored: Vec<(i32, usize, usize)> = paths
        .iter()
        .enumerate()
        .filter_map(|(index, path)| {
            let name_at = path.rfind('/').map_or(0, |slash| slash + 1);
            let whole = fuzzy(term, path).map(|m| m.score);
            let name = fuzzy(term, &path[name_at..]).map(|m| m.score + NAME_BONUS);
            whole.max(name).map(|score| (score, path.len(), index))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    scored
        .into_iter()
        .take(limit)
        .map(|(_, _, index)| {
            let path = paths[index].clone();
            let name_at = path.rfind('/').map_or(0, |slash| slash + 1);
            let in_name = fuzzy(term, &path[name_at..]).map(|m| m.positions);
            let whole = fuzzy(term, &path);
            let name_wins = whole.as_ref().is_none_or(|w| w.score < NAME_BONUS);
            let lit = match in_name {
                Some(positions) if name_wins => {
                    let shift = path[..name_at].chars().count();
                    positions.into_iter().map(|at| at + shift).collect()
                }
                _ => whole.map_or_else(Vec::new, |m| m.positions),
            };
            Hit { path, lit }
        })
        .collect()
}

pub(crate) fn target(root: &Path, hit: &Hit) -> Target {
    Target::File(root.join(&hit.path))
}

/// `@term` up to `cursor` (a byte offset) replaced by the token of `target`. Returns the cursor,
/// in chars, behind it.
pub(crate) fn pick(text: &mut String, query: &Query, cursor: usize, target: &Target) -> usize {
    text.replace_range(query.start..cursor, "");
    let at = text[..query.start].chars().count();
    tokens::insert(text, at, target)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths() -> Vec<String> {
        ["src/main.rs", "src/composer.rs", "docs/composer-notes.md", "README.md", "Cargo.toml"]
            .map(String::from)
            .to_vec()
    }

    #[test]
    fn an_at_that_starts_a_word_opens_the_list_and_anything_else_does_not() {
        let cases = [
            ("@", 1, Some((0, ""))),
            ("look at @comp", 13, Some((8, "comp"))),
            ("a\n@src/m", 8, Some((2, "src/m"))),
            ("mail me@host", 12, None),
            ("@comp more", 10, None),
            ("no at here", 10, None),
        ];
        for (text, cursor, want) in cases {
            let got = query_at(text, cursor).map(|q| (q.start, q.term));
            assert_eq!(got, want.map(|(start, term)| (start, term.to_owned())), "{text:?}");
        }
    }

    #[test]
    fn the_file_name_beats_a_match_only_in_the_folders() {
        let hits = find("composer", &paths(), 3);
        assert_eq!(
            hits.iter().map(|hit| hit.path.as_str()).collect::<Vec<_>>(),
            ["src/composer.rs", "docs/composer-notes.md"]
        );
    }

    #[test]
    fn an_empty_word_lists_the_first_files_and_a_miss_lists_none() {
        assert_eq!(find("", &paths(), 2).len(), 2);
        assert_eq!(find("zzzz", &paths(), 6), Vec::<Hit>::new());
    }

    #[test]
    fn the_lit_letters_of_a_name_match_sit_at_their_place_in_the_whole_path() {
        let hit = &find("mai", &paths(), 1)[0];
        assert_eq!(hit.path, "src/main.rs");
        assert_eq!(hit.lit, vec![4, 5, 6]);
    }

    #[test]
    fn a_pick_swaps_the_typed_word_for_the_file_where_the_cursor_was() {
        let mut text = "check @comp now".to_owned();
        let query = query_at(&text, 11).unwrap();
        let target = Target::File("/repo/src/composer.rs".into());
        let cursor = pick(&mut text, &query, 11, &target);
        assert_eq!(tokens::sendable(&text), "check /repo/src/composer.rs now");
        assert_eq!(text.chars().nth(cursor), Some(' '));
    }

    #[test]
    fn a_pick_at_the_end_leaves_a_space_to_keep_typing_in() {
        let mut text = "see @mai".to_owned();
        let query = query_at(&text, 8).unwrap();
        let cursor = pick(&mut text, &query, 8, &Target::File("/r/main.rs".into()));
        assert_eq!(tokens::sendable(&text), "see /r/main.rs");
        assert!(text.ends_with(' '));
        assert_eq!(cursor, text.chars().count());
    }
}
