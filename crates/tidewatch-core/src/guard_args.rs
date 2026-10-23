//! Finds the text and the facts the guards care about in a `git commit` or `gh` command line. Pure:
//! reading a `--body-file` or standard input is left to the caller.
//!
//! Flags were checked against git 2.55.0 and gh 2.98.0 (`gh help reference`).

use std::path::PathBuf;

/// Where a piece of text comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    Inline(String),
    File(PathBuf),
    Stdin,
}

impl Source {
    fn from_file_arg(path: &str) -> Self {
        if path == "-" { Self::Stdin } else { Self::File(PathBuf::from(path)) }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Message,
    Title,
    Body,
}

impl Part {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Message => "the commit message",
            Self::Title => "the title",
            Self::Body => "the body",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextArg {
    pub part: Part,
    pub source: Source,
}

/// A `git commit` whose message is on the command line.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GitCommit {
    /// Where `git -C` points, so a relative `-F` file is found.
    pub dir: Option<PathBuf>,
    pub texts: Vec<TextArg>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GhAction {
    PrCreate,
    PrEdit,
    PrComment,
    PrReview,
    IssueCreate,
    IssueComment,
    IssueEdit,
    RunRerun,
}

impl GhAction {
    fn parse(group: &str, verb: &str) -> Option<Self> {
        Some(match (group, verb) {
            ("pr", "create") => Self::PrCreate,
            ("pr", "edit") => Self::PrEdit,
            ("pr", "comment") => Self::PrComment,
            ("pr", "review") => Self::PrReview,
            ("issue", "create") => Self::IssueCreate,
            ("issue", "comment") => Self::IssueComment,
            ("issue", "edit") => Self::IssueEdit,
            ("run", "rerun") => Self::RunRerun,
            _ => return None,
        })
    }

    /// The short flags that take a value, so a cluster like `-dt title` is read right.
    fn value_shorts(self) -> &'static str {
        match self {
            Self::PrCreate => "RaBbFHlmprTt",
            Self::IssueCreate => "RabFlmpTt",
            Self::PrEdit | Self::IssueEdit => "RBbFmt",
            Self::PrComment | Self::IssueComment | Self::PrReview => "RbF",
            Self::RunRerun => "Rj",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GhCall {
    pub action: GhAction,
    pub texts: Vec<TextArg>,
    /// `-R owner/name` or `--repo owner/name`.
    pub repo: Option<String>,
    pub draft: bool,
    /// `--failed` or `--job`, either of which reruns less than the whole run.
    pub narrow_rerun: bool,
}

fn text(part: Part, source: Source) -> TextArg {
    TextArg { part, source }
}

/// The value of a long flag: after `=`, or the next argument.
fn long_value<'a>(
    inline: Option<&'a str>,
    rest: &mut std::slice::Iter<'a, String>,
) -> Option<&'a str> {
    inline.or_else(|| rest.next().map(String::as_str))
}

/// `git [global options] commit ...`, or `None` for any other command.
#[must_use]
pub fn parse_git(args: &[String]) -> Option<GitCommit> {
    let mut dir: Option<PathBuf> = None;
    let mut rest = args.iter();
    loop {
        let arg = rest.next()?;
        match arg.as_str() {
            "-C" => {
                let next = PathBuf::from(rest.next()?);
                dir = Some(dir.map_or(next.clone(), |base| base.join(&next)));
            }
            "-c" | "--git-dir" | "--work-tree" | "--namespace" | "--super-prefix"
            | "--config-env" | "--attr-source" => {
                rest.next()?;
            }
            "commit" => break,
            other if other.starts_with('-') => {}
            _ => return None,
        }
    }
    let mut commit = GitCommit { dir, texts: Vec::new() };
    while let Some(arg) = rest.next() {
        if arg == "--" {
            break;
        }
        if let Some(long) = arg.strip_prefix("--") {
            let (name, inline) = long.split_once('=').map_or((long, None), |(n, v)| (n, Some(v)));
            let source = match name {
                "message" | "trailer" => {
                    long_value(inline, &mut rest).map(|v| Source::Inline(v.to_owned()))
                }
                "file" => long_value(inline, &mut rest).map(Source::from_file_arg),
                _ => None,
            };
            commit.texts.extend(source.map(|source| text(Part::Message, source)));
        } else if let Some(cluster) = arg.strip_prefix('-').filter(|c| !c.is_empty()) {
            for (at, flag) in cluster.char_indices() {
                let attached = &cluster[at + flag.len_utf8()..];
                match flag {
                    'm' | 'F' | 'C' | 'c' | 't' => {
                        let value = if attached.is_empty() {
                            rest.next().map(String::as_str)
                        } else {
                            Some(attached)
                        };
                        match (flag, value) {
                            ('m', Some(v)) => {
                                commit
                                    .texts
                                    .push(text(Part::Message, Source::Inline(v.to_owned())));
                            }
                            ('F', Some(v)) => {
                                commit.texts.push(text(Part::Message, Source::from_file_arg(v)));
                            }
                            _ => {}
                        }
                        break;
                    }
                    'S' | 'u' => break,
                    _ => {}
                }
            }
        }
    }
    Some(commit)
}

/// `gh pr create|edit|comment|review`, `gh issue create|comment|edit` or `gh run rerun`, or `None`
/// for any other `gh` command.
#[must_use]
pub fn parse_gh(args: &[String]) -> Option<GhCall> {
    let action = GhAction::parse(args.first()?, args.get(1)?)?;
    let shorts = action.value_shorts();
    let mut call =
        GhCall { action, texts: Vec::new(), repo: None, draft: false, narrow_rerun: false };
    let mut rest = args[2..].iter();
    while let Some(arg) = rest.next() {
        if arg == "--" {
            break;
        }
        if let Some(long) = arg.strip_prefix("--") {
            let (name, inline) = long.split_once('=').map_or((long, None), |(n, v)| (n, Some(v)));
            match name {
                "title" => push(&mut call, Part::Title, long_value(inline, &mut rest), false),
                "body" => push(&mut call, Part::Body, long_value(inline, &mut rest), false),
                "body-file" => push(&mut call, Part::Body, long_value(inline, &mut rest), true),
                "repo" => call.repo = long_value(inline, &mut rest).map(str::to_owned),
                "draft" => call.draft = inline.is_none_or(|v| v != "false"),
                "failed" => call.narrow_rerun = inline.is_none_or(|v| v != "false"),
                "job" => call.narrow_rerun = true,
                _ => {}
            }
        } else if let Some(cluster) = arg.strip_prefix('-').filter(|c| !c.is_empty()) {
            for (at, flag) in cluster.char_indices() {
                if flag == 'd' && action == GhAction::PrCreate {
                    call.draft = true;
                }
                if !shorts.contains(flag) {
                    continue;
                }
                let tail = &cluster[at + flag.len_utf8()..];
                let value = if tail.is_empty() {
                    rest.next().map(String::as_str)
                } else {
                    Some(tail.trim_start_matches('='))
                };
                match flag {
                    't' => push(&mut call, Part::Title, value, false),
                    'b' => push(&mut call, Part::Body, value, false),
                    'F' => push(&mut call, Part::Body, value, true),
                    'R' => call.repo = value.map(str::to_owned),
                    'j' => call.narrow_rerun = true,
                    _ => {}
                }
                break;
            }
        }
    }
    Some(call)
}

fn push(call: &mut GhCall, part: Part, value: Option<&str>, file: bool) {
    let Some(value) = value else { return };
    let source = if file { Source::from_file_arg(value) } else { Source::Inline(value.to_owned()) };
    call.texts.push(text(part, source));
}

#[cfg(test)]
mod tests;
