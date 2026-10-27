//! The seeded efforts of older versions moved with the model (see `profiles::default_effort`). Each
//! saved file gets them rewritten once; `effort-defaults.migrated` beside the files lists the ones
//! done, one per line, so an effort set by hand afterwards is never touched again.

use std::path::Path;

use crate::fsutil::write_atomic;
use crate::paths::TidewatchPaths;

pub(crate) const PROFILES: &str = "profiles";

fn done(marker: &Path) -> Vec<String> {
    std::fs::read_to_string(marker)
        .map(|text| {
            text.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_owned).collect()
        })
        .unwrap_or_default()
}

pub(crate) fn is_done(paths: &TidewatchPaths, name: &str) -> bool {
    done(&paths.effort_migrated_file()).iter().any(|done| done == name)
}

pub(crate) fn mark(paths: &TidewatchPaths, name: &str) {
    let marker = paths.effort_migrated_file();
    let mut names = done(&marker);
    if names.iter().any(|done| done == name) {
        return;
    }
    names.push(name.to_owned());
    if let Err(e) = write_atomic(&marker, &(names.join("\n") + "\n")) {
        eprintln!("tidewatch: cannot write {} ({e})", marker.display());
    }
}
