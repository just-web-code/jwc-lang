# rc.7 plan

Defects and rough edges found by writing and migrating real applications on
`1.0.0-rc.6`. **Nothing here is fixed.** It is the list to decide from.

Each entry says what was run, what happened, where the code is, and which
clause it contradicts — so the decision can be about whether to fix it, not
about whether it is real. Anything that turns out not to be a defect is kept
with that verdict written down, because the reasoning is the useful part.

Companion to [`TODO.md`](TODO.md), which holds field defects found against
0.9.x and earlier. This file is scoped to the v1 candidate series.

## Ledger

| # | Found in | What | Class |
|---|---|---|---|
| 1 | todo CRUD, e-school | `boolean(x)` answers `false` for everything it does not recognise | wrong status |
| 2 | todo CRUD, e-school | `enum(E, x)` never reads `E` | wrong status |
| 3 | e-school | `jwc fmt --check` and `jwc fmt` report each other's outcome | ergonomics |
| 4 | e-school | `jwc explain` lists no write statement | ergonomics |
| 5 | MyWallet, shortener | an absent `jwc` field means "any version", so the projects that need the check never get it | ergonomics |
| 6 | MyWallet, shortener | one `--` comment produces one diagnostic per character on the line | ergonomics |
| 7 | MyWallet, shortener | rc.3's write binder shipped without a migration diagnostic | ergonomics |
| 8 | MyWallet, shortener | 116 diagnostics carried a machine-applicable fix and nothing applies them | ergonomics |
| 9 | MyWallet, shortener | `jwc fmt` cannot format a file with a comment inside `server { }` | ergonomics |
| 10 | reading the language | `after` runs *before* the response is sent, and only a write can be taken off that path | naming, gap |
| 11 | shortener | nothing in the language runs two things at once inside one request | gap |

Classes: **soundness** (a wrong answer), **wrong status** (the right refusal
at the wrong layer, so a client mistake reads as a server fault),
**ergonomics** (correct, but the program has to say it the long way),
**naming** (right thing, wrong word), **gap** (the language has no way to
say it at all — a 1.1 question, not an rc.7 fix).

---

## 1. `boolean(x)` answers `false` for everything it does not recognise

**Class:** wrong status · **Found:** writing a `?done=` filter

`src/exec_call.rs:648` is the whole implementation:

```rust
"boolean" => Value::Bool(matches!(s(0).as_str(), "true" | "1")),
```

`s(0)` is `text(&Value)`, which maps `Value::Null` to `""`
(`src/exec_call.rs:35`). So every input that is not the literal `true` or `1`
— including no input at all — is `false`, silently:

| `?done=` | `boolean(…)` | what the client gets |
|---|---|---|
| absent | `false` | 200, list filtered to `done = false` |
| `bogus` | `false` | 200, list filtered to `done = false` |
| `yes` / `TRUE` / `t` | `false` | 200, list filtered to `done = false` |
| `1` | `true` | 200, filtered to `done = true` |

The absent case is the damaging one. The idiom the spec reaches for is
`where T.done ==? @done`, where a null drops the predicate — but the null
never survives the coercion, so a filter the caller did not ask for stays on
and the finished rows vanish from an unfiltered list. It is a wrong answer
with a 200 on it.

Two things already in the tree disagree with this line:

- **The same type as a route parameter.** `src/serve.rs:617` binds a
  `{x: boolean}` path segment by accepting `"true"` and `"false"` and
  rejecting everything else. One type, two doors, two answers.
- **`date`, eleven lines below.** It validates, and its comment gives the
  reason: *"a `date` this rejects is one Postgres would have rejected later,
  with a fault instead of a sentence."* A `boolean` that reaches a query as
  the wrong value is worse than one Postgres would have rejected — nothing
  rejects it at all.

The checker is the other half: `src/check.rs:2774` types `boolean(x)` as
`Ty::boolean()` for any argument, so `boolean(request.query("done"))` — a
`text?` in, a non-null `boolean` out — passes without a word.

**Proposal.** Parse `true` / `false` and raise `BadRequest` otherwise, the
way `date` does. Decide the null case explicitly and write it in
`builtins.md`: either it raises like any other bad input, or `boolean(x)`
returns `boolean?` and null-in gives null-out the way `enum(E, x)` already
promises. The second reads better at a call site — `?done=` is optional far
more often than it is required — but it is the larger change, because the
signature moves.

**Workaround today**, and what the todo app ships:

```jwc
function flag(raw: text?) -> boolean? {
    if (@raw == null) {
        return null;
    }

    return boolean(@raw);
}
```

It restores the absent case. It does not restore the bogus case — `flag("x")`
is still `false`.

---

## 2. `enum(E, x)` never reads `E`

**Class:** wrong status · **Found:** writing a `?priority=` filter

`src/exec_call.rs:60`:

```rust
if path == "enum" {
    let v = self.eval(&args[1]).await?;
    return Ok(if v.is_null() {
        Value::Null
    } else {
        Value::Text(text(&v))
    });
}
```

The type name is dropped. Whatever the client sent is passed on as text, and
the first thing that has an opinion about it is Postgres:

```
GET /api/v1/todos?priority=bogus
[fault] invalid input value for enum todo.priority: "bogus"
→ HTTP 500
```

`builtins.md §2` says the opposite in two places — the table
(*"`enum(E, x)` | `E?` — `null` in gives `null` out; a non-member raises"*)
and the prose under it (*"`enum(InvoiceStatus, request.query("status"))` is
one line and `?status=bogus` is a 400 rather than a silently dropped
filter"*). The half that is implemented is the null half.

This is not confined to a new application: the specification's own sample
answers 500 to `/api/v1/orgs/1/invoices?status=bogus` for the same reason.

The member list is not missing. `src/check.rs:2795` resolves `E` against
`self.sym.enums` to type the call at all — it is only the interpreter that
never receives it.

**Proposal.** Carry the resolved members to the runtime and raise
`BadRequest` naming the value and the members. The checker already proves
`E` exists, so the runtime arm cannot fail to find it.

**Workaround today:** none. `enum()` is the only construct in the language
that can test membership, so a program cannot defend itself here.

---

## 3. `jwc fmt --check` and `jwc fmt` report each other's outcome

**Class:** ergonomics · **Found:** running the e-school tree through CI's own commands

On a tree that needs no formatting:

```
$ jwc fmt --check .        # reads, writes nothing
ok — 25 files formatted

$ jwc fmt .                # the one that writes
ok — 25 files already formatted
```

The read-only command reports that it formatted them; the writing command
reports that it did not have to. `src/cmd/mod.rs:308` and `:327` hold the two
strings, and they are the wrong way round.

Harmless to the exit code — both are 0, and the failing side of `--check`
(*"would reformat …"*, then `N files need formatting`) is right. It costs a
reader of CI output one double-take, every time.

**Proposal.** `--check` says `ok — N files already formatted`, matching what
it checked and matching `jwc check`'s `ok — N files checked`. The writing
path keeps its own `formatted <path>` lines and gets the summary the other
one is using.

---

## 4. `jwc explain` lists no write statement

**Class:** ergonomics · **Found:** looking for e-school's `update` SQL

```
$ jwc explain .
… 67 queries
```

e-school issues 98 statements. `explain` printed 68 `SELECT`s and **none** of
the 11 `insert`, 15 `update` and 4 `delete`. `jwc --help` describes the
command as *"Print every query the program issues, with its SQL"*.

The cause is structural: `query_sql::sites()` yields `Site { select: &SelectExpr }`,
so walking a declaration can only ever reach a `select`. A write is not a
`SelectExpr` and there is nowhere to put it.

This is the one gap on this list with a demonstrated cost. rc.6 fixed a
lost-write bug whose whole substance was the shape of the `WHERE` clause a
write lowered to — `WHERE x.ctid = (SELECT y.ctid … FOR UPDATE LIMIT 1)`.
The one command whose job is to show a reader the SQL could not show them
that clause, and the defect was instead found by a load test counting 404s.

**Proposal.** Widen `Site` to carry a statement rather than a `SelectExpr`,
and give `explain` an arm per write. It is the largest item here and it is
worth sequencing on its own.

---

## 5. An absent `jwc` field means "any version"

**Class:** ergonomics · **Found:** opening MyWallet, written for 0.9.901

rc.4 added `jwcproj.json`'s `jwc` field for one episode: *"an application
written against one release, compiled by another, answering fifteen
diagnostics that were about the version gap and read as though they were
about the code."*

MyWallet's manifest has no such field — it was written before the field
existed, which is true of every project the field is for. The field is
optional and absent means no opinion, so `Workspace::load` checks nothing and
rc.6 compiles 0.9.901 source as though it were current:

```
$ jwc check .
290 errors
```

Zero of them mention a version. The episode repeats exactly, nineteen times
larger.

**Proposal.** An absent field is not the same as a satisfied one. When a
project names no version **and** the source raises the diagnostics that only
an old dialect produces (`E0900`–`E0903`), say so once, before the list:
this project's manifest names no `jwc` version, and these diagnostics are
what source written for 0.9.x looks like. `jwc new` already writes the field;
nothing offers it to a project that predates it.

---

## 6. One `--` comment produces one diagnostic per character on the line

**Class:** ergonomics · **Found:** the same 290

`E0901` is right and its help is complete:

```
error[E0901]: `--` does not start a comment
   = help: a line comment starts with `//`, a doc comment with `///`
```

But the lexer then keeps reading the line as source, so every backtick, em
dash, `§` and apostrophe in the prose becomes its own error:

```
error[E0100]: unexpected character `—`
error[E0100]: unexpected character ```
error[E0100]: unexpected character `§`
```

**89 of the 290 were this**, and all 89 sat on a line that starts with `--`.
Nearly a third of the wall was one fact, restated once per punctuation mark.

**Proposal.** Having decided a line opens with the old comment marker, skip
to the newline. One `E0901` per line, and the reader sees the 29 real ones.

---

## 7. rc.3's write binder shipped without a migration diagnostic

**Class:** ergonomics · **Found:** the 35 errors left after the comments and sigils

One changelog entry — *"BREAKING: a column names its binding, and every query
binds one"* — made two changes. `$name` → `@name` got a diagnostic that names
the fix:

```
error[E0903]: `$raw` — the sigil is `@`
   = help: write `@raw`
```

`insert into T` → `insert T into T` got this:

```
error[E0001]: expected `;`, found `into`
```

Same entry, same release, same reader. The parser error says nothing about
which release removed the form or what replaced it, and it cascades — one
write produced three or four errors as the parser resynchronised.

The `E0900`-series covers the 0.9 → 1.0 cutover well. It does not cover the
changes made *inside* the candidate series, which is where the
still-supported projects actually are. SEMVER.md promises `rc.N → rc.N+1`
carries *"a promise that the reason is written down"* — it is, in
`CHANGELOG.md`, which is not where the compiler points.

**Proposal.** A diagnostic per rc-series removal, in the shape `E0903`
already has. `insert into <table>` and `update <table> set` are both
recognisable at the parse site.

---

## 8. 116 diagnostics carried a machine-applicable fix and nothing applies them

**Class:** ergonomics · **Found:** the last wave of the MyWallet migration

With the file parsing, `E0904` arrived 103 times, each carrying its own
answer:

```
error[E0904]: `id` does not name its binding
  --> ./src/services/auth.jwc:19:16
   = help: write `Users.id`
```

A thirty-line script that reads `file:line:col` and the ``help: write `X.y` ``
string fixed all 103 in a single pass, and the tree then checked clean. The
same script, unchanged, fixed the shortener's 13. No judgement was involved
at any of the 116 sites — the compiler had already made every decision.

Of the 358 edits this migration took, **351 were mechanical**: 110 `///`, 10
`//`, 121 `@`, 12 write binders, 1 `for (let …)`, 103 qualifications, 1
`serve()`, 1 `pool_size`. Only the last two needed a person, and both were
one line.

**Proposal.** `jwc fix` — re-run the check, apply every diagnostic that
carries a literal replacement, repeat until the count stops falling, print
what it changed. The `--help` line writes itself: *the migrations the
compiler already knows how to do.* It also raises the value of every
`help:` string in `diag.rs`, because each one becomes executable.

---

## 9. `jwc fmt` cannot format a file with a comment inside `server { }`

**Class:** ergonomics · **Found:** formatting the migrated MyWallet

```
$ jwc fmt .
./src/app.jwc: not formatted — 3 comments would be lost:
    /// Required by any `page` query (config.md §3): the cursor is a
    /// client-supplied predicate and is signed so a caller cannot hand back
    /// an ordering tuple the query's own `where` was meant to exclude.
  the printer re-emits from the AST, which carries a comment on a declaration
  or a statement but not inside a record literal, a `server { }` body or an
  `insert` value list.
Error: 2 files left alone rather than lose a comment
```

The refusal is the right call — losing a comment silently would be worse —
and the message explains itself completely. The problem is what it leaves the
author: both comments are where anyone would put them. One documents
`cursor_secret` beside `cursor_secret`; the other explains a narrowing beside
the narrowing. The only remedy offered is to move them somewhere less useful,
and until then `jwc fmt --check` — which CI runs — fails on the file forever.

**Proposal.** Carry a comment on those three positions in the AST. Until
then the message should say that this is a limitation being tracked, not a
style rule the author broke.

---

## 10. `after` runs *before* the response is sent

**Class:** naming, and a gap behind it · **Found:** a reader asking what
`after` does

`after` is not a deferred hook. It is the unwind half of the middleware
onion — after the *handler chain*, before the *socket write* — and it has
to be, for three reasons the spec is right about:

- it may set response headers (§5.4), and nothing that touches the response
  can run once the bytes are gone;
- `response.status()` and `response.duration_ms()` exist nowhere else
  (`E0734`), and the status is not known until the handler **and** the
  `errorHandler` are done. jwc-shortener's 0.9.x version is the proof: with
  no `after`, it hardcoded `status = 200` for 404s and 500s and
  `latency_ms = 0`, so every percentile it reported was a zero;
- it runs for **every** outcome, including a middleware short-circuit where
  the handler never ran at all (§4.3). Nothing written in a handler covers
  that case.

So the mechanism is sound. Three things around it are not.

**The name promises the opposite of what it does.** Measured, on a
middleware whose `after` block spins a million turns:

| | latency |
|---|---|
| route with no `after` | 0.001 s |
| same route, `after` spinning 10⁶ | **5.8 s**, and `x-spun: 1000000` on the response |

With `request_timeout = "2s"` the same request is a **504 at 2.004 s** — the
200 the handler built is discarded. A block named `after` can turn a
finished response into a gateway timeout.

**Only a write can be taken off that path.** `writes.md §7.3` states the
problem in as many words — *"an ordinary insert there puts a database round
trip in front of every response: every request waits for its own log row"* —
and answers it with `buffered`. Measured on jwc-shortener:

| | median | p90 |
|---|---|---|
| `insert … buffered` | 2.3 ms | 2.8 ms |
| the same insert, plain | 3.8 ms | 4.8 ms |

`BENCHMARK.md` records 6.28 ms → 482 µs and 7 951 → 102 551 req/s on the
native backend. But `buffered` is a write form. An `http.post` to a webhook,
a `redis` write or a `mail.send` in an `after` block still sits in front of
the response, and `E0811` means each also needs a postfix `catch`.

**There is no hook that runs after the write.** Between `after` (in the
request) and `job` + `dispatch` (a durable row in `_jwc_jobs`, replayed by a
worker) there is nothing. Telemetry wants neither: the first charges the
client, and the second writes a queue row per request, which costs more than
the insert it was avoiding.

**Proposal.** Not a rename — `after` is the right word for the chain
position and every middleware system uses it. Say what it means in the one
place a reader looks first: middleware.md §5 opens with *"after the handler
chain, before the response is written"*, and the `E0811` help says so too.
Then decide whether the third point is a gap worth closing in 1.1, next to
finding 11 — they are the same question asked from two ends.

---

## 11. Nothing in the language runs two things at once inside one request

**Class:** gap — a 1.1 question, not an rc.7 fix · **Found:** reading
jwc-shortener's `/api/v1/stats`

The route runs four queries that do not depend on each other. With
`JWC_LOG_SQL=1`:

```
[sql] 4.73ms  count(*) over api_call
[sql] 2.62ms  count/sum/max over link
[sql] 1.56ms  top 5 links by hits
[sql] 1.12ms  count(*) over api_call in the last 24h
```

10.03 ms, one after another, inside a 17.2 ms request. Nothing reads another's
result — all four are combined only in the final `json({ … })`. Run together
the wall clock is the slowest of them, 4.73 ms, so **5.3 ms of a 17 ms request
is spent waiting in series for no reason**.

The language has no way to express it. `dispatch` is durable and answers
minutes later; `after` is in the same request and in front of the response;
everything else is sequential. The runtime is already multi-threaded *across*
requests — this is the gap *within* one.

Three different needs sit behind this, and they should not get one answer:

**(a) Concurrent I/O in one handler.** The shortener's case, and any handler
that calls two services. The JWC-shaped form is structured and scoped, the
way `transaction { }` already is:

```
parallel {
    let calls = select A from App.public.ApiCalls as { total: count(A.id) } first;
    let links = select L from App.public.Links   as { … } first;
    let top   = select L from App.public.Links … limit 5;
}
```

The bindings are in scope after the block; the block joins before it ends.
What makes it a design question rather than a feature request is what it has
to answer:

| | |
|---|---|
| connections | N branches want N pool connections. `pool_size = 10` with ten requests each holding one and wanting two is a deadlock, not a slowdown. A cap, or a rule that a branch borrows from a reservation the request already holds |
| inside `transaction { }` | one connection by definition, so this has to be an error there — the same shape of rule `buffered` already carries (`E0612`) |
| raise sets | two branches can raise. The language computes raise sets statically, so it can require them compatible and pick a deterministic winner — first in source order, the others cancelled |
| writes | two concurrent writes to one table in one block is a self-deadlock a reader will not see. Reads and pure computation first |

**(b) Fire-and-forget that is not durable.** Finding 10's third point. A
webhook call or a cache warm that should not charge the client and does not
deserve a `_jwc_jobs` row.

**(c) CPU work off the request.** The million-turn loop from finding 10. This
is not concurrency inside a request at all — it wants a worker, which is what
`job` already is. Worth stating explicitly so it stops being asked as though
it were the same thing as (a).

**Proposal.** None yet, deliberately — this is the one item on the list that
is a language change rather than a defect. Recorded here so the question is
written down with the measurement attached, and so rc.7's scope can be
decided knowing it is waiting. (a) has the number; (b) is the one that keeps
coming back through `after`.

---

## Migration log

What each application needed to reach rc.6 from the release it was written
for. One had only to cross rc.5 → rc.6, which carried no breaking change;
the other two had to cross the whole candidate series.

| Application | Files | Written for | Source change needed |
|---|---|---|---|
| e-school | 24 `.jwc`, 2,593 lines | rc.5 | none — the `jwc` field in `jwcproj.json` |
| MyWallet | 18 `.jwc`, 784 lines | 0.9.901 | 358 edits over 17 files, 351 of them mechanical |
| jwc-shortener | 10 `.jwc`, 404 lines | 0.9.9 | 218 edits over 11 files, 215 of them mechanical |

e-school checks clean, formats clean, applies its DDL, passes its own nine
`jwc test` cases, and serves all 62 routes: bootstrap → login → year → term
→ class → enrolment → assignment → schedule → lesson → attendance → grades
→ term grades, then the pupil's six views of the same data. Its error paths
answer 409 `TermClosed`, 403 for a foreign journal, 401 unauthenticated,
400 with a field path for each validation rule, and 400 for a tampered
cursor.

Findings 1 and 2 reproduce on it: `/admin/users?role=bogus` is a 500, and
`/admin/subjects?active=TRUE` answers `200 []` — every subject hidden by a
filter the caller did not write.

---

### What the two 0.9.x ports cost

Both had to cross the whole candidate series, and both crossed it the same
way. The counts are what findings 5 to 8 are arguing about:

| | MyWallet | shortener |
|---|---:|---:|
| errors on first `jwc check` | 290 | 312 |
| …that mention a version | 0 | 0 |
| …that are `E0100` cascade from one `--` | 89 (31%) | 207 (66%) |
| edits the compiler itself specified | 351 | 215 |
| edits needing a person | 2 | 3 |

Five edits across two applications needed judgement, and all five were one
line: `serve(port)` → `serve()` with `port` in `server { }`, and `env()` out
of `init()`. Everything else the compiler had already decided and was
already printing. Neither port surfaced a *language* gap — both services do
under rc.6 exactly what they did under 0.9.x, verified against a real
Postgres and Redis.

That is the shape of most of the list. rc.6 does not get the answers wrong,
except in the two coercions at the top. What it gets wrong is the walk from
an older release to this one: it knows every step and makes the reader take
each one by hand.

Findings 10 and 11 are the exception, and they did not come from a
migration. They came from reading the language and asking what it can say.
11 is a language change and does not belong in rc.7 at all; it is written
down here because the measurement was in hand and the question keeps
arriving through 10.

### Suggested order

1. **1 and 2** — the only two that produce a wrong answer or a wrong status
   to a live client. Small, and they close a hole the sample shares.
2. **6** — one `continue` in the lexer removes a third to two thirds of
   what a migrating reader has to read.
3. **8** — `jwc fix`. It pays for itself on the next release that moves
   anything, and both ports above were dress rehearsals for it.
4. **5 and 7** — say which release the source is from, and name the
   rc-series removals the way the 0.9 ones are named.
5. **3 and 9** — small honesty fixes in `fmt`.
6. **4** — `jwc explain` over writes. The largest of the defects, and the
   one with a defect of its own already charged to it.
7. **10**, the documentation half only — one sentence in middleware.md §5
   and one in the `E0811` help. The gap behind it waits for 11.

Not rc.7: **11**. It is a language change with a pool-deadlock question
inside it, and the right time to decide it is when rc.7 is out and the
freeze gates are what is left. The number to hold onto is 5.3 ms of a
17 ms request, on a route that was written the only way the language
allows.
