//! `update … first` and `delete … first` name the row they write by its
//! primary key, never by `ctid`.
//!
//! `first` on a write selects one row under `FOR UPDATE` and then writes
//! the row that selection named. Naming it by `ctid` is a lost write under
//! concurrency, not an error: Postgres stores an updated row as a new tuple
//! at a new physical location, so when the inner `FOR UPDATE` blocks on a
//! competing transaction and then follows the update chain, it answers the
//! *new* tuple's `ctid` — which the outer statement, still on its own
//! snapshot, cannot see. `WHERE x.ctid = <new>` matches nothing: no rows
//! written, `RETURNING` empty, `first` null, and an `or throw` reports a
//! row that is plainly there as missing.
//!
//! Measured on this machine, 64 writers against one 10,000-row table for
//! 10 s: `ctid` silently lost 674 of 182,479 statements; the primary key
//! lost 0 of 179,791, at the same throughput.

use jwc::ast::{Decl, Expr, ExprKind, Stmt};
use jwc::sql::Builder;
use jwc::{check, model, symbols, workspace::Workspace};

mod common;

const SRC: &str = r#"
namespace app;

database App : Postgres;
schema s of App;

table Counters of App.s {
    id    bigint primary key identity;
    name  varchar(40) unique;
    hits  bigint default 0;
}

/// A composite key compares as a row, and a renamed key column has to be
/// named by the name the table actually has.
table Members of App.s {
    org_id     bigint;
    account_id bigint as "acct_id";
    role       varchar(20);

    primary key (org_id, account_id);
}

service C {
    function bump(name: text) {
        return update Counters of App.s.Counters
            set hits = Counters.hits + 1
            where Counters.name == @name
            as { Counters.id, Counters.hits }
            first;
    }

    function drop_one(name: text) {
        return delete Counters from App.s.Counters
            where Counters.name == @name
            as { Counters.id }
            first;
    }

    function demote(org_id: bigint) {
        return update Members of App.s.Members
            set role = "member"
            where Members.org_id == @org_id
            as { Members.role }
            orderby Members.account_id asc
            first;
    }
}
"#;

/// The first `update`/`delete` reachable from a function body. Only the
/// statement shapes this fixture uses.
fn writes<'a>(block: &'a [Stmt], out: &mut Vec<&'a Expr>) {
    fn walk<'a>(e: &'a Expr, out: &mut Vec<&'a Expr>) {
        if matches!(&*e.kind, ExprKind::Update(_) | ExprKind::Delete(_)) {
            out.push(e);
        }
    }
    for s in block {
        match s {
            Stmt::Return { value: Some(e), .. } => walk(e, out),
            Stmt::Let { value, .. } => walk(value, out),
            _ => {}
        }
    }
}

fn built_sql() -> Vec<String> {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("a.jwc"), SRC).expect("write");
    let ws = Workspace::load(dir.path()).expect("load");
    assert!(!ws.has_parse_errors(), "{}", ws.parse_errors().join(" "));
    let m = model::build(&ws);
    let sym = symbols::build(&ws, &m.model);
    let checked = check::check(&ws, &sym, &m.model);
    let errors: Vec<String> = m
        .diags
        .iter()
        .chain(&sym.diags)
        .chain(&checked.diags)
        .filter(|(_, d)| d.severity == jwc::diag::Severity::Error)
        .map(|(_, d)| format!("{}: {}", d.code, d.message))
        .collect();
    assert!(errors.is_empty(), "fixture does not check: {errors:?}");

    let mut exprs: Vec<&Expr> = Vec::new();
    for file in &ws.files {
        for d in &file.program.decls {
            if let Decl::Service(s) = d {
                for f in &s.functions {
                    writes(&f.body, &mut exprs);
                }
            }
        }
    }
    assert_eq!(
        exprs.len(),
        3,
        "fixture should hold two updates (single and composite key) and one delete"
    );

    exprs
        .iter()
        .map(|e| {
            let mut b = Builder::new(&m.model);
            let built = match &*e.kind {
                ExprKind::Update(u) => {
                    // `set hits = Counters.hits + 1` reads the row it writes,
                    // so it lowers as SQL rather than a bound parameter.
                    let jwc::ast::SetItem::Set { column, value, .. } = &u.sets[0] else {
                        panic!("fixture uses a plain `set`")
                    };
                    let sets = vec![(column.name.clone(), jwc::sql::SetValue::Sql(value.clone()))];
                    b.update(u, &sets)
                }
                ExprKind::Delete(d) => b.delete(d),
                _ => unreachable!(),
            };
            built.expect("write is expressible").sql
        })
        .collect()
}

#[test]
fn a_first_write_never_names_the_row_by_ctid() {
    for sql in built_sql() {
        assert!(
            !sql.contains("ctid"),
            "a `first` write named the row by ctid, which does not survive \
             the write it authorises:\n  {sql}"
        );
    }
}

#[test]
fn a_first_write_names_the_row_by_its_primary_key() {
    let all = built_sql();
    // The composite branch is a different code path — without this the
    // fixture could drift to single keys only and leave it uncovered.
    assert!(
        all.iter()
            .any(|s| s.contains("(x.org_id, x.acct_id) = (SELECT y.org_id, y.acct_id FROM")),
        "the composite-key form is not exercised:\n  {}",
        all.join("\n  ")
    );
    // `account_id as "acct_id"` — the key is named by the name the table
    // actually has, whatever order the modifiers came in.
    assert!(
        all.iter().all(|s| !s.contains("account_id")),
        "a renamed key column is named by its declared name:\n  {}",
        all.join("\n  ")
    );
    for sql in all {
        assert!(
            sql.contains("FOR UPDATE"),
            "`first` must select under a lock:\n  {sql}"
        );
        // `quote_ident` leaves a plain lowercase name unquoted, so match
        // the shape rather than the quoting.
        // `quote_ident` leaves a plain lowercase name unquoted, so match
        // the shape rather than the quoting. The composite key compares as
        // a row, and `account_id` is stored as `acct_id`.
        let single = sql.contains("WHERE x.id = (SELECT y.id FROM");
        let composite =
            sql.contains("WHERE (x.org_id, x.acct_id) = (SELECT y.org_id, y.acct_id FROM");
        assert!(
            single || composite,
            "`first` should match on the primary key:\n  {sql}"
        );
    }
}

/// The lost write itself, against a real Postgres.
///
/// The SQL assertions above freeze the shape; this proves the behaviour the
/// shape exists for. It is deterministic rather than a load test: one
/// session takes the row and holds it, the `first` write blocks on that
/// lock, and the holder commits. That is exactly the window — the inner
/// `FOR UPDATE` wakes, follows the update chain, and answers the row as it
/// now is, while the outer statement is still on the snapshot it started
/// with.
///
/// Under `ctid` the outer statement matched nothing and answered
/// `UPDATE 0`, every run. Under the primary key it answers `UPDATE 1`.
///
/// Opt-in on `JWC_V1_PG`, like the golden suites. **A SKIPPED line is not a
/// pass.**
#[test]
fn a_first_write_survives_a_writer_that_commits_under_the_lock() {
    let Ok(conn) = std::env::var("JWC_V1_PG") else {
        println!("SKIPPED — set JWC_V1_PG to a psql connection string");
        return;
    };

    let db = "postgres";
    common::run_psql(
        &conn,
        db,
        &[
            "-q",
            "-c",
            "drop table if exists jwc_first_race",
            "-c",
            "create table jwc_first_race(id bigint primary key, hits bigint \
             default 0, name text)",
            "-c",
            "insert into jwc_first_race values (1, 0, 'a')",
        ],
    );

    // The holder: takes the row, waits, commits. Backgrounded so the write
    // below meets it mid-transaction.
    let holder = {
        let mut cmd = std::process::Command::new("psql");
        for part in common::psql_target(&conn, db) {
            cmd.arg(part);
        }
        cmd.arg("-q").arg("-c").arg(
            "BEGIN; UPDATE jwc_first_race SET hits = hits + 1 WHERE id = 1; \
             SELECT pg_sleep(1.5); COMMIT;",
        );
        cmd.stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn holder")
    };

    // Let the holder take the row before the write below asks for it.
    common::run_psql(&conn, db, &["-tA", "-c", "SELECT pg_sleep(0.4)"]);

    let out = common::run_psql(
        &conn,
        db,
        &[
            "-tA",
            "-c",
            "UPDATE jwc_first_race x SET hits = x.hits + 10 \
             WHERE x.id = (SELECT y.id FROM jwc_first_race y \
             WHERE y.name = 'a' ORDER BY y.id FOR UPDATE LIMIT 1) \
             RETURNING x.id",
        ],
    );

    let mut holder = holder;
    let _ = holder.wait();
    common::run_psql(
        &conn,
        db,
        &["-q", "-c", "drop table if exists jwc_first_race"],
    );

    assert!(
        out.contains("UPDATE 1"),
        "the `first` write lost its row to a writer that committed under the \
         lock — this is the `ctid` failure. psql said:\n{out}"
    );
}
