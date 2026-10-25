//! Which tabs run on their own: a YOLO agent and every Meeseeks. Their PR watch, done check, due items
//! and delivery act without waiting for a click, and their house style says to decide and proceed.
//! Only product design and architecture choices still go to the owner.

use crate::agents::Level;

#[must_use]
pub fn autonomous(level: Level, is_meeseeks: bool) -> bool {
    is_meeseeks || level == Level::Yolo
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yolo_and_every_meeseeks_run_on_their_own_and_nothing_else_does() {
        let cases = [
            (Level::Plan, false, false),
            (Level::Ask, false, false),
            (Level::Edits, false, false),
            (Level::Auto, false, false),
            (Level::Yolo, false, true),
            (Level::Plan, true, true),
            (Level::Ask, true, true),
            (Level::Edits, true, true),
            (Level::Auto, true, true),
            (Level::Yolo, true, true),
        ];
        for (level, is_meeseeks, want) in cases {
            assert_eq!(autonomous(level, is_meeseeks), want, "{level:?} meeseeks={is_meeseeks}");
        }
    }
}
