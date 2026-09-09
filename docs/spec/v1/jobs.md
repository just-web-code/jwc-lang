# Jobs

Normative. Background work: how it is declared, how it is dispatched, and
what the runtime guarantees.

---

## 1. `job`

### 1.1 Declaration

```jwc no-compile
job SendWelcome(account_id: bigint, email: text) retries 5 backoff "30s" {
    let account = select A from App.auth.Accounts
        where id == $account_id
        first or throw NotFound("akkaunt topilmadi");

    mail.send($email, "Welcome", "<p>salom</p>");
}
```

A `job` is a top-level declaration, like a `function`. Its parameters are
its payload.

Parameters are **scalars and arrays of scalars**. A `class`, a record or a
`raw` is `E0362`: a payload is written to a table and replayed minutes or
hours later, and re-validating a request-boundary shape on the way out is
a contract nothing states. Pass the id; read the row in the handler, where
it is current.

A name declared twice is `E0363`, and the first wins — the name is the key
its queued rows carry, so two declarations mean two meanings for one row.

### 1.2 Policy

| | Default | |
|---|---|---|
| `retries N` | 5 | total attempts, including the first. Outside `1..=100` is `E0022` |
| `backoff "30s"` | 30s | the wait after a failed attempt |

`retries 1` means one attempt and no retry.

### 1.3 The body

A job body is a `service` function that answers nothing. It has its
parameters and the database. It has **no request and no response**,
because by the time it runs the request is long gone: `request.*`,
`context.*`, `@param` and the response builders are all out of scope.

A raise ends the attempt. `throw NotFound(…)` in a job is not a 404 —
there is nobody to send one to — it is a failed attempt, recorded with its
message.

---

## 2. `dispatch`

```jwc no-compile
route POST "register" {
    let account = AuthService.register($req);

    dispatch SendWelcome(account_id: $account.id, email: $account.email);

    return created(json($account));
}
```

Arguments are **named**, and checked against the declaration:

| | |
|---|---|
| unknown job | `E0364` |
| a parameter given twice | `E0366` |
| the wrong type | `E0367` |
| a name the job does not declare | `E0368` |
| a non-optional parameter left out | `E0369` |

An optional parameter left out is `null`, which is what `T?` means.

A payload passed as an untyped string is why this is a declaration
rather than a call: a handler expecting `account_id` and a caller sending
`accountId` would typecheck, run, and fail at 3am with a JSON parse error
in a worker log.

### 2.1 It is part of the transaction

The row is written on the request's connection, before the response goes
out. Inside a `transaction { }` it rolls back with everything else — which
is what makes "enqueue the email **only if** the account was created"
expressible at all. Enqueueing to a broker outside the database cannot say
that.

### 2.2 Not from a job

`dispatch` inside a job body is `E0365`. A job that dispatches jobs has no
bound on the work it creates, and the failure mode is a queue that fills
faster than it drains with nothing in the source that looks wrong.

A request can fill it too, just more slowly, which is what §3.7 bounds.

---

## 3. The queue

### 3.1 Two tables the runtime owns

`public._jwc_jobs` and `public._jwc_jobs_dead`, created at boot the way
`_jwc_migrations` is, with `IF NOT EXISTS` so every replica can run it.

They are deliberately **not** part of the declared schema: `jwc migrate
new` would want to diff them, `jwc migrate down` would want to drop them,
and a snapshot would carry rows of pending work as if they were schema.

### 3.2 Durable only

There is one driver and it is the database the program already has.

There is no in-memory alternative, and no switch to pick one. A queue
that can lose every pending job on deploy has no guarantee anyone can
build on, and the loss is invisible — the enqueue succeeded, the work
simply never happened.

### 3.3 At-least-once

A worker claims one row at a time:

```sql
UPDATE _jwc_jobs SET leased_until = now() + interval '5 minutes', attempts = attempts + 1
WHERE id = (SELECT id FROM _jwc_jobs
            WHERE run_at <= now() AND (leased_until IS NULL OR leased_until < now())
            ORDER BY run_at, id FOR UPDATE SKIP LOCKED LIMIT 1)
RETURNING …
```

`SKIP LOCKED` is what makes a second worker walk past a row the first is
taking rather than block on it. A worker that dies mid-job leaves
`leased_until` in the past, and the next poll picks the job up again.

That is at-least-once, which is the only delivery guarantee a queue on a
database can actually make, and it has a consequence worth stating
plainly: **a handler must tolerate running twice.** Deleting a row it
already deleted is fine. Charging a card twice is not, and the fix is an
idempotency key in the handler, not a stronger promise here.

### 3.4 Failure

An attempt that raises is retried after `backoff`. The attempt that
exhausts `retries` moves the job to `_jwc_jobs_dead` with its last error
and is not retried again.

A queued row whose `job` declaration is gone — a deploy that dropped one
while rows were still waiting — is dead-lettered rather than retried
forever.

### 3.5 Workers

| Env var | Default | |
|---|---|---|
| `JWC_JOB_WORKERS` | 2 | worker tasks per process; `0` = this process does not drain |
| `JWC_JOB_POLL_MS` | 1000 | poll interval when the queue is empty |

A program that declares no `job` starts no workers and creates no tables.

### 3.6 `/metrics`

`jwc_jobs_pending`, `jwc_jobs_dead` (gauges), and
`jwc_jobs_processed_total`, `jwc_jobs_failed_total`, `jwc_jobs_dead_total`
(counters). Absent when the program has no jobs.

### 3.7 What bounds it

| `server { }` key | Default | |
|---|---|---|
| `job_max_payload` | 65536 | biggest payload one `dispatch` may write, in bytes of JSON |
| `job_queue_limit` | 10000 | how many jobs may be waiting before `dispatch` is refused |

`0` disables either, meaning what it means for `max_sockets` and
`max_body_bytes`: the deployment has something else doing this.

A job payload is built from request data and then **sits in a table**.
`max_body_bytes` bounds the request; until 0.9.951 nothing bounded what a
handler carried out of one into the queue, and nothing bounded how many
rows accumulated. Measured at the 1 MB default body cap, against a queue
with no workers draining it: twenty requests put **20 MB** of
incompressible payload into `_jwc_jobs`. That storage is durable and
shared — the database filling is every table failing, not only this one,
and §2.2 already refuses `dispatch` from a job body on exactly this
argument while leaving the request path open.

Both refusals are **faults**, and neither is silent. Nothing in the source
raised them, so no `catch` can name them; the detail goes to the log and
the caller gets the ordinary `internal_error` (security §6.3). The
enqueue's transaction rolls back with the request's, which is the point:
§3.2 rules out a queue that can lose a job invisibly, and quietly dropping
an enqueue at the limit is that same loss written on a different line.

The depth test runs **inside** the insert. Counting first and inserting
second lets two requests both see room and both take it.

---

## 4. Diagnostics introduced here

| Code | |
|---|---|
| `E0022` | `retries N` outside `1..=100` |
| `E0362` | a job parameter that is not a scalar or an array of scalars |
| `E0363` | two `job`s with one name |
| `E0364` | `dispatch` of an undeclared job |
| `E0365` | `dispatch` inside a job body |
| `E0366` | a parameter given twice at a dispatch site |
| `E0367` | a dispatch argument of the wrong type |
| `E0368` | a dispatch argument the job does not declare |
| `E0369` | a non-optional parameter left out |
