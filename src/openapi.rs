//! `jwc openapi` — an OpenAPI 3.1 document, derived and never authored.
//!
//! Every part of it already exists in the compiler: the route table
//! (routing.md §5), typed path parameters (§3.1), the `class` a route
//! validates its body against (types.md §4.1), the type of each response
//! builder's payload, and the raise set that decides which non-2xx statuses
//! a route can produce (errors.md §3, §4.3). This module arranges them; it
//! infers nothing of its own.
//!
//! ## Two rules that make the document truthful
//!
//! **A `Raw` response has no schema.** It is emitted as
//! `application/json` with no `schema` at all, because the compiler did not
//! check that shape either (types.md §5.1). Writing a plausible object there
//! would be the document asserting something the type system refused to.
//!
//! **Scalars map to their wire form, not their Postgres form** (types.md
//! §2.3). `bigint` and `numeric` are `{"type": "string"}` because that is
//! what the runtime sends — JavaScript loses digits above 2^53, and no float
//! ever touches money.

use crate::check::{Checked, RouteResponse};
use crate::symbols::{ClassSym, Symbols};
use crate::types::{Scalar, Ty};
use crate::wiring::{ResolvedRoute, Wired};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};

pub struct Input<'a> {
    pub title: String,
    pub version: String,
    pub sym: &'a Symbols,
    pub wired: &'a Wired,
    pub checked: &'a Checked,
    /// Per route, the declared errors it can raise (errors.md §3).
    pub raises: BTreeMap<String, Vec<String>>,
    /// Middleware that verifies a bearer token, so a route behind one is
    /// marked `security` and a client knows to send `Authorization`.
    pub bearer_middleware: BTreeSet<String>,
}

/// The document, from artifacts a caller already has.
///
/// `jwc openapi`, `jwc swagger` and the reference `server { swagger }`
/// serves all come through here. That is the whole point: the 0.9 command
/// was a second generator, 661 lines, kept in step with the first by hand,
/// and a page that disagrees with the document beside it is worse than no
/// page. One walk, three callers.
pub fn document_for(
    ws: &crate::workspace::Workspace,
    database: Option<&str>,
    sym: &Symbols,
    checked: &Checked,
    wired: &Wired,
    title: Option<String>,
) -> Value {
    // Which declared errors each route can raise. errors.md §4.3 makes a
    // declared error's default status the answer whether or not an
    // `errorHandler` arm names it, so this is exactly the non-2xx set.
    let bodies = crate::wiring::function_bodies(ws);
    let mut raises: BTreeMap<String, Vec<String>> = Default::default();
    for file in &ws.files {
        for d in &file.program.decls {
            let crate::ast::Decl::Routes(r) = d else {
                continue;
            };
            for rt in &r.routes {
                let key = format!(
                    "{} {}",
                    rt.method.name.to_uppercase(),
                    crate::wiring::route_pattern(&r.prefix, &rt.suffix)
                );
                let mut set: Vec<String> = crate::wiring::raises_from(sym, &bodies, &rt.body)
                    .into_iter()
                    .collect();
                // Middleware runs before the handler and can answer on its
                // own, so what it raises the route can produce.
                for m in rt.uses.iter().chain(&r.uses) {
                    if let Some(b) = middleware_body(ws, &m.name) {
                        set.extend(crate::wiring::raises_from(sym, &bodies, b));
                    }
                }
                set.sort();
                set.dedup();
                raises.insert(key, set);
            }
        }
    }

    // Which middleware authenticates. `jwt.*` is the signal: the only way
    // a program verifies a token it did not mint itself is to call one of
    // them, and the chain is already recorded per route. Without this the
    // document says nothing about `Authorization`, so a generated client
    // and the reference page both offer a call that cannot succeed.
    let mut bearer_middleware: BTreeSet<String> = Default::default();
    for file in &ws.files {
        for d in &file.program.decls {
            let crate::ast::Decl::Middleware(m) = d else {
                continue;
            };
            let mut reached = crate::wiring::callees(&m.body);
            // A middleware that delegates to `AuthService.verify(...)` is
            // the same middleware; the call graph is what says so.
            for name in reached.clone() {
                if let Some(b) = bodies.get(&name) {
                    reached.extend(crate::wiring::callees(b));
                }
            }
            if reached.iter().any(|c| c.starts_with("jwt.")) {
                bearer_middleware.insert(m.name.name.clone());
            }
        }
    }

    document(&Input {
        bearer_middleware,
        title: title.unwrap_or_else(|| {
            database
                .map(str::to_string)
                .unwrap_or_else(|| "JWC application".to_string())
        }),
        version: "1.0.0".to_string(),
        sym,
        wired,
        checked,
        raises,
    })
}

fn middleware_body<'a>(
    ws: &'a crate::workspace::Workspace,
    name: &str,
) -> Option<&'a crate::ast::Block> {
    ws.files.iter().find_map(|f| {
        f.program.decls.iter().find_map(|d| match d {
            crate::ast::Decl::Middleware(m) if m.name.name == name => Some(&m.body),
            _ => None,
        })
    })
}

pub fn document(input: &Input) -> Value {
    let mut paths: Map<String, Value> = Map::new();
    let mut used: BTreeSet<String> = BTreeSet::new();
    let mut secured = false;
    let mut tags: BTreeSet<String> = BTreeSet::new();

    // OpenAPI 3.1 has no way to describe a WebSocket, and emitting the
    // upgrade as a `GET` that answers 200 would be a lie a client
    // generator acts on. They are listed under `x-jwc-sockets` instead,
    // so a reader of the document can at least see they exist.
    let mut routes: Vec<&ResolvedRoute> = input.wired.routes.iter().filter(|r| !r.socket).collect();
    routes.sort_by(|a, b| (&a.pattern, &a.method).cmp(&(&b.pattern, &b.method)));

    for r in routes {
        let key = format!("{} {}", r.method, r.pattern);
        let mut op: Map<String, Value> = Map::new();
        op.insert("operationId".into(), json!(operation_id(r)));
        let tag = tag_of(&r.pattern);
        tags.insert(tag.clone());
        op.insert("tags".into(), json!([tag]));

        if !r.params.is_empty() {
            let params: Vec<Value> = r
                .params
                .iter()
                .map(|(name, ty)| {
                    json!({
                        "name": name,
                        "in": "path",
                        "required": true,
                        "schema": scalar_schema(ty),
                    })
                })
                .collect();
            op.insert("parameters".into(), json!(params));
        }

        if let Some((_, class)) = input
            .checked
            .request_bodies
            .iter()
            .find(|(route, _)| route == &key)
        {
            used.insert(class.clone());
            op.insert(
                "requestBody".into(),
                json!({
                    "required": true,
                    "content": { "application/json": { "schema": reference(class) } },
                }),
            );
        }

        let mut responses: Map<String, Value> = Map::new();
        for resp in input.checked.responses.iter().filter(|x| x.route == key) {
            let (body, mut named) = response_body(input.sym, resp);
            used.append(&mut named);
            responses.insert(resp.status.to_string(), body);
        }

        // Everything the route can raise, whether or not an `errorHandler`
        // arm names it: a declared error's default status is what makes an
        // arm optional (errors.md §4.3), so the status is known either way.
        for name in input.raises.get(&key).into_iter().flatten() {
            let Some(e) = input.sym.errors.get(name) else {
                continue;
            };
            responses
                .entry(e.status.to_string())
                .or_insert_with(|| error_response(name));
        }
        if responses.is_empty() {
            responses.insert("200".into(), json!({ "description": "OK" }));
        }
        op.insert("responses".into(), Value::Object(responses));

        if !r.chain.is_empty() {
            // Not an OpenAPI concept, but the chain is what decides whether
            // a call needs a token, and a reader of the document has no
            // other way to find out.
            op.insert(
                "x-jwc-middleware".into(),
                json!(r.chain.iter().collect::<Vec<_>>()),
            );
            // The part of it OpenAPI *does* model. An empty `security` on
            // the operation would mean "no auth" and override a document
            // default, so it is written only where it is true.
            if r.chain.iter().any(|m| input.bearer_middleware.contains(m)) {
                secured = true;
                op.insert("security".into(), json!([{ "bearerAuth": [] }]));
            }
        }

        let entry = paths.entry(r.pattern.clone()).or_insert_with(|| json!({}));
        if let Some(o) = entry.as_object_mut() {
            o.insert(r.method.to_lowercase(), Value::Object(op));
        }
    }

    // Schemas are emitted for what the paths actually reference, plus
    // whatever those reference in turn.
    let mut schemas: Map<String, Value> = Map::new();
    let mut queue: Vec<String> = used.into_iter().collect();
    while let Some(name) = queue.pop() {
        if schemas.contains_key(&name) {
            continue;
        }
        let Some(class) = input.sym.classes.get(&name) else {
            continue;
        };
        let (schema, refs) = class_schema(input.sym, class);
        schemas.insert(name, schema);
        queue.extend(refs);
    }

    let mut doc = Map::new();
    doc.insert("openapi".into(), json!("3.1.0"));
    doc.insert(
        "info".into(),
        json!({ "title": input.title, "version": input.version }),
    );
    doc.insert("paths".into(), Value::Object(paths));

    // Declared, not left implicit. A reader of the document gets the order
    // and a renderer gets one section per group; without this every
    // operation lands under `default` and a service of any size is one
    // flat list.
    if !tags.is_empty() {
        doc.insert(
            "tags".into(),
            json!(tags
                .iter()
                .map(|t| json!({ "name": t }))
                .collect::<Vec<_>>()),
        );
    }

    // Not `paths`: OpenAPI cannot model them, and a reader who cannot see
    // them at all concludes the service has no sockets.
    let sockets: Vec<Value> = input
        .wired
        .routes
        .iter()
        .filter(|r| r.socket)
        .map(|r| {
            json!({
                "path": r.pattern,
                "middleware": r.chain.iter().collect::<Vec<_>>(),
            })
        })
        .collect();
    if !sockets.is_empty() {
        doc.insert("x-jwc-sockets".into(), json!(sockets));
    }

    let mut components: Map<String, Value> = Map::new();
    if !schemas.is_empty() {
        components.insert("schemas".into(), Value::Object(schemas));
    }
    if secured {
        components.insert(
            "securitySchemes".into(),
            json!({
                "bearerAuth": {
                    "type": "http",
                    "scheme": "bearer",
                    "bearerFormat": "JWT",
                }
            }),
        );
    }
    if !components.is_empty() {
        doc.insert("components".into(), Value::Object(components));
    }
    Value::Object(doc)
}

/// The group an operation is listed under.
///
/// The first literal segment that is not a version marker: `/api/v1/admin/
/// users` is `admin`, `/api/v1/me` is `me`. JWC has no `tag` keyword and
/// the `routes` prefix is the only grouping an author actually writes, so
/// this reads it back rather than inventing a second one.
///
/// `api` and `v1` are skipped because every route in a versioned service
/// carries them, and a tag every operation shares groups nothing.
fn tag_of(pattern: &str) -> String {
    for seg in pattern.split('/') {
        if seg.is_empty() || seg.starts_with('{') {
            continue;
        }
        let lower = seg.to_lowercase();
        let version = lower.starts_with('v')
            && lower.len() > 1
            && lower[1..].chars().all(|c| c.is_ascii_digit());
        if lower == "api" || version {
            continue;
        }
        return seg.to_string();
    }
    // `/{code}` and `/` have nothing to group by, and OpenAPI's own word
    // for that is `default`.
    "default".to_string()
}

/// `getApiV1OrgsOrgIdInvoices` — stable across runs, and unique because the
/// route table already refuses two routes of the same shape (routing §7).
fn operation_id(r: &ResolvedRoute) -> String {
    let mut out = r.method.to_lowercase();
    let mut upper = true;
    for c in r.pattern.chars() {
        if c == '/' || c == '{' || c == '}' || c == '-' || c == '_' {
            upper = true;
            continue;
        }
        if upper {
            out.extend(c.to_uppercase());
            upper = false;
        } else {
            out.push(c);
        }
    }
    out
}

fn reference(name: &str) -> Value {
    json!({ "$ref": format!("#/components/schemas/{name}") })
}

fn error_response(name: &str) -> Value {
    json!({
        "description": name,
        "content": { "application/json": { "schema": {
            "type": "object",
            "properties": {
                "error": { "type": "string" },
                "message": { "type": "string" },
            },
        } } },
    })
}

/// The `content` for one recorded response, plus any class it names.
fn response_body(sym: &Symbols, r: &RouteResponse) -> (Value, BTreeSet<String>) {
    let mut named = BTreeSet::new();
    if matches!(r.payload, Ty::Void) {
        return (json!({ "description": describe(r.status) }), named);
    }
    // A `content(mime, body)` route: the media type is known and the body
    // is text, so there is a type to name and no schema to give.
    if let Some(media) = &r.media {
        return (
            json!({
                "description": describe(r.status),
                "content": { media.clone(): { "schema": { "type": "string" } } },
            }),
            named,
        );
    }
    match schema(sym, &r.payload, &mut named) {
        // tooling.md §5.3 — a `Raw` response is emitted with no schema.
        // The compiler did not check that shape either.
        None => (
            json!({
                "description": describe(r.status),
                "content": { "application/json": {} },
            }),
            named,
        ),
        Some(s) => (
            json!({
                "description": describe(r.status),
                "content": { "application/json": { "schema": s } },
            }),
            named,
        ),
    }
}

fn describe(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        409 => "Conflict",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        _ => "Response",
    }
}

/// `None` when the type carries no checked shape — `Raw`, and the error
/// recovery type.
fn schema(sym: &Symbols, ty: &Ty, named: &mut BTreeSet<String>) -> Option<Value> {
    match ty {
        Ty::Raw | Ty::Unknown | Ty::Null | Ty::Response | Ty::Void => None,
        Ty::Optional(inner) => schema(sym, inner, named),
        Ty::Array(inner) => Some(match schema(sym, inner, named) {
            Some(items) => json!({ "type": "array", "items": items }),
            None => json!({ "type": "array" }),
        }),
        Ty::Class(name) => {
            named.insert(name.clone());
            Some(reference(name))
        }
        Ty::Enum(name) => Some(match sym.enums.get(name) {
            Some(e) => json!({ "type": "string", "enum": e.members }),
            None => json!({ "type": "string" }),
        }),
        Ty::Scalar(s) => Some(scalar(*s)),
        Ty::Record(fields) => {
            let mut props = Map::new();
            let mut required = Vec::new();
            for (name, ft) in fields.iter() {
                if !ft.is_optional() {
                    required.push(name.clone());
                }
                props.insert(
                    name.clone(),
                    schema(sym, ft, named).unwrap_or_else(|| json!({})),
                );
            }
            let mut out = Map::new();
            out.insert("type".into(), json!("object"));
            out.insert("properties".into(), Value::Object(props));
            if !required.is_empty() {
                out.insert("required".into(), json!(required));
            }
            Some(Value::Object(out))
        }
    }
}

fn class_schema(sym: &Symbols, class: &ClassSym) -> (Value, Vec<String>) {
    let mut props = Map::new();
    let mut required = Vec::new();
    let refs = Vec::new();
    for f in &class.fields {
        // `transient` is validated and never stored (types.md §4.3), but it
        // is still part of the request body, which is what this schema
        // describes.
        if !f.ty.is_optional() {
            required.push(f.name.clone());
        }
        props.insert(f.name.clone(), class_field_schema(sym, f));
    }
    let mut out = Map::new();
    out.insert("type".into(), json!("object"));
    out.insert("properties".into(), Value::Object(props));
    if !required.is_empty() {
        out.insert("required".into(), json!(required));
    }
    (Value::Object(out), refs)
}

/// A class field carries its validation rules, and several of them have an
/// exact JSON Schema spelling. Emitting them makes the document able to
/// reject what the server would reject.
fn class_field_schema(sym: &Symbols, f: &crate::symbols::ClassFieldSym) -> Value {
    let base = match &f.ty {
        Ty::Optional(inner) => &**inner,
        other => other,
    };
    let mut out = match base {
        Ty::Scalar(s) => scalar(*s),
        // An enum's members are the whole point of the type: a document
        // that says `string` lets a caller send `superadmin` and find out
        // from a 400.
        Ty::Enum(name) => match sym.enums.get(name) {
            Some(e) => json!({ "type": "string", "enum": e.members }),
            None => json!({ "type": "string" }),
        },
        Ty::Array(_) => json!({ "type": "array" }),
        _ => json!({}),
    };
    let Some(o) = out.as_object_mut() else {
        return out;
    };
    for r in &f.rules {
        let (rule, args) = (r.name.as_str(), &r.args);
        let n = args.first().and_then(number_literal);
        match (rule, n) {
            ("minLength", Some(v)) => {
                o.insert("minLength".into(), v);
            }
            ("maxLength", Some(v)) => {
                o.insert("maxLength".into(), v);
            }
            ("min", Some(v)) => {
                o.insert("minimum".into(), v);
            }
            ("max", Some(v)) => {
                o.insert("maximum".into(), v);
            }
            ("pattern", _) => {
                if let Some(p) = args.first().and_then(string_literal) {
                    o.insert("pattern".into(), json!(p));
                }
            }
            _ => {}
        }
    }
    out
}

/// JSON Schema's `minLength` is an integer and its `minimum` is a number.
/// `2.0` where `2` was written is valid JSON and wrong-looking in every
/// generated client.
fn number_literal(e: &crate::ast::Expr) -> Option<Value> {
    match &*e.kind {
        crate::ast::ExprKind::Int(n) => n.parse::<i64>().ok().map(|v| json!(v)),
        crate::ast::ExprKind::Decimal(n) => n.parse::<f64>().ok().map(|v| json!(v)),
        _ => None,
    }
}

fn string_literal(e: &crate::ast::Expr) -> Option<String> {
    match &*e.kind {
        // `pattern(r"…")` is a raw string; the regex is its text.
        crate::ast::ExprKind::Str(s) | crate::ast::ExprKind::RawStr(s) => Some(s.clone()),
        _ => None,
    }
}

/// A path parameter's declared type, which routing.md §3.1 restricts to a
/// small set.
fn scalar_schema(name: &str) -> Value {
    match Scalar::from_name(name) {
        Some(s) => scalar(s),
        None => json!({ "type": "string" }),
    }
}

/// types.md §2.3 — the wire form. `bigint` and `numeric` are JSON strings.
fn scalar(s: Scalar) -> Value {
    match s {
        Scalar::Smallint | Scalar::Int => json!({ "type": "integer" }),
        Scalar::Bigint => json!({ "type": "string", "format": "int64" }),
        Scalar::Numeric => json!({ "type": "string", "format": "decimal" }),
        Scalar::Boolean => json!({ "type": "boolean" }),
        Scalar::Varchar | Scalar::Text => json!({ "type": "string" }),
        Scalar::Timestamptz => json!({ "type": "string", "format": "date-time" }),
        Scalar::Date => json!({ "type": "string", "format": "date" }),
        Scalar::Time => json!({ "type": "string", "format": "time" }),
        Scalar::Interval => json!({ "type": "string" }),
        Scalar::Uuid => json!({ "type": "string", "format": "uuid" }),
        Scalar::Jsonb => json!({}),
        Scalar::Inet => json!({ "type": "string" }),
        Scalar::Bytea => json!({ "type": "string", "format": "byte" }),
    }
}
