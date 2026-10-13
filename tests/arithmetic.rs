//! Arithmetic as the checker typed it (types.md §2.2, §2.3, §12) —
//! V1.1-PLAN entry 8.
//!
//! A fractional literal "is never a binary float", and the interpreter
//! went through an `f64`: `12345678901234.56 + 0.01` answered
//! `12345678901234.570312`. Worse, a `bigint` or `numeric` read back from
//! Postgres is a string — its wire form — and the runtime decided by the
//! value, so `price + price` was `"19.9919.99"` and `id + id` was `"11"`.
//!
//! The literal cases and the checker's marks need no database. The column
//! cases do: they read `JWC_V1_DATABASE_URL` and print SKIPPED without it
//! — and, as everywhere else in this repo, a SKIPPED line is not a pass.

use jwc::ast::{Arith, Decl, Stmt};
use jwc::serve::{self, Incoming};
use jwc::workspace::Workspace;
use std::collections::HashMap;
use std::sync::Arc;

fn workspace(source: &str) -> (tempfile::TempDir, Workspace) {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("a.jwc"), source).expect("write");
    let ws = Workspace::load(dir.path()).expect("load");
    assert!(!ws.has_parse_errors(), "{}", ws.parse_errors().join(""));
    (dir, ws)
}

fn program(source: &str) -> Arc<jwc::exec::Program> {
    let (_dir, ws) = workspace(source);
    Arc::new(serve::load(&ws).unwrap_or_else(|e| panic!("{e}")))
}

async fn get(program: &Arc<jwc::exec::Program>, path: &str) -> jwc::exec::Response {
    serve::handle(
        program.clone(),
        Incoming {
            method: "GET".into(),
            path: path.into(),
            query: Vec::new(),
            headers: HashMap::new(),
            body: Vec::new(),
            peer_ip: "203.0.113.7".into(),
        },
    )
    .await
}

const LITERALS: &str = "namespace a;\n\
     routes \"/n\" {\n\
     \x20   route GET \"/sum\" { return json({ v: 1.5 + 2.25 }); }\n\
     \x20   route GET \"/money\" { return json({ v: 12345678901234.56 + 0.01 }); }\n\
     \x20   route GET \"/tenth\" { return json({ v: 0.1 + 0.2 }); }\n\
     \x20   route GET \"/product\" { return json({ v: 2.5 * 2 }); }\n\
     \x20   route GET \"/third\" { return json({ v: 1.0 / 3 }); }\n\
     \x20   route GET \"/neg\" {\n\
     \x20       let x = -2.5;\n\
     \x20       return json({ v: -x });\n\
     \x20   }\n\
     \x20   route GET \"/overflow\" {\n\
     \x20       let big = 9223372036854775807;\n\
     \x20       return json({ v: big + 1 });\n\
     \x20   }\n\
     \x20   route GET \"/zero\" {\n\
     \x20       let z = 0.0;\n\
     \x20       return json({ v: 1.5 / z });\n\
     \x20   }\n\
     }\n";

#[tokio::test]
async fn numeric_is_decimal_not_a_float() {
    let p = program(LITERALS);
    for (path, want) in [
        ("/n/sum", r#"{"v":"3.75"}"#),
        ("/n/money", r#"{"v":"12345678901234.57"}"#),
        ("/n/tenth", r#"{"v":"0.3"}"#),
        // Trailing zeros were always dropped; that stays.
        ("/n/product", r#"{"v":"5"}"#),
        ("/n/third", r#"{"v":"0.3333333333333333333333333333"}"#),
        // Was `--2.5`, which nothing could read back.
        ("/n/neg", r#"{"v":"2.5"}"#),
    ] {
        let r = get(&p, path).await;
        assert_eq!((r.status, r.body.as_str()), (200, want), "{path}");
    }
}

#[tokio::test]
async fn overflow_and_division_by_zero_are_faults() {
    let p = program(LITERALS);
    for path in ["/n/overflow", "/n/zero"] {
        let r = get(&p, path).await;
        assert_eq!(r.status, 500, "{path}: {}", r.body);
    }
}

const COLUMNS: &str = "namespace a;\n\
     database App : Postgres { init() { pool_size = 2; tls = false; } }\n\
     schema arith of App;\n\
     table Items of App.arith {\n\
     \x20   id    bigint primary key identity;\n\
     \x20   price numeric(12,2);\n\
     \x20   qty   int;\n\
     \x20   label text;\n\
     }\n\
     service Shop {\n\
     \x20   function one(id: bigint) {\n\
     \x20       return select I from App.arith.Items\n\
     \x20           where I.id == @id\n\
     \x20           as { I.id, I.price, I.qty, I.label }\n\
     \x20           first or throw NotFound(\"no item\");\n\
     \x20   }\n\
     }\n\
     routes \"/p/{id: bigint}\" {\n\
     \x20   route GET \"\" {\n\
     \x20       let it = Shop.one(@id);\n\
     \x20       let sum = it.price + it.price;\n\
     \x20       let line = it.price * it.qty;\n\
     \x20       let neg = -it.price;\n\
     \x20       let ids = string.of(it.id + it.id);\n\
     \x20       let next = it.id + 1;\n\
     \x20       let both = it.label + it.label;\n\
     \x20       return json({ sum: @sum, line: @line, neg: @neg, ids: @ids, next: @next, both: @both });\n\
     \x20   }\n\
     }\n";

/// Every operator over a column is marked with the arithmetic its declared
/// types make it, and concatenation is not — including through a function
/// call, whose result the first checker pass cannot type.
#[test]
fn the_checker_marks_what_each_operator_is() {
    let (_dir, ws) = workspace(COLUMNS);
    let built = jwc::model::build(&ws);
    let sym = jwc::symbols::build(&ws, &built.model);
    let first = jwc::check::check(&ws, &sym, &built.model);
    jwc::check::check_with(&ws, &sym, &built.model, &first.function_returns);

    let mut marks = HashMap::new();
    for decl in &ws.files[0].program.decls {
        let Decl::Routes(r) = decl else { continue };
        for stmt in &r.routes[0].body {
            if let Stmt::Let { name, value, .. } = stmt {
                marks.insert(name.name.clone(), value.arith.get());
            }
        }
    }
    assert_eq!(marks["sum"], Some(Arith::Numeric));
    assert_eq!(marks["line"], Some(Arith::Numeric));
    assert_eq!(marks["neg"], Some(Arith::Numeric));
    assert_eq!(marks["next"], Some(Arith::Int));
    assert_eq!(marks["both"], None, "text concatenation is not arithmetic");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_column_is_the_type_it_declares() {
    let Ok(url) = std::env::var("JWC_V1_DATABASE_URL") else {
        eprintln!(
            "SKIPPED a_column_is_the_type_it_declares — set JWC_V1_DATABASE_URL \
             to a Postgres connection string. A SKIPPED line is not a pass."
        );
        return;
    };
    let out = std::process::Command::new("psql")
        .arg(&url)
        .args(["-q", "-v", "ON_ERROR_STOP=1", "-c"])
        .arg(
            "DROP SCHEMA IF EXISTS arith CASCADE; CREATE SCHEMA arith;\n\
             CREATE TABLE arith.items (\n\
             \x20   id bigint GENERATED BY DEFAULT AS IDENTITY PRIMARY KEY,\n\
             \x20   price numeric(12,2) NOT NULL, qty integer NOT NULL, label text NOT NULL);\n\
             INSERT INTO arith.items (price, qty, label) VALUES (19.99, 3, 'ab');",
        )
        .output()
        .expect("psql");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::env::set_var("DATABASE_URL", &url);
    jwc::engine::init_engine(&url).expect("engine");

    let p = program(COLUMNS);
    let r = get(&p, "/p/1").await;
    assert_eq!(r.status, 200, "{}", r.body);
    // 1.0.1 answered `"19.9919.99"`, a 500 for `line` and `neg`, and
    // `"11"` for `ids`. `next` is a `bigint`, so a string on the wire
    // (types.md §2.3), as `@id + 1` on a `bigint` path parameter always was.
    assert_eq!(
        r.body,
        r#"{"sum":"39.98","line":"59.97","neg":"-19.99","ids":"2","next":"2","both":"abab"}"#
    );
}
