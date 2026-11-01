//! What a tab's agent has spent in tokens, read from the usage fields its own transcript carries
//! (pure, fed line by line by the transcript parser).
//!
//! Claude Code 2.1.316 writes a message with several content blocks as one line per block, each
//! repeating the message id and the same usage (identical on all 1433 lines of a real transcript),
//! so a message counts once. Codex 0.162.1 writes a running total whose input already includes the
//! cached share, so each reading is subtracted from the one before. Gemini and Copilot fixtures
//! hold no token counts, so those sources count nothing.

use std::collections::{HashSet, VecDeque};

use serde_json::Value;

use crate::sent::Record;
use crate::text::squash;
use crate::transcript::Source;

const TURNS_KEPT: usize = 400;
const FINGERPRINT_CHARS: usize = 2000;
const SENT_SLACK_MS: i64 = 5_000;
const SENT_WINDOW_MS: i64 = 600_000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    pub input: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub output: u64,
}

impl Usage {
    #[must_use]
    pub fn total(self) -> u64 {
        self.prompt() + self.output
    }

    /// Everything the model read: new input, cache hits and the part written to the cache.
    #[must_use]
    pub fn prompt(self) -> u64 {
        self.input + self.cache_read + self.cache_write
    }

    /// Whole percent of what the model read that came from the cache; none before it read anything.
    #[must_use]
    pub fn cached_percent(self) -> Option<u8> {
        let prompt = self.prompt();
        (prompt > 0).then(|| {
            u8::try_from(
                (u128::from(self.cache_read) * 100 + u128::from(prompt) / 2) / u128::from(prompt),
            )
            .unwrap_or(100)
        })
    }

    #[must_use]
    pub fn is_zero(self) -> bool {
        self == Self::default()
    }

    fn add(&mut self, other: Self) {
        self.input += other.input;
        self.cache_read += other.cache_read;
        self.cache_write += other.cache_write;
        self.output += other.output;
    }

    fn since(self, earlier: Self) -> Self {
        Self {
            input: self.input.saturating_sub(earlier.input),
            cache_read: self.cache_read.saturating_sub(earlier.cache_read),
            cache_write: self.cache_write.saturating_sub(earlier.cache_write),
            output: self.output.saturating_sub(earlier.output),
        }
    }
}

/// What one user message started: the turn's spend until the next user message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Turn {
    pub started_ms: Option<i64>,
    pub fingerprint: String,
    pub usage: Usage,
}

/// A transcript's token spend as of the lines read so far.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tally {
    pub total: Usage,
    /// Model calls counted (a Claude message once however many lines it was written as).
    pub calls: u32,
    /// User messages seen, including ones no longer in `turns`.
    pub turn_count: u32,
    /// The newest turns, oldest first.
    pub turns: Vec<Turn>,
}

impl Tally {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.calls == 0
    }
}

#[derive(Debug, Default)]
pub(crate) struct Counter {
    seen: HashSet<String>,
    total: Usage,
    calls: u32,
    turn_count: u32,
    turns: VecDeque<Turn>,
    codex_last: Usage,
}

impl Counter {
    /// Changes whenever a call or a turn is counted, so a caller can tell cheaply that `tally` moved.
    pub(crate) fn stamp(&self) -> (u32, u32) {
        (self.calls, self.turn_count)
    }

    pub(crate) fn tally(&self) -> Tally {
        Tally {
            total: self.total,
            calls: self.calls,
            turn_count: self.turn_count,
            turns: self.turns.iter().cloned().collect(),
        }
    }

    pub(crate) fn user(&mut self, text: &str, started_ms: Option<i64>) {
        self.turn_count += 1;
        if self.turns.len() == TURNS_KEPT {
            self.turns.pop_front();
        }
        self.turns.push_back(Turn {
            started_ms,
            fingerprint: fingerprint(text),
            usage: Usage::default(),
        });
    }

    pub(crate) fn line(&mut self, source: Source, line: &Value) {
        match source {
            Source::Claude => self.claude(line),
            Source::Codex => self.codex(line),
            Source::Gemini | Source::Copilot => {}
        }
    }

    fn spend(&mut self, usage: Usage) {
        self.total.add(usage);
        self.calls += 1;
        if let Some(turn) = self.turns.back_mut() {
            turn.usage.add(usage);
        }
    }

    fn claude(&mut self, line: &Value) {
        let message = &line["message"];
        if line["type"].as_str() != Some("assistant")
            || message["model"].as_str() == Some("<synthetic>")
        {
            return;
        }
        let raw = &message["usage"];
        if !raw.is_object() {
            return;
        }
        if let Some(id) = message["id"].as_str()
            && !self.seen.insert(id.to_owned())
        {
            return;
        }
        let count = |key: &str| raw[key].as_u64().unwrap_or(0);
        self.spend(Usage {
            input: count("input_tokens"),
            cache_read: count("cache_read_input_tokens"),
            cache_write: count("cache_creation_input_tokens"),
            output: count("output_tokens"),
        });
    }

    fn codex(&mut self, line: &Value) {
        let payload = &line["payload"];
        if line["type"].as_str() != Some("event_msg")
            || payload["type"].as_str() != Some("token_count")
        {
            return;
        }
        let raw = &payload["info"]["total_token_usage"];
        if !raw.is_object() {
            return;
        }
        let count = |key: &str| raw[key].as_u64().unwrap_or(0);
        let cached = count("cached_input_tokens");
        let written = count("cache_write_input_tokens");
        let running = Usage {
            input: count("input_tokens").saturating_sub(cached + written),
            cache_read: cached,
            cache_write: written,
            output: count("output_tokens"),
        };
        let spent = running.since(self.codex_last);
        self.codex_last = running;
        if !spent.is_zero() {
            self.spend(spent);
        }
    }
}

/// A message squashed to single spaces and cut, so a turn and a record of what Tidewatch sent can
/// be compared without keeping a pasted wall of text.
#[must_use]
pub fn fingerprint(text: &str) -> String {
    squash(text).chars().take(FINGERPRINT_CHARS).collect()
}

/// What the messages Tidewatch typed into the agent cost: the turns they started.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Share {
    pub messages: u32,
    pub usage: Usage,
}

/// The turns that a record of `sent` started. A record that a hook told (no transcript line) or
/// that is not in the transcript yet counts nothing, and a turn answers at most one record.
#[must_use]
pub fn share_of(sent: &[Record], turns: &[Turn]) -> Share {
    let mut taken = vec![false; turns.len()];
    let mut share = Share::default();
    for record in sent.iter().filter(|record| !record.timed()) {
        let wanted = fingerprint(&record.text);
        let found = turns
            .iter()
            .enumerate()
            .filter(|(index, turn)| !taken[*index] && turn.fingerprint == wanted)
            .filter(|(_, turn)| {
                turn.started_ms.is_none_or(|at| {
                    at >= record.at_ms - SENT_SLACK_MS && at <= record.at_ms + SENT_WINDOW_MS
                })
            })
            .min_by_key(|(_, turn)| {
                turn.started_ms.map_or(SENT_WINDOW_MS, |at| (at - record.at_ms).abs())
            });
        if let Some((index, turn)) = found {
            taken[index] = true;
            share.messages += 1;
            share.usage.add(turn.usage);
        }
    }
    share
}

/// `1.2M`, `14k`, `980`: a count for a line of a few words.
#[must_use]
pub fn compact(count: u64) -> String {
    const STEPS: [(u64, &str); 2] = [(1_000_000, "M"), (1_000, "k")];
    for (unit, suffix) in STEPS {
        if count >= unit - unit / 2000 {
            let tenths = (count * 10 + unit / 2) / unit;
            let whole = tenths / 10;
            return if whole >= 10 || tenths.is_multiple_of(10) {
                format!("{}{suffix}", (count + unit / 2) / unit)
            } else {
                format!("{whole}.{}{suffix}", tenths % 10)
            };
        }
    }
    count.to_string()
}

/// `1.2M tokens`.
#[must_use]
pub fn short(tally: &Tally) -> Option<String> {
    (!tally.is_empty()).then(|| format!("{} tokens", compact(tally.total.total())))
}

/// `1.2M tokens · 94% cached · 18 turns`.
#[must_use]
pub fn summary(tally: &Tally) -> Option<String> {
    let mut parts = vec![short(tally)?];
    if let Some(percent) = tally.total.cached_percent() {
        parts.push(format!("{percent}% cached"));
    }
    if tally.turn_count > 0 {
        let word = if tally.turn_count == 1 { "turn" } else { "turns" };
        parts.push(format!("{} {word}", tally.turn_count));
    }
    Some(parts.join(" \u{b7} "))
}

/// `Tidewatch cost this tab 14k tokens (4 messages)`; none when it sent nothing the transcript holds.
#[must_use]
pub fn share_line(share: Share) -> Option<String> {
    (share.messages > 0).then(|| {
        let word = if share.messages == 1 { "message" } else { "messages" };
        format!(
            "Tidewatch cost this tab {} tokens ({} {word})",
            compact(share.usage.total()),
            share.messages
        )
    })
}

#[cfg(test)]
mod tests;
