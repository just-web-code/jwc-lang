//! `xs = array.push(@xs, v)` — the language's spelling of "append".
//!
//! Reading a local copies its value, so the obvious lowering makes the
//! accumulating loop that builds a response body copy the array once per
//! iteration: n copies for n elements. A 1000-row endpoint measured 28
//! responses a second against 14,645 for the same program on a build
//! without the regression, with a p99 of 22.5 seconds.
//!
//! Both backends answer it by moving the local into the call when the
//! result is assigned back over it, which leaves the callee holding the
//! only reference. They share the rule — `ast::self_append_item` — so
//! they cannot disagree about which programs take the path.
//!
//! What has to stay true is the part that is not about speed: appending
//! must not reach a value somebody else is still holding.

use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn fixture() -> PathBuf {
    repo_root().join("tests/append_in_place")
}

fn generated() -> String {
    let ws = jwc::workspace::Workspace::load(fixture()).expect("load");
    assert!(!ws.has_parse_errors(), "{}", ws.parse_errors().join(""));
    jwc::native::codegen_for_test(&ws).expect("codegen")
}

/// `accumulate(20000)`, `aliasing()`, `item_reads_the_target()` — see
/// `tests/append_in_place/src/app.jwc` for what each one is for.
const EXPECTED: &str = "20000\n1/2\n2/1\n";

#[test]
fn the_interpreter_appends_without_copying_and_without_aliasing() {
    let started = Instant::now();
    let out = Command::new(env!("CARGO_BIN_EXE_jwc"))
        .arg("run")
        .arg(fixture())
        .output()
        .expect("run jwc");
    let elapsed = started.elapsed();

    assert!(
        out.status.success(),
        "jwc run failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        EXPECTED,
        "the three shapes must answer the same as the native backend"
    );

    // Twenty thousand elements copied once each is minutes, not seconds,
    // in the debug build CI runs. The ceiling is deliberately far above
    // the ~0.25s this takes, so it fails on a return to copying and not
    // on a slow runner.
    assert!(
        elapsed < Duration::from_secs(30),
        "appending 20000 elements took {elapsed:?} — the copy per iteration is back"
    );
}

/// The generated source, not a built binary: what is pinned here is which
/// shape the backend emits for each of the three, which is the half that
/// decides whether the loop is linear.
#[test]
fn the_native_backend_moves_the_local_only_where_that_is_safe() {
    let src = generated();

    let appends: Vec<&str> = src
        .lines()
        .map(str::trim)
        // Call sites, not the prelude's definition of it.
        .filter(|l: &&str| l.contains("= jwc_b_v1_array_push("))
        .collect();

    // One per `array.push` in the fixture: the loop, the two in
    // `aliasing`, and the two in `item_reads_the_target`.
    assert_eq!(appends.len(), 5, "unexpected appends:\n{appends:#?}");

    // The first argument is the one that decides: the item may clone
    // whatever it likes.
    let moved = appends
        .iter()
        .filter(|l: &&&str| l.contains("jwc_b_v1_array_push(::std::mem::replace"))
        .count();
    let cloned = appends
        .iter()
        .filter(|l: &&&str| l.contains("jwc_b_v1_array_push(v_out.clone()"))
        .count();

    // `aliasing` reads the array into a second local between its two
    // appends, but that does not change how either one is emitted — the
    // move is safe there because `jwc_b_v1_array_push` copies when the
    // value is shared. Only the append whose item reads the local being
    // assigned has to keep the clone: the move empties it first, and
    // Rust evaluates arguments left to right.
    assert_eq!(
        cloned, 1,
        "expected exactly one copying append:\n{appends:#?}"
    );
    assert_eq!(moved, 4, "expected four moving appends:\n{appends:#?}");
}

/// `xs[i]` is an element by position. The backend used to emit
/// `jwc_get_field` with the index rendered as a string, so every index
/// looked up the key `""` and answered null while `jwc serve` answered
/// the element — the two backends disagreeing about an operator.
#[test]
fn the_native_backend_indexes_an_array_by_position() {
    let src = generated();

    assert!(
        src.contains("jwc_index(&"),
        "indexing must lower to `jwc_index`"
    );
    assert!(
        !src.contains("jwc_str_view(&V::Int"),
        "an index must not be rendered as a string key"
    );
}
