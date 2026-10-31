//! Files in the sentence: an attached picture, file or folder is a token inside the box text, spelled
//! as hex between two private-use marks, so the text alone is the state of a draft, a recalled
//! message and a held one.

use std::fmt::Write;
use std::ops::Range;
use std::path::{Path, PathBuf};

pub(crate) const OPEN: char = '\u{E000}';
pub(crate) const CLOSE: char = '\u{E001}';
const PASTED: &str = "pasted:";
const AFTER_WORD: &str = ",.;:!?)]}";
const BEFORE_WORD: &str = "([{\"'";

/// What a token stands for: a file or folder on the disk, or the numbered picture a transcript holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Target {
    File(PathBuf),
    Pasted(u32),
}

impl Target {
    fn spelled(&self) -> String {
        match self {
            Self::File(path) => path.to_string_lossy().into_owned(),
            Self::Pasted(number) => format!("{PASTED}{number}"),
        }
    }

    fn parse(spelled: &str) -> Option<Self> {
        if let Some(number) = spelled.strip_prefix(PASTED) {
            return number.parse().ok().map(Self::Pasted);
        }
        Path::new(spelled).is_absolute().then(|| Self::File(PathBuf::from(spelled)))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Token {
    pub(crate) bytes: Range<usize>,
    pub(crate) chars: Range<usize>,
    pub(crate) target: Target,
}

fn escape(spelled: &str) -> String {
    spelled.bytes().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}

fn unescape(body: &str) -> Option<String> {
    if body.is_empty() || !body.len().is_multiple_of(2) {
        return None;
    }
    let bytes: Option<Vec<u8>> = (0..body.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(body.get(at..at + 2)?, 16).ok())
        .collect();
    String::from_utf8(bytes?).ok()
}

pub(crate) fn encode(target: &Target) -> String {
    format!("{OPEN}{}{CLOSE}", escape(&target.spelled()))
}

fn byte_of(text: &str, chars: usize) -> usize {
    text.char_indices().nth(chars).map_or(text.len(), |(byte, _)| byte)
}

/// The well formed tokens of `text`, in order. A stray `OPEN` or `CLOSE`, an empty or broken token is
/// not one.
pub(crate) fn find(text: &str) -> Vec<Token> {
    let mut found = Vec::new();
    let mut open: Option<(usize, usize)> = None;
    for (chars, (byte, c)) in text.char_indices().enumerate() {
        match c {
            OPEN => open = Some((byte, chars)),
            CLOSE => {
                let Some((from_byte, from_chars)) = open.take() else { continue };
                let body = &text[from_byte + OPEN.len_utf8()..byte];
                if let Some(target) = unescape(body).as_deref().and_then(Target::parse) {
                    found.push(Token {
                        bytes: from_byte..byte + CLOSE.len_utf8(),
                        chars: from_chars..chars + 1,
                        target,
                    });
                }
            }
            c if c.is_whitespace() => open = None,
            _ => {}
        }
    }
    found
}

pub(crate) fn has_tokens(text: &str) -> bool {
    text.contains(OPEN)
}

/// `target` put into `text` at char position `cursor`, with a space on a side where it would touch a
/// word. Returns the cursor after it (and after the space it added behind it).
pub(crate) fn insert(text: &mut String, cursor: usize, target: &Target) -> usize {
    let at = byte_of(text, cursor.min(text.chars().count()));
    let before = text[..at].chars().next_back();
    let after = text[at..].chars().next();
    let lead = before.is_some_and(|c| !c.is_whitespace() && !BEFORE_WORD.contains(c));
    let trail = match after {
        None => true,
        Some(c) => !c.is_whitespace() && !AFTER_WORD.contains(c),
    };
    let mut put = String::new();
    if lead {
        put.push(' ');
    }
    put.push_str(&encode(target));
    if trail {
        put.push(' ');
    }
    text.insert_str(at, &put);
    cursor.min(text.chars().count()) + put.chars().count()
}

/// `targets` put after everything in `text`, as a draft saved with a list of files comes back.
pub(crate) fn append(text: &mut String, targets: impl IntoIterator<Item = Target>) {
    for target in targets {
        let end = text.chars().count();
        insert(text, end, &target);
    }
}

/// Backspace at `cursor` when a token ends there: the token goes whole, like a word. Returns where
/// the cursor goes.
pub(crate) fn remove_before(text: &mut String, cursor: usize) -> Option<usize> {
    let token = find(text).into_iter().find(|token| token.chars.end == cursor)?;
    text.replace_range(token.bytes, "");
    Some(token.chars.start)
}

pub(crate) fn remove_after(text: &mut String, cursor: usize) -> Option<usize> {
    let token = find(text).into_iter().find(|token| token.chars.start == cursor)?;
    text.replace_range(token.bytes, "");
    Some(cursor)
}

/// A cursor inside a token goes to the end when it moved forward from `was`, else to the start.
pub(crate) fn snap(text: &str, cursor: usize, was: usize) -> usize {
    find(text)
        .into_iter()
        .find(|token| cursor > token.chars.start && cursor < token.chars.end)
        .map_or(cursor, |token| if cursor >= was { token.chars.end } else { token.chars.start })
}

/// The targets of `text`, in order.
pub(crate) fn targets(text: &str) -> Vec<Target> {
    find(text).into_iter().map(|token| token.target).collect()
}

fn with_tokens_as(text: &str, spell: impl Fn(&Target) -> String) -> String {
    let mut out = String::with_capacity(text.len());
    let mut from = 0;
    for token in find(text) {
        out.push_str(&text[from..token.bytes.start]);
        out.push_str(&spell(&token.target));
        from = token.bytes.end;
    }
    out.push_str(&text[from..]);
    out.retain(|c| c != OPEN && c != CLOSE);
    out
}

/// The message to send: each token as its path, quoted only when it needs it, in its place.
pub(crate) fn sendable(text: &str) -> String {
    let text = text.strip_suffix(' ').filter(|rest| rest.ends_with(CLOSE)).unwrap_or(text);
    with_tokens_as(text, |target| match target {
        Target::File(path) => shell_words::quote(&path.to_string_lossy()).into_owned(),
        Target::Pasted(number) => format!("[Image #{number}]"),
    })
}

/// The words of `text` with every token left out, for what only reads words (a worktree name).
pub(crate) fn plain(text: &str) -> String {
    with_tokens_as(text, |_| String::new())
}

fn path_at(text: &str) -> Option<(Range<usize>, PathBuf)> {
    if let Some(rest) = text.strip_prefix("'/") {
        let end = rest.find('\'')?;
        let path = PathBuf::from(format!("/{}", &rest[..end]));
        return Some((0..end + 3, path));
    }
    if !text.starts_with('/') {
        return None;
    }
    let word = text.split(char::is_whitespace).next().unwrap_or_default();
    let word = word.trim_end_matches(|c: char| AFTER_WORD.contains(c));
    (word.len() > 1).then(|| (0..word.len(), PathBuf::from(word)))
}

/// A sent message turned back into box text: each word that is the path of something on the disk
/// becomes the token it was, so a recalled, held or sent message shows its files where they were.
pub(crate) fn from_sent(sent: &str, exists: &dyn Fn(&Path) -> bool) -> String {
    let mut out = String::with_capacity(sent.len());
    let mut at = 0;
    let mut word_start = true;
    while at < sent.len() {
        let rest = &sent[at..];
        let c = rest.chars().next().unwrap_or(' ');
        if word_start
            && let Some((span, path)) = path_at(rest)
            && exists(&path)
        {
            out.push_str(&encode(&Target::File(path)));
            at += span.end;
            word_start = false;
            continue;
        }
        out.push(c);
        at += c.len_utf8();
        word_start = c.is_whitespace();
    }
    out
}

/// [`from_sent`] against the real disk.
pub(crate) fn from_sent_on_disk(sent: &str) -> String {
    from_sent(sent, &|path| path.exists())
}

/// `text` with a token for every picture path left out, for matching a sent message to the line the
/// agent wrote with `[Image #N]` where the picture was.
pub(crate) fn without_picture_paths(sent: &str) -> String {
    let marked = from_sent(sent, &|path| tidewatch_core::image_paste::is_image_file(path));
    with_tokens_as(&marked, |_| String::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str) -> Target {
        Target::File(PathBuf::from(path))
    }

    fn token(path: &str) -> String {
        encode(&file(path))
    }

    #[test]
    fn a_token_survives_find_with_its_target_and_both_kinds_of_position() {
        let text = format!("é {} ok", token("/a b/Zürich_1.png"));
        let found = find(&text);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].target, file("/a b/Zürich_1.png"));
        assert_eq!(found[0].chars, 2..found[0].chars.end);
        assert_eq!(&text[found[0].bytes.clone()], token("/a b/Zürich_1.png"));
        assert_eq!(text[found[0].bytes.end..].to_owned(), " ok");
    }

    #[test]
    fn a_token_spells_only_hex_digits_so_no_markdown_or_wrap_rule_can_split_it() {
        let body = token("/Users/me/My *File*_[1] (a)-b.png");
        let inner = &body[OPEN.len_utf8()..body.len() - CLOSE.len_utf8()];
        assert_eq!(inner.chars().filter(|c| !c.is_ascii_hexdigit()).count(), 0);
        assert_eq!(find(&body).len(), 1);
    }

    #[test]
    fn a_pasted_picture_is_a_token_too() {
        let text = format!("see {}", encode(&Target::Pasted(2)));
        assert_eq!(targets(&text), vec![Target::Pasted(2)]);
        assert_eq!(sendable(&text), "see [Image #2]");
    }

    #[test]
    fn stray_and_broken_markers_are_not_tokens_and_never_sent() {
        let cases = [
            format!("a{OPEN}b"),
            format!("a{CLOSE}b"),
            format!("{OPEN}/x y{CLOSE}"),
            format!("{OPEN}{CLOSE}"),
            format!("{OPEN}relative/path{CLOSE}"),
            format!("{OPEN}/ok{OPEN}/ok2"),
        ];
        for text in cases {
            assert_eq!(find(&text), Vec::new(), "{text:?}");
            assert!(!sendable(&text).contains([OPEN, CLOSE]), "{text:?}");
        }
    }

    #[test]
    fn insert_mid_sentence_puts_the_token_between_the_words_around_the_cursor() {
        let mut text = "I am writing  and then".to_owned();
        let cursor = "I am writing ".chars().count();
        let after = insert(&mut text, cursor, &file("/s/shot-1.png"));
        assert_eq!(sendable(&text), "I am writing /s/shot-1.png and then");
        assert_eq!(text[..byte_of(&text, after)].chars().last(), Some(CLOSE));
    }

    #[test]
    fn insert_adds_a_space_only_where_it_would_touch_a_word() {
        let cases = [
            ("ab|cd", "ab /f cd"),
            ("ab |cd", "ab /f cd"),
            ("ab| cd", "ab /f cd"),
            ("ab |", "ab /f"),
            ("|", "/f"),
            ("(|)", "(/f)"),
            ("see|, ok", "see /f, ok"),
            ("a\n|b", "a\n/f b"),
        ];
        for (marked, want) in cases {
            let cursor = marked.find('|').map(|byte| marked[..byte].chars().count()).unwrap();
            let mut text = marked.replace('|', "");
            insert(&mut text, cursor, &file("/f"));
            assert_eq!(sendable(&text), want, "{marked:?}");
        }
    }

    #[test]
    fn a_draft_saved_with_a_file_list_gets_its_files_after_the_words() {
        let mut text = "see".to_owned();
        append(&mut text, [file("/a"), file("/b c")]);
        assert_eq!(sendable(&text), "see /a '/b c'");
    }

    #[test]
    fn two_inserts_in_a_row_stay_in_order_with_a_space_between() {
        let mut text = "x ".to_owned();
        let cursor = insert(&mut text, 2, &file("/a"));
        insert(&mut text, cursor, &file("/b"));
        assert_eq!(sendable(&text), "x /a /b");
    }

    #[test]
    fn backspace_after_a_token_and_delete_before_it_take_it_whole() {
        let mut text = format!("a {} b", token("/f"));
        let end = find(&text)[0].chars.end;
        assert_eq!(remove_before(&mut text, end), Some(2));
        assert_eq!(text, "a  b");
        let mut text = format!("a {} b", token("/f"));
        assert_eq!(remove_after(&mut text, 2), Some(2));
        assert_eq!(text, "a  b");
        let mut text = format!("a {} b", token("/f"));
        assert_eq!(remove_before(&mut text, 1), None);
        assert_eq!(remove_after(&mut text, 1), None);
    }

    #[test]
    fn a_cursor_inside_a_token_goes_to_the_edge_it_was_heading_for() {
        let text = format!("a {} b", token("/ff"));
        let (start, end) = (2, find(&text)[0].chars.end);
        assert_eq!(snap(&text, start + 3, start), end);
        assert_eq!(snap(&text, start + 3, end), start);
        assert_eq!(snap(&text, start, 0), start);
        assert_eq!(snap(&text, end, 0), end);
        assert_eq!(snap(&text, 1, 0), 1);
    }

    #[test]
    fn sending_quotes_a_path_only_when_it_needs_it() {
        let text = format!("{} and {}", token("/tmp/a.png"), token("/tmp/My Report.pdf"));
        assert_eq!(sendable(&text), "/tmp/a.png and '/tmp/My Report.pdf'");
    }

    #[test]
    fn plain_leaves_the_tokens_out() {
        let text = format!("fix {} now", token("/tmp/a.png"));
        assert_eq!(plain(&text), "fix  now");
    }

    #[test]
    fn a_sent_message_gets_its_tokens_back_where_the_paths_were() {
        let here = |path: &Path| path.starts_with("/tmp");
        let cases = [
            (
                "numbers in /tmp/q3.pdf don't match '/tmp/My Shot 2.png', can you check?",
                format!(
                    "numbers in {} don't match {}, can you check?",
                    token("/tmp/q3.pdf"),
                    token("/tmp/My Shot 2.png")
                ),
            ),
            ("look at /etc/hosts and /usr/x", "look at /etc/hosts and /usr/x".to_owned()),
            ("/compact", "/compact".to_owned()),
            ("a/tmp/b", "a/tmp/b".to_owned()),
            ("see /tmp/x.", format!("see {}.", token("/tmp/x"))),
            ("/tmp/a\n/tmp/b", format!("{}\n{}", token("/tmp/a"), token("/tmp/b"))),
        ];
        for (sent, want) in cases {
            assert_eq!(from_sent(sent, &here), want, "{sent:?}");
        }
    }

    #[test]
    fn from_sent_then_sendable_is_the_message_that_was_sent() {
        let here = |_: &Path| true;
        for sent in ["a /tmp/x.png b", "'/tmp/a b.pdf'", "/tmp/x, /tmp/y"] {
            assert_eq!(sendable(&from_sent(sent, &here)), sent);
        }
    }

    #[test]
    fn picture_paths_are_left_out_for_matching_the_line_the_agent_wrote() {
        assert_eq!(without_picture_paths("see /tmp/a.png and /tmp/b.pdf"), "see  and /tmp/b.pdf");
    }
}
