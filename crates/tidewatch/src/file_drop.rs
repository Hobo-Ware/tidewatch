//! Files dragged from Finder onto the window. The window hands them to egui in the raw input: the
//! hover lasts while the drag is over the window, the drop arrives for one frame.

use std::path::PathBuf;

use egui::Context;

pub(crate) fn hovering(ctx: &Context) -> bool {
    ctx.input(|input| !input.raw.hovered_files.is_empty())
}

/// The files dropped this frame. A drop without a path (a browser build) has nothing to type.
pub(crate) fn dropped(ctx: &Context) -> Vec<PathBuf> {
    ctx.input(|input| input.raw.dropped_files.iter().filter_map(|file| file.path.clone()).collect())
}
