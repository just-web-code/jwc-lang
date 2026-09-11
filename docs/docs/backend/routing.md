---
sidebar_position: 1
title: "Routing"
description: "Routes, typed path parameters, responses. What the router matches, and why a malformed parameter is a 400 before any middleware runs."
---

# Routing

```jwc
namespace routes.notes;

routes "/api/notes" {
    route GET "" {
        return json({ ok: true });
    }

    route GET "{id: bigint}" {
        return json({ id: @id });
    }
}

function main() {
    serve();
}
```

A `routes` block declares a prefix; a `route` inside it declares a method
and a suffix. Blocks do not nest.

A `routes` block also holds [`socket`](./sockets) declarations, which share
the prefix and the middleware chain — the upgrade is a `GET`, so a `route
GET` on the same path is a duplicate.

## Path parameters are typed

```jwc no-compile
route GET "{id: bigint}" { … }
```

`@id` is the parameter, and it arrives as a `bigint` — not as text that
the query layer will try to cast. A segment that does not parse as its
declared type is a **400 before any middleware runs**:

```json
{"error":"bad_path_parameter","parameter":"id","expected":"bigint"}
```

which is the honest answer, and keeps malformed input out of Postgres
where it would have been a 500.

`@name` covers all three: a local, a function parameter and a path
parameter. One sigil, because a `let` may not shadow a parameter, so the
name decides which it is. Inside a query clause `@` is required — a column
there is `T.column` — and everywhere else `account` and `@account` are the
same reference.

## Which route wins

The route with the most **literal** segments. `/notes/{code}` and
`/notes/docs` both match `/notes/docs`; `docs` is a literal, so it wins.
Registration order does not enter into it.

Two routes with the same shape — same literals in the same places,
parameters wherever the other has one — are a duplicate, and a hard error
at startup rather than a coin flip at run time.

## Responses

| Builder | Status |
|---|---|
| `json(v)` | 200 |
| `created(v)` | 201 |
| `accepted(v)` | 202 |
| `noContent()` | 204, no body and no content-type |
| `badRequest(v)` | 400 — takes a **value**, which becomes the body |
| `unauthorized(msg)` | 401 |
| `forbidden(msg)` | 403 |
| `notFound(msg)` | 404 |
| `conflict(msg)` | 409 |
| `tooManyRequests(msg)` | 429 |
| `internalError()` | 500, one fixed message |
| `statusCode(n, v)` | any |
| `redirect(n, url)` | any 3xx, with `Location` |
| `content(mime, body)` | 200, body verbatim |
| `text(body)` | 200, `text/plain; charset=utf-8` |
| `html(body)` | 200, `text/html; charset=utf-8` |

A builder applied to something that is **already** a response replaces its
status and keeps its body — so `created(json(row))` is 201 with that
body, not 201 wrapping a response object.

`content` is the one that does not JSON-encode. An HTML page through
`json()` reaches the browser as a quoted string.

```jwc no-compile
return content("text/html", page);
```

## Headers

```jwc no-compile
return created(json(w)) with { "Location": "/wallets/" + string.of(w.id) };
```

`with { … }` **replaces** a header of the same name rather than appending
one. A builder has already stamped `content-type`, and two of them is a
malformed message that clients resolve inconsistently — so
`with { "Content-Type": … }` has to win, or it does nothing an author can
rely on.

`cookie(name, value)` is the append form, because `Set-Cookie` legitimately
repeats.

## Request input

| | |
|---|---|
| `request.body() as <Class>` | the validated body. The cast is what validates — see [Validation](./validation.md). |
| `request.header(name)` | `text?` |
| `request.query(name)` | `text?` |
| `request.query_all(name)` | every value, in order |
| `request.method()`, `request.path()` | as sent |
| `request.route()` | the **declared** pattern, `/orgs/{org_id}` |
| `request.client_ip()` | the forwarded chain, walked against `trusted_proxies` |
| `request.raw_body()` | the bytes, unparsed |

`request.route()` is the one to key a rate limit on: `request.path()`
buckets by every distinct id.

## Where the port stops

`server { port }` says where the listener binds, and `serve()` in `main`
says that it binds at all:

```jwc no-compile
server { port = 8080; }

function main() {
    serve();
}
```

The environment wins over the declared value, the way it does for every
other key in the block: `--port`, then `JWC_PORT`, then `PORT`, then
`server { port }`, then 8080. `PORT` unprefixed because that is the name a
platform injects.
