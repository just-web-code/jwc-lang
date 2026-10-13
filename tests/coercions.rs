//! Coercions through the real pipeline (builtins.md §2 and §3, types.md
//! §7.2, routing.md §3.2).
//!
//! These are the doors client text comes in through, and each one used to
//! let a wrong value past. `boolean("bogus")` answered `false` and the
//! list came back filtered by a predicate the client never wrote.
//! `enum(E, "bogus")` never read `E`, so the first thing with an opinion
//! was Postgres and the client got a 500 for its own typo.
//! `date.parse("kecha")` returned the string it was given, so the
//! `timestamptz?` it declares was never null and the `or throw
//! BadRequest(…)` the type system makes the author write never fired.
//! And five of the scalar types were accepted as a path parameter without
//! being read at all.
//!
//! No database: every route here answers from the coercion alone, which
//! is the point — none of this should ever have needed one.

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

async fn get(
    program: Arc<jwc::exec::Program>,
    path: &str,
    query: &[(&str, &str)],
) -> jwc::exec::Response {
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

const DATES: &str = "namespace c;\n\
                     routes \"/d\" {\n\
                     \x20   route GET \"\" {\n\
                     \x20       let t = date.parse(request.query(\"t\") ?? \"\");\n\
                     \x20       return json({ at: @t });\n\
                     \x20   }\n\
                     \x20   route GET \"strict\" {\n\
                     \x20       let raw = request.query(\"t\") ?? \"\";\n\
                     \x20       let t = date.parse(@raw) or throw BadRequest(\"sana yaroqsiz\");\n\
                     \x20       return json({ at: @t });\n\
                     \x20   }\n\
                     \x20   route GET \"fmt\" {\n\
                     \x20       return text(date.format(date.parse(\"2026-03-01T10:20:30Z\") ?? date.now(), \"%d/%m/%Y %H:%M\"));\n\
                     \x20   }\n\
                     }\n";

/// The `?` in `timestamptz?` is the contract. It never held: the builtin
/// returned its own input wrapped as a timestamp, so null was impossible
/// and the string travelled on to Postgres.
#[tokio::test]
async fn date_parse_answers_null_for_a_string_that_is_not_a_timestamp() {
    let p = program(DATES);

    let r = get(p.clone(), "/d", &[("t", "2026-03-01T10:20:30Z")]).await;
    assert_eq!(r.status, 200, "{}", r.body);
    assert!(r.body.contains("2026-03-01T10:20:30"), "{}", r.body);

    for raw in ["kecha", "2026-13-45", "", "2026-03-01"] {
        let r = get(p.clone(), "/d", &[("t", raw)]).await;
        assert_eq!(r.status, 200, "`{raw}`: {}", r.body);
        assert!(
            r.body.contains("\"at\":null"),
            "`{raw}` must be null: {}",
            r.body
        );
    }
}

/// What the null is *for*. The type system refuses `timestamptz?` where a
/// `timestamptz` is wanted, so the author writes the guard — and before
/// this the guard was dead code and the client got a 500 from Postgres
/// where the program plainly said 400.
#[tokio::test]
async fn the_guard_the_type_system_demands_actually_fires() {
    let p = program(DATES);

    let r = get(p.clone(), "/d/strict", &[("t", "2026-03-01T10:20:30Z")]).await;
    assert_eq!(r.status, 200, "{}", r.body);

    let r = get(p, "/d/strict", &[("t", "kecha")]).await;
    assert_eq!(r.status, 400, "{}", r.body);
    assert!(r.body.contains("sana yaroqsiz"), "{}", r.body);
}

/// The second argument was read by nobody: the checker ignored it and the
/// runtime returned the timestamp whatever it said, so a report formatted
/// for a human came out as a machine timestamp with no error to say so.
#[tokio::test]
async fn date_format_uses_the_format_it_was_given() {
    let p = program(DATES);
    let r = get(p, "/d/fmt", &[]).await;
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.body, "01/03/2026 10:20", "{}", r.body);
}

const PARAMS: &str = "namespace c;\n\
                      routes \"/p\" {\n\
                      \x20   route GET \"date/{x: date}\" { return json({ v: @x }); }\n\
                      \x20   route GET \"ts/{x: timestamptz}\" { return json({ v: @x }); }\n\
                      \x20   route GET \"time/{x: time}\" { return json({ v: @x }); }\n\
                      \x20   route GET \"inet/{x: inet}\" { return json({ v: @x }); }\n\
                      \x20   route GET \"bytea/{x: bytea}\" { return json({ v: @x }); }\n\
                      }\n";

/// routing.md §3.2 promises a 400 naming the parameter, and gives the
/// reason: "malformed input reached Postgres and became a 500". Five of
/// the scalar types fell through to the `text` catch-all, so for those it
/// still did.
///
/// `2026-02-30` is the one worth keeping: it is shaped like a date and
/// only a real calendar rejects it, which is why Postgres was the first
/// thing to notice.
///
/// No CIDR case here, and not because it is unchecked: a `/` is a segment
/// boundary, so `192.0.2.1/24` in a path is two segments and the route
/// does not match at all. An `inet` path parameter is an address; the
/// prefix form only reaches a program through a body or a query string.
#[tokio::test]
async fn every_typed_path_parameter_is_read_before_the_handler_runs() {
    let p = program(PARAMS);

    for (ty, good) in [
        ("date", "2026-03-01"),
        ("ts", "2026-03-01T10:20:30Z"),
        ("time", "10:20:30"),
        ("inet", "192.0.2.1"),
        ("bytea", "aGVsbG8="),
    ] {
        let r = get(p.clone(), &format!("/p/{ty}/{good}"), &[]).await;
        assert_eq!(r.status, 200, "{ty} `{good}`: {}", r.body);
    }

    for (ty, bad) in [
        ("date", "kecha"),
        ("date", "2026-02-30"),
        ("date", "2026-13-01"),
        ("ts", "kecha"),
        ("ts", "2026-03-01"),
        ("time", "25:00:00"),
        ("inet", "999.1.1.1"),
        ("inet", "192.0.2.1."),
        ("bytea", "not_base64"),
        ("bytea", "aGVsbG8"),
    ] {
        let r = get(p.clone(), &format!("/p/{ty}/{bad}"), &[]).await;
        assert_eq!(r.status, 400, "{ty} `{bad}` must be refused: {}", r.body);
        assert!(r.body.contains("bad_path_parameter"), "{}", r.body);
    }
}

const INTERVALS: &str = "namespace c;\n\
                         routes \"/i\" {\n\
                         \x20   route GET \"\" {\n\
                         \x20       let a = date.parse(\"2026-10-07T06:00:00Z\") ?? date.now();\n\
                         \x20       let b = date.parse(\"2026-10-07T06:00:01.248Z\") ?? date.now();\n\
                         \x20       let d = @b - @a;\n\
                         \x20       return json({\n\
                         \x20           iso: string.of(@d),\n\
                         \x20           s: date.total_seconds(@d),\n\
                         \x20           ms: date.total_millis(@d),\n\
                         \x20           us: date.total_micros(@d),\n\
                         \x20           back: string.of(@a + @d) == string.of(@b),\n\
                         \x20           whole: string.of(date.seconds(10))\n\
                         \x20       });\n\
                         \x20   }\n\
                         }\n";

/// Both operands carry microseconds on the wire. The difference used to
/// be truncated to whole seconds — 1.248 s came back `PT1S` — and nothing
/// read a number out of an interval at all, so a program could not time
/// itself, or divide a count by a duration, or compare one to a budget.
#[tokio::test]
async fn a_timestamp_difference_keeps_its_fraction_and_reads_back_as_a_number() {
    let p = program(INTERVALS);
    let r = get(p, "/i", &[]).await;
    assert_eq!(r.status, 200, "{}", r.body);
    assert!(r.body.contains("\"iso\":\"PT1.248S\""), "{}", r.body);
    assert!(r.body.contains("\"s\":\"1.248\""), "{}", r.body);
    // `bigint` is a string on the wire (types.md §2.3).
    assert!(r.body.contains("\"ms\":\"1248\""), "{}", r.body);
    assert!(r.body.contains("\"us\":\"1248000\""), "{}", r.body);
    // and the interval goes back on as the same shift
    assert!(r.body.contains("\"back\":true"), "{}", r.body);
    // a whole number of seconds renders the bytes it always did
    assert!(r.body.contains("\"whole\":\"PT10S\""), "{}", r.body);
}
