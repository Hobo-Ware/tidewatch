//! Test fixtures that a test thread forking at the wrong moment cannot break.

use std::path::Path;
use std::process::Command;

/// Linux refuses to exec a file that any process holds open for writing (ETXTBSY), and a test
/// thread that forks while this process writes the file inherits that handle, so `cp` writes it.
pub(crate) fn write_executable_from_a_child(path: &Path, text: &str) {
    use std::os::unix::fs::PermissionsExt as _;
    let source = path.with_extension("src");
    std::fs::write(&source, text).unwrap();
    let copied = Command::new("cp").arg(&source).arg(path).status().unwrap();
    assert!(copied.success(), "cp {} failed", path.display());
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// A forked test thread holds a copy of the listener until its exec, so this waits until nothing
/// answers.
pub(crate) fn leave_stale_socket(path: &Path) {
    use std::os::unix::net::{UnixListener, UnixStream};
    drop(UnixListener::bind(path).unwrap());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while UnixStream::connect(path).is_ok() {
        assert!(std::time::Instant::now() < deadline, "the dropped listener still answers");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}
