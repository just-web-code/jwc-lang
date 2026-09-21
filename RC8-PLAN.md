# rc.8 plan

What rc.7 turned up the day it shipped. Five projects moved to it in one
afternoon, and moving them broke three things in the compiler and one
production deployment. The compiler fixes are on `main` already, under
`[Unreleased]`; the deployment is what the rest of this plan is about.

Each entry says what was run, what happened, where the code is, and which
clause it contradicts — so the decision can be about whether to fix it, not
about whether it is real.

Companion to [`RC7-PLAN.md`](RC7-PLAN.md). Item 11 there (`parallel { }`)
stays in 1.1, and so does the thing it was for; this plan does not reopen it.

## Ledger

| # | Found in | What | Class | Status |
|---|---|---|---|---|
| 1 | shortener prod | a `NOT NULL` violation reads `constraint  violated` — two spaces, no name, no column | ergonomics | fixed |
| 2 | shortener prod | `migrate baseline` adopts a schema whose columns have no defaults, and `verify` calls it fine | soundness | fixed |
| 3 | shortener, e-school | nothing runs a `job` on a clock — expired rows wait for a cron outside the program | gap | planned — `every` |
| 4 | e-school, redis, task-tracker | three `jwc fix` / `jwc fmt` defects, already on `main` | ergonomics | fixed |

Classes as in RC7-PLAN.md. 3 is a **gap** and would ordinarily wait for
1.1; it is here because it is the missing half of a feature 1.0 already
ships (`job` / `dispatch`), not a new one, and because the two projects
that need it are the two that are deployed.

---

## 1. `constraint  violated`

**Class:** ergonomics · **Found:** `POST /api/links` on 1kb.uz, 500

The live `link` table had been built by 0.9.x and its `hits` column had no
`DEFAULT 0`. rc.7's insert omits the column, trusting the declared default,
so Postgres answered `23502 not_null_violation`. The fault that reached the
log:

```
[fault] constraint  violated
```

`src/db.rs:296` takes `db.constraint()`, which a not-null violation does not
carry — it is not a named constraint — so `name` is empty and the message
says nothing. The same line in the native prelude (`db.rs.in:348`) says
the same nothing plus the SQL. Postgres had the whole sentence ready:
`null value in column "hits" of relation "link" violates not-null
constraint`, with `table` and `column` as fields.

**Fix.** `DbError::Constraint` carries `table` and `column` alongside
`name`, filled from the fields Postgres sends. A fault without a declared
message is Postgres's own message — the sentence above — on both backends.
A violation *with* a declared message is unchanged: it is a `Conflict` /
`BadRequest` and the sentence is the author's. Pinned by a test in
`tests/coercions.rs`'s style against a table with a `not null` column
and no default, on both backends.

---

## 2. `baseline` adopts columns it has not looked at

**Class:** soundness · **Found:** the same 500, one layer down

migrations.md §12.2 gates `baseline` on `information_schema`: every
declared table and column has to exist. §12.3 then reports what differs
— but `apply::verify` reads constraint *names*, index *names* and views,
and nothing about the columns themselves. A live column with no default
where the declaration has one, or nullable where the declaration says
`not null`, passes both the gate and the report:

```
2 migrations adopted; the database was not touched
3 differences remain …
  public.link: index `ix_link__hits` is missing
  …
```

and then `hits` faults on the first insert. `verify` said "ok — every
constraint, index and view is present under its expected name", which was
true and beside the point.

**Fix.** `verify` reads `information_schema.columns` too, and compares
each declared column's **presence of a default** and **nullability**
against the live one. Presence, not text: Postgres normalises a default
expression (`0` → `0`, `'x'` → `'x'::text`, `now()` → `now()`), and
comparing text would report a difference on every adopted database. The
two checks that matter are the two that make a write fault:

```
public.link: column `hits` has no default — declared `0`
public.link: column `created_at` has no default — declared `now()`
public.api_call: column `ts` has no default — declared `now()`
public.link: column `slug` is nullable — declared `not null`
```

A live column with a default the declaration does not have is *not*
reported: it cannot make a write fail. Type differences are out of scope
here — the snapshot renders the type and Postgres reports it in another
spelling, and matching them is its own piece of work.

Both `baseline`'s "differences remain" list and `migrate verify` come from
`verify`, so both say it. Pinned in `tests/migrate_apply.rs` against a
table created by hand without its default.

---

## 3. `every` — a job on a clock

**Class:** gap · **Found:** shortener's expired links, e-school's reports

jobs.md gives a `job` one way to run: a `dispatch` from a request. A
program that has to delete expired rows every ten minutes, or close a
report at midnight, has no way to say so, and the two deployed projects
both carry a cron entry outside the program for it — the one place the
runtime's guarantees (§3.3 at-least-once, §3.4 dead-letter, `/metrics`)
do not reach.

**Design.** One modifier on `job`, in the row `retries` and `backoff` are
in:

```jwc no-compile
job CleanupExpired() every "10m" {
    delete L from App.Links where L.expires_at < now();
}
```

- A scheduled job **takes no parameters** (`E0377`): there is no call to
  receive them from.
- It **cannot be dispatched** (`E0378`): the clock is its only caller.
  "Run it now" is a question for 1.1 if anyone asks it.
- The interval is the `backoff` grammar, the same parser, `1s..=30d`
  (`E0379` outside).

**Runtime.** No new table and no new worker. `_jwc_jobs` gains a column,
`every_secs int`, added at boot with `ADD COLUMN IF NOT EXISTS` the way the
tables are created, and a unique partial index on `(name) WHERE every_secs
IS NOT NULL`. That index is the whole scheduler:

- At boot every replica seeds one row per `every` job, `run_at = now() +
  every`, `ON CONFLICT DO NOTHING` — so N replicas seed one row, not N.
- When the row finishes — success or dead-letter — the settling statement
  deletes it and inserts the next one, `run_at = now() + every`, in one
  statement (`WITH d AS (DELETE … RETURNING …) INSERT … ON CONFLICT DO
  NOTHING`). One row of a scheduled job exists at any time, so two ticks
  cannot overlap, and a tick that took longer than the interval simply
  starts the next one late rather than stacking.
- `retries` / `backoff` mean what they mean: attempts *within* a tick.
  The attempt that exhausts them dead-letters that tick, as §3.4 says, and
  the next tick is still scheduled — a report that failed at midnight
  should try again tomorrow, and the dead row is the record of tonight.
- Removing the declaration: the seeded row outlives it, is claimed, and
  dead-letters as "a queued row outlived its declaration" (§3.4) — once,
  because the re-insert happens only in `succeed`, which never runs.

Both backends transcribe the same SQL, as they do for the rest of the
queue, and `tests/jobs_queue.rs` pins the seed, the settle and the
`ON CONFLICT` against a real database. `jobs.md` §1.4 specifies it,
`grammar.ebnf` carries `every`, `jwc fmt` prints it, and the user docs
under `docs/docs/backend/jobs.md` show the cleanup job.

**Not in this.** `dispatch` of a scheduled job, cron expressions
(`"0 0 * * *"`), a time zone, "at most once" — every one is a real
question and none is what the two projects need.

---

## 4. The three on `main`

`fdffb40` (`jwc fix` inside a nested shape), `0104c83` (`jwc fmt` keeps a
trailing comment), `503e31d` (`---` → `///`). Written up under
`[Unreleased]` already; rc.8 is what releases them.

---

## Order

1, 2, 3, then the docs and the version — 1 and 2 are small and are what
prod needed; 3 is the one that takes the afternoon. Each on `rc8`, one
commit, pinned by the test its entry names.
