//! One tab's numbered events in memory (pure). Trimming writes a checkpoint of the state first, so
//! replaying a trimmed log gives exactly the state the whole log gives.

use crate::panel_event::{Event, Fact, Source};
use crate::panel_state::PanelState;

pub const CAP_EVENTS: usize = 2_000;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Log {
    events: Vec<Event>,
    next_seq: u64,
}

impl Log {
    #[must_use]
    pub fn new() -> Self {
        Self { events: Vec::new(), next_seq: 1 }
    }

    #[must_use]
    pub fn from_events(events: impl IntoIterator<Item = Event>) -> Self {
        let mut log = Self::new();
        for event in events {
            if event.seq >= log.next_seq {
                log.next_seq = event.seq + 1;
                log.events.push(event);
            }
        }
        log
    }

    /// The returned event, with its texts clipped, is the one to fold and to write.
    pub fn append(&mut self, source: Source, fact: Fact, at_ms: i64) -> Event {
        let event = Event { seq: self.next_seq, at_ms, source, fact: fact.clipped() };
        self.next_seq += 1;
        self.events.push(event.clone());
        event
    }

    #[must_use]
    pub fn events(&self) -> &[Event] {
        &self.events
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.events.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    #[must_use]
    pub fn next_seq(&self) -> u64 {
        self.next_seq
    }

    #[must_use]
    pub fn events_since(&self, seq: u64) -> &[Event] {
        let from = self.events.partition_point(|event| event.seq <= seq);
        &self.events[from..]
    }

    #[must_use]
    pub fn replay(&self) -> PanelState {
        PanelState::replay(&self.events)
    }

    /// Replaces all but the newest events by a checkpoint of them.
    pub fn trim(&mut self, keep: usize) {
        let Some(cut) = self.events.len().checked_sub(keep).filter(|cut| *cut > 0) else { return };
        let state = PanelState::replay(&self.events[..cut]);
        let checkpoint = Event {
            seq: self.events[cut - 1].seq,
            at_ms: state.clock_ms(),
            source: Source::App,
            fact: Fact::Checkpoint(Box::new(state)),
        };
        self.events.splice(..cut, [checkpoint]);
    }
}

#[cfg(test)]
mod tests;
