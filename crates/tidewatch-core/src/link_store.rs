//! A tab's links on disk: `<tab state>/links.json` beside its notes, kept for the life of the tab.
//! The listener thread (an agent's report) and the app (links it found) both write the file, so
//! every write reads the file again under one lock and merges into what it holds.

use std::path::Path;
use std::sync::Mutex;

use crate::fsutil::{read_json, save_json};
use crate::link_board::{Link, LinkBoard};

static WRITING: Mutex<()> = Mutex::new(());

/// The saved board. A missing or unreadable file is `None`.
#[must_use]
pub fn load(file: &Path) -> Option<LinkBoard> {
    read_json(file)
}

pub fn save(file: &Path, board: &LinkBoard) -> std::io::Result<()> {
    save_json(file, board)
}

/// Merges `incoming` into the saved board and saves it when it changed. Returns the board as it
/// stands, so a caller never has to read the file again.
pub fn merge(file: &Path, incoming: &[Link]) -> std::io::Result<LinkBoard> {
    let _held = WRITING.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut board = load(file).unwrap_or_default();
    if board.absorb_all(incoming) {
        save(file, &board)?;
    }
    Ok(board)
}

#[cfg(test)]
mod tests;
