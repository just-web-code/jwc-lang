//! `jwcproj.json`'s `"jwc"` — the language version the source is written
//! for (packages.md §1).
//!
//! The episode this exists for: an application written against one release
//! and compiled by another answered fifteen diagnostics that were about
//! the version gap and read as though they were about the code. Nothing in
//! the project said which release it was for, so there was nothing to
//! compare and no way to say so.
//!
//! The check sits in `Workspace::load`, which every command that compiles,
//! formats, runs or serves comes through — so what these tests pin is that
//! it reaches each of them, and that the shapes a manifest can carry mean
//! what they say.

use std::path::Path;
use std::process::Command;

const MINE: &str = env!("CARGO_PKG_VERSION");

fn project(language: Option<&str>) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let manifest = match language {
        Some(v) => format!(r#"{{ "name": "demo", "version": "0.1.0", "jwc": "{v}" }}"#),
        None => r#"{ "name": "demo", "version": "0.1.0" }"#.to_string(),
    };
    std::fs::write(dir.path().join("jwcproj.json"), manifest).expect("manifest");
    std::fs::write(
        dir.path().join("main.jwc"),
        "namespace app;\n\nserver {\n    port = 8099;\n}\n\nfunction main() {\n    serve();\n}\n",
    )
    .expect("source");
    dir
}

fn run(command: &str, dir: &Path) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_jwc"))
        .args([command, dir.to_str().expect("utf8")])
        .output()
        .expect("run jwc");
    (
        out.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}

#[test]
fn a_project_that_does_not_say_is_not_asked() {
    let dir = project(None);
    let (ok, out) = run("check", dir.path());
    assert!(ok, "{out}");
}

#[test]
fn the_version_this_compiler_is_satisfies_it() {
    let dir = project(Some(MINE));
    let (ok, out) = run("check", dir.path());
    assert!(ok, "{out}");
}

#[test]
fn a_star_is_any_version() {
    let dir = project(Some("*"));
    let (ok, out) = run("check", dir.path());
    assert!(ok, "{out}");
}

/// A bare version means exactly that version, as it does for a dependency.
/// `rc.N` and `rc.N+1` promise nothing to each other (SEMVER.md), so
/// "close enough" would be the wrong default for the one field whose job
/// is to catch the gap.
#[test]
fn another_version_is_refused_by_every_command_that_reads_the_source() {
    let dir = project(Some("0.9.900"));
    for command in ["check", "fmt", "run", "build", "gen-sql", "explain", "test"] {
        let (ok, out) = run(command, dir.path());
        assert!(!ok, "`jwc {command}` accepted the wrong compiler:\n{out}");
        assert!(
            out.contains("0.9.900") && out.contains(MINE),
            "`jwc {command}` must name both versions:\n{out}"
        );
    }
}

/// The message has to carry the fix, because the reader is looking at a
/// version number and not at their code.
#[test]
fn the_refusal_names_the_file_and_both_ways_out() {
    let dir = project(Some("0.9.900"));
    let (_, out) = run("check", dir.path());
    assert!(out.contains("jwcproj.json"), "{out}");
    assert!(out.contains("Install the version it asks for"), "{out}");
}

/// Standard semver: a range that names no pre-release never matches one.
/// It is the first thing people write, so the refusal says why rather
/// than leaving it to read as a bug here.
#[test]
fn a_range_that_names_no_prerelease_says_why_it_did_not_match() {
    if semver::Version::parse(MINE)
        .expect("own version")
        .pre
        .is_empty()
    {
        return;
    }
    let dir = project(Some("^1.0"));
    let (ok, out) = run("check", dir.path());
    assert!(!ok, "{out}");
    assert!(out.contains("pre-release"), "{out}");
}

/// A range that does name one is honoured — this is how a project says
/// "any candidate in this series".
#[test]
fn a_range_that_names_a_prerelease_matches() {
    let mine = semver::Version::parse(MINE).expect("own version");
    if mine.pre.is_empty() {
        return;
    }
    let dir = project(Some(&format!(
        "^{}.{}.{}-rc.1",
        mine.major, mine.minor, mine.patch
    )));
    let (ok, out) = run("check", dir.path());
    assert!(ok, "{out}");
}

/// A typo must not read as "whatever shipped today" — the same rule a
/// dependency's requirement follows.
#[test]
fn a_requirement_that_is_neither_is_an_error_naming_what_to_write() {
    let dir = project(Some("latest"));
    let (ok, out) = run("check", dir.path());
    assert!(!ok, "{out}");
    assert!(out.contains("neither a version nor a range"), "{out}");
    assert!(
        out.contains(MINE),
        "the message must name what to write:\n{out}"
    );
}

/// The specification's sample is compiled by this repository's own tests
/// with this repository's own compiler, so its pin has to move with every
/// release. Forgetting it turns every suite that loads the sample red at
/// once, which is loud but says nothing about what to do — this says it.
#[test]
fn the_sample_is_pinned_to_this_release() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/spec/v1/sample/jwcproj.json");
    let text = std::fs::read_to_string(&path).expect("sample manifest");
    let manifest: serde_json::Value = serde_json::from_str(&text).expect("json");
    assert_eq!(
        manifest.get("jwc").and_then(|v| v.as_str()),
        Some(MINE),
        "bump `jwc` in {} to {MINE} — it moves with Cargo.toml",
        path.display()
    );
}
