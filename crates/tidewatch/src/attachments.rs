//! Files that are not pictures, attached to a message: how each one is kept for the agent (a copy in
//! the tab's `pastes` folder, or its own path), and the pure text rules of its chip. A picture file
//! keeps its own path, as before. The copy runs on a `Jobs` thread, never in a frame.

use std::path::{Path, PathBuf};

use tidewatch_core::image_paste::is_image_file;

use crate::pictures::size_text;

pub(crate) const COPY_LIMIT: u64 = 50_000_000;
const EXT_MAX: usize = 5;
const ELLIPSIS: char = '\u{2026}';

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Image,
    Folder,
    File,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Route {
    Copy,
    Reference,
    TooBig,
}

pub(crate) fn kind_of(path: &Path, is_dir: bool) -> Kind {
    if is_dir {
        Kind::Folder
    } else if is_image_file(path) {
        Kind::Image
    } else {
        Kind::File
    }
}

pub(crate) fn route(kind: Kind, bytes: u64) -> Route {
    match kind {
        Kind::Image | Kind::Folder => Route::Reference,
        Kind::File if bytes <= COPY_LIMIT => Route::Copy,
        Kind::File => Route::TooBig,
    }
}

/// The capitals shown on a file chip: `CSV`, `PDF`. `FILE` when the name has no extension or a long one.
pub(crate) fn ext_label(path: &Path) -> String {
    path.extension()
        .and_then(|ext| ext.to_str())
        .filter(|ext| !ext.is_empty() && ext.chars().count() <= EXT_MAX)
        .map_or_else(|| "FILE".to_owned(), str::to_uppercase)
}

/// `name` cut in the middle, the start and the end kept, until `fits` says it is short enough.
pub(crate) fn fit_middle(name: &str, fits: impl Fn(&str) -> bool) -> String {
    if fits(name) {
        return name.to_owned();
    }
    let chars: Vec<char> = name.chars().collect();
    for keep in (1..chars.len()).rev() {
        let tail = keep / 2;
        let head = keep - tail;
        let cut: String = chars[..head]
            .iter()
            .chain(std::iter::once(&ELLIPSIS))
            .chain(&chars[chars.len() - tail..])
            .collect();
        if fits(&cut) {
            return cut;
        }
    }
    ELLIPSIS.to_string()
}

/// The name a copy of `name` gets in a folder where `taken` says which names are used: `name`, else
/// `stem-2.ext`, `stem-3.ext`.
pub(crate) fn copy_name(name: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(name) {
        return name.to_owned();
    }
    let path = Path::new(name);
    let stem = path.file_stem().and_then(|stem| stem.to_str()).unwrap_or(name);
    let ext = path.extension().and_then(|ext| ext.to_str());
    (2..10_000)
        .map(|n| match ext {
            Some(ext) => format!("{stem}-{n}.{ext}"),
            None => format!("{stem}-{n}"),
        })
        .find(|candidate| !taken(candidate))
        .unwrap_or_else(|| name.to_owned())
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Staged {
    pub(crate) paths: Vec<PathBuf>,
    pub(crate) notes: Vec<String>,
}

fn name_of(path: &Path) -> String {
    path.file_name().and_then(|name| name.to_str()).unwrap_or("file").to_owned()
}

fn copy_into(path: &Path, dir: &Path) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let target = dir.join(copy_name(&name_of(path), |name| dir.join(name).exists()));
    std::fs::copy(path, &target)?;
    Ok(target)
}

/// What each of `paths` becomes as a chip: a file up to `COPY_LIMIT` is copied into `dir` and
/// the copy is the chip's path; a picture, a folder, a bigger file, a missing one or one that fails
/// to copy keeps its own path. A bigger file or a failed copy adds a note for a toast.
pub(crate) fn stage(paths: &[PathBuf], dir: &Path) -> Staged {
    let mut staged = Staged::default();
    for path in paths {
        let meta = std::fs::metadata(path).ok();
        let kind = kind_of(path, meta.as_ref().is_some_and(std::fs::Metadata::is_dir));
        let bytes = meta.as_ref().map_or(0, std::fs::Metadata::len);
        let name = name_of(path);
        match (meta.is_some(), route(kind, bytes)) {
            (true, Route::Copy) => match copy_into(path, dir) {
                Ok(copy) => staged.paths.push(copy),
                Err(error) => {
                    staged.paths.push(path.clone());
                    staged.notes.push(format!(
                        "Could not copy {name} ({error}), attached by its original path"
                    ));
                }
            },
            (true, Route::TooBig) => {
                staged.paths.push(path.clone());
                staged.notes.push(format!(
                    "{name} is {}, over the {} copy limit, attached by its original path",
                    size_text(bytes),
                    size_text(COPY_LIMIT)
                ));
            }
            _ => staged.paths.push(path.clone()),
        }
    }
    staged
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_tells_pictures_folders_and_other_files_apart() {
        let cases = [
            ("/a/shot.PNG", false, Kind::Image),
            ("/a/photo.jpeg", false, Kind::Image),
            ("/a/data.csv", false, Kind::File),
            ("/a/report.pdf", false, Kind::File),
            ("/a/Makefile", false, Kind::File),
            ("/a/archive.tar.gz", false, Kind::File),
            ("/a/pics", true, Kind::Folder),
            ("/a/odd.png", true, Kind::Folder),
        ];
        for (path, dir, want) in cases {
            assert_eq!(kind_of(Path::new(path), dir), want, "{path} dir {dir}");
        }
    }

    #[test]
    fn a_file_is_copied_up_to_the_limit_and_everything_else_is_referenced() {
        let cases = [
            (Kind::File, 0, Route::Copy),
            (Kind::File, 12_000, Route::Copy),
            (Kind::File, COPY_LIMIT, Route::Copy),
            (Kind::File, COPY_LIMIT + 1, Route::TooBig),
            (Kind::Image, 100, Route::Reference),
            (Kind::Image, COPY_LIMIT * 4, Route::Reference),
            (Kind::Folder, 4096, Route::Reference),
        ];
        for (kind, bytes, want) in cases {
            assert_eq!(route(kind, bytes), want, "{kind:?} {bytes}");
        }
    }

    #[test]
    fn the_chip_shows_the_extension_in_capitals() {
        let cases = [
            ("/a/data.csv", "CSV"),
            ("/a/Report.Pdf", "PDF"),
            ("/a/bundle.zip", "ZIP"),
            ("/a/log.jsonl", "JSONL"),
            ("/a/Makefile", "FILE"),
            ("/a/notes.markdown", "FILE"),
            ("/a/.env", "FILE"),
        ];
        for (path, want) in cases {
            assert_eq!(ext_label(Path::new(path)), want, "{path}");
        }
    }

    #[test]
    fn size_labels_round_to_the_unit_shown() {
        let cases = [
            (0, "0 B"),
            (999, "999 B"),
            (1_000, "1 KB"),
            (12_345, "12 KB"),
            (999_499, "999 KB"),
            (1_500_000, "1.5 MB"),
            (COPY_LIMIT, "50.0 MB"),
        ];
        for (bytes, want) in cases {
            assert_eq!(size_text(bytes), want, "{bytes}");
        }
    }

    #[test]
    fn a_long_name_is_cut_in_the_middle_and_keeps_its_end() {
        let fits = |max: usize| move |text: &str| text.chars().count() <= max;
        let cases = [
            ("data.csv", 20, "data.csv"),
            ("quarterly-revenue-by-region-2026.csv", 16, "quarterl\u{2026}026.csv"),
            ("quarterly-revenue-by-region-2026.csv", 9, "quar\u{2026}.csv"),
            ("abcdef", 3, "a\u{2026}f"),
            ("abcdef", 2, "a\u{2026}"),
            ("abcdef", 1, "\u{2026}"),
        ];
        for (name, max, want) in cases {
            assert_eq!(fit_middle(name, fits(max)), want, "{name} in {max}");
        }
    }

    #[test]
    fn a_copy_never_takes_a_name_that_is_used() {
        assert_eq!(copy_name("a.csv", |_| false), "a.csv");
        assert_eq!(copy_name("a.csv", |n| n == "a.csv"), "a-2.csv");
        assert_eq!(copy_name("a.csv", |n| n == "a.csv" || n == "a-2.csv"), "a-3.csv");
        assert_eq!(copy_name("Makefile", |n| n == "Makefile"), "Makefile-2");
    }

    fn file(dir: &Path, name: &str, bytes: usize) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, vec![b'x'; bytes]).unwrap();
        path
    }

    #[test]
    fn staging_copies_a_small_file_and_leaves_pictures_and_folders_where_they_are() {
        let from = tempfile::tempdir().unwrap();
        let to = tempfile::tempdir().unwrap();
        let pastes = to.path().join("pastes");
        let csv = file(from.path(), "sales.csv", 40);
        let png = file(from.path(), "shot.png", 10);
        let folder = from.path().join("exports");
        std::fs::create_dir(&folder).unwrap();
        let staged = stage(&[csv.clone(), png.clone(), folder.clone()], &pastes);
        assert_eq!(staged.paths, [pastes.join("sales.csv"), png, folder]);
        assert_eq!(staged.notes, Vec::<String>::new());
        assert_eq!(std::fs::read(pastes.join("sales.csv")).unwrap(), vec![b'x'; 40]);
        assert!(csv.is_file(), "the original stays");
        let again = stage(&[csv], &pastes);
        assert_eq!(again.paths, [pastes.join("sales-2.csv")]);
    }

    #[test]
    fn a_missing_file_keeps_its_path_so_its_chip_says_so() {
        let to = tempfile::tempdir().unwrap();
        let gone = PathBuf::from("/no/such/dir/gone.csv");
        let staged = stage(std::slice::from_ref(&gone), to.path());
        assert_eq!(staged, Staged { paths: vec![gone], notes: Vec::new() });
        assert_eq!(std::fs::read_dir(to.path()).unwrap().count(), 0);
    }

    #[test]
    fn a_failed_copy_keeps_the_original_path_and_says_so() {
        let from = tempfile::tempdir().unwrap();
        let csv = file(from.path(), "sales.csv", 5);
        let blocked = file(from.path(), "not-a-folder", 1);
        let staged = stage(std::slice::from_ref(&csv), &blocked);
        assert_eq!(staged.paths, [csv]);
        assert_eq!(staged.notes.len(), 1);
        assert!(staged.notes[0].starts_with("Could not copy sales.csv"), "{:?}", staged.notes);
    }
}
