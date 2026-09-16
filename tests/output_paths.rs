//! Where mutating commands are allowed to write. A world is two files and
//! only one of them is ever rewritten, so an output path that names the wrong
//! one, or something that is not a file at all, has to be caught rather than
//! acted on.

mod common;
use common::{corrupt_example, fejdit};

/// Backing up renames the target aside, so an existing directory would be
/// carried off wholesale and replaced with a file. Applies to every caller,
/// which is why it lives in `write_file` rather than in one command.
#[test]
fn write_file_refuses_a_target_that_is_not_a_file() {
    let dir = std::env::temp_dir().join(format!("fejdit-paths-{}", std::process::id()));
    let victim = dir.join("not-a-file");
    std::fs::create_dir_all(victim.join("contents")).unwrap();

    let err = fejdit::paths::write_file(&victim, b"payload", true).unwrap_err();
    assert!(
        err.to_string().contains("not a regular file"),
        "unexpected error: {err:#}"
    );
    assert!(victim.is_dir(), "the directory should be untouched");
    assert!(victim.join("contents").is_dir(), "its contents too");
    assert_eq!(
        std::fs::read_dir(&dir).unwrap().count(),
        1,
        "nothing else should have been created beside it"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// The world's `.fwl` is never rewritten by the cleaner, so `-o something.fwl`
/// can only produce a data file wearing the wrong extension.
#[test]
fn clean_rejects_an_fwl_output_path() {
    let Some(source) = corrupt_example() else {
        return;
    };
    let dir = std::env::temp_dir().join(format!("fejdit-fwl-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let target = dir.join("wrong.fwl");

    let (ok, _, stderr) = fejdit(&[
        "world".as_ref(),
        "clean-netobj-corruption".as_ref(),
        source.as_os_str(),
        "-o".as_ref(),
        target.as_os_str(),
    ]);
    assert!(!ok, "should have refused");
    assert!(
        stderr.contains("never the .fwl"),
        "unexpected error: {stderr}"
    );
    assert!(!target.exists(), "nothing should have been written");
    std::fs::remove_dir_all(&dir).ok();
}

/// A directory is a reasonable thing to point `-o` at; the world's own name
/// supplies the file.
#[test]
fn clean_writes_into_an_output_directory() {
    let Some(source) = corrupt_example() else {
        return;
    };
    let dir = std::env::temp_dir().join(format!("fejdit-outdir-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("bystander.txt"), "keep me").unwrap();

    let (ok, stdout, stderr) = fejdit(&[
        "world".as_ref(),
        "clean-netobj-corruption".as_ref(),
        source.as_os_str(),
        "-o".as_ref(),
        dir.as_os_str(),
    ]);
    assert!(ok, "{stderr}");
    let written = dir.join("taming11.db");
    assert!(
        written.is_file(),
        "expected {}, got {stdout}",
        written.display()
    );
    assert!(
        dir.is_dir(),
        "the output directory should still be a directory"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("bystander.txt")).unwrap(),
        "keep me"
    );
    // The .fwl is not written, so the user has to be told where to get one.
    assert!(
        stdout.contains("taming11.fwl"),
        "missing the .fwl note: {stdout}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// Either half of the pair, or the bare stem, names the same world.
#[test]
fn clean_accepts_either_file_of_the_pair() {
    let Some(db) = corrupt_example() else {
        return;
    };
    let fwl = db.with_extension("fwl");
    let stem = db.with_extension("");
    let mut reports = Vec::new();
    for input in [&db, &fwl, &stem] {
        let (ok, stdout, stderr) = fejdit(&[
            "world".as_ref(),
            "clean-netobj-corruption".as_ref(),
            input.as_os_str(),
            "--dry-run".as_ref(),
        ]);
        assert!(ok, "{} failed: {stderr}", input.display());
        reports.push(stdout);
    }
    assert_eq!(reports[0], reports[1], ".fwl and .db should resolve alike");
    assert_eq!(reports[1], reports[2], "the bare stem should too");
}
