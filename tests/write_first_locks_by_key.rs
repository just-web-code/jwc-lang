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

const SRC: &str = r#"
namespace app;

database App : Postgres;
schema s of App;

table Counters of App.s {
    id    bigint primary key identity;
    name  varchar(40) unique;
    hits  bigint default 0;
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
        2,
        "fixture should hold one update and one delete"
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
    for sql in built_sql() {
        assert!(
            sql.contains("FOR UPDATE"),
            "`first` must select under a lock:\n  {sql}"
        );
        // `quote_ident` leaves a plain lowercase name unquoted, so match
        // the shape rather than the quoting.
        assert!(
            sql.contains("WHERE x.id = (SELECT y.id FROM"),
            "`first` should match on the primary key:\n  {sql}"
        );
    }
}
