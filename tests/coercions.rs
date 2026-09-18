//! `boolean(x)` and `enum(E, x)` through the real pipeline (builtins.md
//! §2, types.md §7.2).
//!
//! Both are the door a `?done=` or `?status=` filter comes in through, and
//! both used to let a wrong value past: `boolean("bogus")` answered `false`
//! and the list came back filtered by a predicate the client never wrote;
//! `enum(E, "bogus")` never read `E`, so the first thing with an opinion
//! was Postgres, and the client got a 500 for its own typo.
//!
//! No database: every route here answers from the coercion alone.

use jwc::serve::{self, Incoming};
use jwc::workspace::Workspace;
use std::collections::HashMap;
use std::sync::Arc;

fn program(source: &str) -> Arc<jwc::exec::Program> {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("a.jwc"), source).expect("write");
    let ws = Workspace::load(dir.path()).expect("load");
    Arc::new(serve::load(&ws).unwrap_or_else(|e| panic!("{e}")))
}

async fn get(program: Arc<jwc::exec::Program>, path: &str, query: &[(&str, &str)]) -> jwc::exec::Response {
    serve::handle(
        program,
        Incoming {
            method: "GET".into(),
            path: path.into(),
            query: query
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            headers: HashMap::new(),
            body: Vec::new(),
            peer_ip: "203.0.113.7".into(),
        },
    )
    .await
}

const BOOLEAN: &str = "namespace c;\n\
                       routes \"/b\" {\n\
                       \x20   route GET \"\" {\n\
                       \x20       let done = boolean(request.query(\"done\"));\n\
                       \x20       return json({ done: @done });\n\
                       \x20   }\n\
                       \x20   route GET \"/{x: boolean}\" {\n\
                       \x20       return json({ done: @x });\n\
                       \x20   }\n\
                       }\n";

#[tokio::test]
async fn boolean_accepts_the_two_strings_a_route_parameter_accepts() {
    let p = program(BOOLEAN);

    for (raw, want) in [("true", "true"), ("false", "false")] {
        let r = get(p.clone(), "/b", &[("done", raw)]).await;
        assert_eq!(r.status, 200, "{}", r.body);
        assert!(r.body.contains(&format!("\"done\":{want}")), "{}", r.body);
    }

    // The same two strings through the other door. `1` is a 400 here, so
    // it cannot be a `true` through `boolean()` either.
    let r = get(p.clone(), "/b/true", &[]).await;
    assert_eq!(r.status, 200, "{}", r.body);
    let r = get(p.clone(), "/b/1", &[]).await;
    assert_eq!(r.status, 400, "{}", r.body);
    assert!(r.body.contains("bad_path_parameter"), "{}", r.body);

    for raw in ["bogus", "1", "TRUE", "yes", "t", ""] {
        let r = get(p.clone(), "/b", &[("done", raw)]).await;
        assert_eq!(r.status, 400, "`{raw}` must be refused: {}", r.body);
        assert!(r.body.contains("is not a boolean"), "{}", r.body);
    }
}

/// The absent case is the one that hid rows: `?done=` missing became
/// `false`, and an unfiltered list lost everything that was done.
#[tokio::test]
async fn boolean_of_null_raises_like_int_and_date() {
    let p = program(BOOLEAN);
    let r = get(p, "/b", &[]).await;
    assert_eq!(r.status, 400, "{}", r.body);
    assert!(r.body.contains("is not a boolean"), "{}", r.body);
}

const ENUM: &str = "namespace c;\n\
                    enum Priority { low, medium, high }\n\
                    routes \"/e\" {\n\
                    \x20   route GET \"\" {\n\
                    \x20       let p = enum(Priority, request.query(\"priority\"));\n\
                    \x20       return json({ priority: @p });\n\
                    \x20   }\n\
                    }\n";

#[tokio::test]
async fn enum_of_a_member_passes_and_null_stays_null() {
    let p = program(ENUM);

    let r = get(p.clone(), "/e", &[("priority", "high")]).await;
    assert_eq!(r.status, 200, "{}", r.body);
    assert!(r.body.contains("\"priority\":\"high\""), "{}", r.body);

    let r = get(p, "/e", &[]).await;
    assert_eq!(r.status, 200, "{}", r.body);
    assert!(r.body.contains("\"priority\":null"), "{}", r.body);
}

#[tokio::test]
async fn enum_of_a_non_member_is_the_clients_400_naming_the_members() {
    let p = program(ENUM);
    for raw in ["bogus", "High", "", "low "] {
        let r = get(p.clone(), "/e", &[("priority", raw)]).await;
        assert_eq!(r.status, 400, "`{raw}` must be refused: {}", r.body);
        assert!(r.body.contains("is not a Priority"), "{}", r.body);
        assert!(r.body.contains("low, medium, high"), "{}", r.body);
    }
}
