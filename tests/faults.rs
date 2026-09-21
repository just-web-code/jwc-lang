//! What a fault *says* (RC8-PLAN.md §1).
//!
//! 1kb.uz's `link` table was built by 0.9.x, and its `hits` column has no
//! `DEFAULT 0`. rc.7's insert omits the column, trusting the declared
//! default, so Postgres answers `23502 not_null_violation` — which is not
//! a named constraint, so `db.constraint()` is empty and the fault read
//!
//! ```text
//! [fault] constraint  violated
//! ```
//!
//! two spaces and nothing between them. Postgres had the sentence ready:
//! `null value in column "hits" of relation "link" violates not-null
//! constraint`. That sentence is what the fault says now.
//!
//! Requires Postgres. Set `JWC_V1_DATABASE_URL`. **A SKIPPED line is not a
//! pass.**

use jwc::serve::{self, Incoming};
use jwc::workspace::Workspace;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[tokio::test(flavor = "multi_thread")]
async fn a_not_null_violation_names_the_column() {
    let Ok(url) = std::env::var("JWC_V1_DATABASE_URL") else {
        eprintln!(
            "SKIPPED a_not_null_violation_names_the_column — set JWC_V1_DATABASE_URL. \
             A SKIPPED line is not a pass."
        );
        return;
    };

    let ws = Workspace::load(repo_root().join("tests/faults")).expect("load");
    let built = jwc::model::build(&ws);
    let ddl = jwc::ddl::render(&ws, &jwc::ddl::emit(&built.model), false);
    // The declared DDL, then the one thing 0.9.x's table lacked.
    let reset = format!(
        "DROP SCHEMA IF EXISTS f CASCADE;\n{ddl}\n\
         ALTER TABLE f.link ALTER COLUMN hits DROP DEFAULT;"
    );
    let out = std::process::Command::new("psql")
        .arg(&url)
        .args(["-q", "-v", "ON_ERROR_STOP=1", "-c", &reset])
        .output()
        .expect("psql");
    assert!(
        out.status.success(),
        "could not prepare the database: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // The detail is for the log; `JWC_DEBUG_ERRORS` puts it in the body,
    // which is the one place a test can read it.
    std::env::set_var("JWC_DEBUG_ERRORS", "1");
    jwc::engine::init_engine(&url).expect("engine");
    let program = Arc::new(serve::load(&ws).expect("the fixture must compile"));

    let r = serve::handle(
        program,
        Incoming {
            method: "POST".into(),
            path: "/links/abc".into(),
            query: Vec::new(),
            headers: HashMap::new(),
            body: Vec::new(),
            peer_ip: "203.0.113.7".into(),
        },
    )
    .await;

    // A fault, not a declared error: the column carries no message.
    assert_eq!(r.status, 500, "{}", r.body);
    let body: serde_json::Value = serde_json::from_str(&r.body).expect("json");
    let detail = body["error"].as_str().expect("an error string");
    assert!(
        detail.contains("column \"hits\"") && detail.contains("relation \"link\""),
        "the fault must name the column and the table: {detail}"
    );
    assert!(
        !detail.contains("constraint  violated"),
        "the empty-name message is back: {detail}"
    );
}
