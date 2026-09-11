<p align="center">
  <img src="https://raw.githubusercontent.com/just-web-code/jwc-lang/main/vscode-extension/icon.png" alt="JWC" width="128">
</p>

# JWC Language for VS Code

Syntax highlighting, snippets and language-server diagnostics for
[JWC (Just Web Code)](https://jwc.1kb.uz) — a backend language with
first-class routes, tables and views over Postgres.

## Features

### Syntax highlighting

Every `.jwc` construct in [the v1 grammar](https://jwc.1kb.uz): declarations
(`database`, `schema`, `table`, `view`, `enum`, `class`, `error`, `service`,
`middleware`, `routes`, `route`, `socket`, `errorHandler`, `server`, `job`,
`test`), the query clauses, the scalar dictionary, the validation rules, and
the two sigils that are easy to misread without colour — `$local` for a
binding and `@name` for a path parameter.

Comments are `//` for a line and `///` for a doc comment, which attaches to
the declaration, column or field below it.

### Diagnostics

The language server checks the buffer on every keystroke, not on save, so a
diagnostic is about the text on screen. Each one carries the same code the
compiler prints — `E0305`, `W0102` — and the same message.

### Hover, Go to Definition, completion, signature help

- Hover a table, class, enum, service or function for its declaration.
- `F12` (or `Ctrl`+click) jumps to it.
- Completion after `.` offers the members of what precedes it —
  `request.`, `date.`, `string.`, a service, a projected row — and elsewhere
  the names in scope.
- Signature help fires inside `(` and on each `,`.

### Snippets

One per declaration and per query shape:

`namespace`, `import`, `database`, `schema`, `table`, `enum`, `class`,
`error`, `service`, `function`, `middleware`, `routes`, `routes-use`,
`route-get`, `route-get-id`, `route-post`, `route-patch`, `route-delete`,
`socket`, `errorHandler`, `server`, `sel-page`, `sel-first`, `sel-where`,
`insert`, `update`, `delete`, `transaction`, `test`, `job`, `main`.

Every one of them is compiled by the toolchain's own test suite, so a
snippet cannot drift from the language it expands into.

## Requirements

Install the `jwc` toolchain:

```bash
curl -fsSL https://raw.githubusercontent.com/just-web-code/jwc-lang/main/install.sh | bash
```

The language server is a subcommand of the compiler — `jwc lsp` — so the
server and the checker are always the same build. The extension looks for
`jwc` on `PATH`, then under `~/.jwc/bin`. Without it you still get
highlighting and snippets.

## Settings

- `jwc.lspPath` — path to the `jwc` executable. Leave empty to search.
- `jwc.trace.server` — `off` / `messages` / `verbose` LSP trace.

## Commands

- `JWC: Restart Language Server`
- `JWC: Show Language Server Output`

## Build from source

```bash
npm install
npm run compile
npm run package
```
