//! The session id Tidewatch gives an agent at launch (`--session-id`), so the Reader can find the
//! agent's transcript by name instead of guessing.

/// A random UUID in the form Claude, Gemini and Copilot accept for `--session-id`.
#[must_use]
pub fn new() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_uuids_and_differ() {
        let (a, b) = (new(), new());
        assert_ne!(a, b);
        assert_eq!(a.len(), 36);
        assert_eq!(a.matches('-').count(), 4);
        assert!(a.chars().all(|c| c == '-' || c.is_ascii_hexdigit()));
    }
}
