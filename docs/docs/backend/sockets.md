---
sidebar_position: 5
title: WebSockets
description: "Declaring a socket, the three handlers, and what middleware does before the upgrade."
---

# WebSockets

A `socket` sits inside a `routes` block, beside the HTTP routes, and shares
the prefix and the `use` chain.

```jwc no-compile
routes "/live" use RequireAuth {
    socket "rooms/{room: text}" {
        on open {
            socket.send("joined " + @room);
        }

        on message (text) {
            socket.send("echo: " + text);
        }

        on close {
            // runs however the connection ended
        }
    }
}
```

All three blocks are optional; a `socket` with none of them does not
compile, because it would accept the upgrade and then do nothing.

## The runtime owns the loop

There is no `while` in JWC, and a socket handler does not need one. You do
not write a receive loop — you say what happens at the three moments a
connection has, and the runtime drives it.

That is not only a syntax convenience. A hand-written receive loop that
forgets to break holds a task for the life of the process, and that is the
single most common WebSocket bug there is.

## Middleware runs before the upgrade

This is the reason `use` on a socket is worth anything:

```jwc no-compile
middleware RequireAuth provides account_id: bigint {
    let header = request.header("Authorization") or throw Unauthorized("a bearer token is required");
    // …
}
```

A client with no token gets **`401` with that message**, as an ordinary
HTTP response. It does not get a `101` followed by an immediate close it
has to guess about.

Whatever the chain puts in `context` is readable in all three handlers and
persists for the connection. Locals do not: each handler runs on its own
scope, and `context` is the state that is meant to survive.

## `socket.send` and `socket.close`

Both are legal only inside one of the three handlers — anywhere else is a
compile error, not a runtime fault.

Both **queue**. The connection writes what a handler produced once that
handler returns, so:

```jwc no-compile
on message (text) {
    if (text == "bye") {
        socket.close();
    }
    socket.send("this never goes");   // the close came first
}
```

A handler that raises ends the connection: there is no response to put an
error in, and closing is the only signal the protocol has.

## What the wire does

| | |
|---|---|
| The upgrade | a `GET`, so `route GET` on the same path is a duplicate and does not compile |
| A plain `GET` at a socket path | `400` — the path exists, the request is wrong |
| A binary frame | closes the connection; `on message (m)` binds `text` |
| A text frame with no `on message` | dropped — a peer that speaks first is not an error |
| `after` blocks, after a successful upgrade | do not run: they observe a response, and the response was the `101` |
| `after` blocks, when middleware answers | run, in reverse order, for every middleware that started — the client got an ordinary response, and a rejected upgrade is what an access log is for |

## How many, and how long

Two `server { }` keys bound sockets, and they answer different questions.

| Key | Default | What it does |
|---|---|---|
| `max_sockets` | half the descriptor limit, clamped to [64, 4096]; 512 on Windows | past it the upgrade is `503`, answered before the handshake so the descriptor is never spent |
| `socket_keepalive` | `"30s"` | pings a quiet connection; a peer that has not answered by the next tick is dropped and its slot returned |

The second exists because the first is not enough on its own. A cap on how
many connections may be open says nothing about how long a **dead** one
stays open — `socket.recv()` waits with no timeout, so a peer that
disappeared without closing (a lid shut, a NAT entry expired, a cable
pulled) held its slot until the kernel gave up on the TCP connection,
which for an idle socket is never. The cap then works against you: live
clients get `503` on behalf of peers that no longer exist.

Any frame counts as the answer, not only a pong — a peer that is talking
has answered the question the ping asks — so a busy connection is never at
risk. A dead one is reclaimed between one and two intervals after it dies.

```jwc
server {
    max_sockets = 5000;
    socket_keepalive = "45s";   // "0s" turns the ping off
}
```

Turn it off only when something else is doing the same job — a load
balancer that drops idle connections on its own timer, say. With no ping
and no such proxy, a dead peer keeps its slot for the life of the process.

## Tooling

`jwc routes` prints sockets as `WS`:

```
WS      /live/rooms/{room}  RequireAuth
GET     /live/health        -
```

`jwc openapi` cannot describe a WebSocket — OpenAPI has no notion of one —
so sockets are listed under `x-jwc-sockets` rather than emitted as a `GET`
that answers 200, which is a lie a client generator would act on.

Both backends run sockets: `jwc serve` and `jwc build`. They are held to
the same answers as every other route.

## Server-Sent Events

Not implemented — deliberately absent rather than half-wired. A
transport you can declare, pass every check against, and then serve
nothing from is worse than one that is not there. Use a `socket`, or
long-polling.
