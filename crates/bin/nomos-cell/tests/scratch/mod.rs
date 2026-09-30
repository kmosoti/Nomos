#![allow(dead_code)]

//! Scratch directories for tests, removed when the test that made them
//! ends, whether it passed or failed. The test harness runs every test on a
//! thread of its own, so each path is registered with that thread and
//! removed by the thread's destructor; nothing is left in the system's
//! temporary directory, however many times the suite runs.

use std::cell::RefCell;
use std::path::PathBuf;

struct Made(Vec<PathBuf>);

impl Drop for Made {
    fn drop(&mut self) {
        for path in &self.0 {
            if std::fs::remove_dir_all(path).is_err() {
                let _ = std::fs::remove_file(path);
            }
        }
    }
}

thread_local! {
    static MADE: RefCell<Made> = const { RefCell::new(Made(Vec::new())) };
}

/// A fresh, empty directory `nomos-<prefix>-<process>-<label>` in the
/// system's temporary directory.
pub fn dir(prefix: &str, label: &str) -> PathBuf {
    let dir = path(prefix, label);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The same path, created by nobody: for a file, a link, or a directory
/// the test makes itself. It is removed with the test all the same.
pub fn path(prefix: &str, label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("nomos-{prefix}-{}-{label}", std::process::id()));
    MADE.with(|made| made.borrow_mut().0.push(path.clone()));
    path
}
