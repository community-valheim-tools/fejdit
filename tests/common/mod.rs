//! Helpers shared by the integration test binaries.

use std::path::PathBuf;

/// The most corrupt increment of `examples/BuildElementsCorrupted`, a world
/// version 34 save. `None` when the (git-ignored) samples are not checked out,
/// which is how these tests skip on a bare clone.
// Each test binary compiles this module separately, so a helper only some of
// them use reads as dead code in the rest.
#[allow(dead_code)]
pub fn corrupt_example() -> Option<PathBuf> {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/BuildElementsCorrupted");
    let mut hit = std::fs::read_dir(root)
        .ok()?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.is_dir()
                && p.file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with("11th"))
        })
        .collect::<Vec<_>>();
    hit.pop()
        .map(|d| d.join("taming11.db"))
        .filter(|p| p.is_file())
}

/// Run the CLI, returning (success, stdout, stderr).
pub fn fejdit<S: AsRef<std::ffi::OsStr>>(args: &[S]) -> (bool, String, String) {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_fejdit"))
        .args(args)
        .output()
        .unwrap();
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}
