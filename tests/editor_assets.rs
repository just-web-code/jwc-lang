//! The editor assets, against the language they claim to describe.
//!
//! The VS Code extension ships two hand-maintained copies of the grammar: a
//! snippet set and a TextMate grammar. Nothing linked either to the
//! compiler, so both went on describing the pre-v1 language — `entity`,
//! `dbcontext`, `//` comments — through a whole major version while every
//! other suite stayed green. These tests are that link.

use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read_json(rel: &str) -> Value {
    let path = repo_root().join(rel);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// names.md §2.8. Only the words that are never anything else: `group`,
/// `new` and `patch` are all still ordinary text somewhere.
const REMOVED: &[&str] = &[
    "entity",
    "dbcontext",
    "dome",
    "mount",
    "nav",
    "via",
    "autoincrement",
];

fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase())
        .collect()
}

fn collect_matches(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Object(map) => {
            for (k, v) in map {
                match (k.as_str(), v.as_str()) {
                    ("match" | "begin" | "end", Some(s)) => out.push(s.to_string()),
                    _ => collect_matches(v, out),
                }
            }
        }
        Value::Array(items) => items.iter().for_each(|v| collect_matches(v, out)),
        _ => {}
    }
}

#[test]
fn no_editor_asset_names_a_removed_keyword() {
    let snippets =
        std::fs::read_to_string(repo_root().join("vscode-extension/snippets/jwc.code-snippets"))
            .expect("read");
    let grammar = read_json("vscode-extension/syntaxes/jwc.tmLanguage.json");

    // Only what the grammar *matches* counts: `entity.name.type` is
    // TextMate's own scope vocabulary, not the JWC keyword.
    let mut haystack = vec![snippets];
    collect_matches(&grammar, &mut haystack);

    for text in &haystack {
        let found: Vec<&str> = REMOVED
            .iter()
            .copied()
            .filter(|k| words(text).iter().any(|w| w == k))
            .collect();
        assert!(
            found.is_empty(),
            "an editor asset still names {found:?} — removed in names.md §2.8"
        );
    }
}

#[test]
fn the_grammar_highlights_the_comment_the_lexer_reads() {
    // A grammar painting a comment syntax the lexer does not read is worse
    // than no grammar: the comments in the file go unpainted, and text the
    // compiler rejects is painted as a comment.
    let g = read_json("vscode-extension/syntaxes/jwc.tmLanguage.json");
    let patterns = g["repository"]["comments"]["patterns"]
        .as_array()
        .expect("comments rule");
    let matches: Vec<&str> = patterns
        .iter()
        .filter_map(|p| p["match"].as_str())
        .collect();
    assert!(
        matches.iter().any(|m| m.starts_with("///")),
        "no doc-comment rule for `///`: {matches:?}"
    );
    assert!(
        matches
            .iter()
            .any(|m| m.starts_with("//") && !m.starts_with("///")),
        "no line-comment rule for `//`: {matches:?}"
    );
    assert!(
        !matches.iter().any(|m| m.starts_with("--")),
        "a `--` comment rule outlived the syntax: {matches:?}"
    );
    // `///` is a prefix of `//`, so the order decides which one wins.
    let doc = matches.iter().position(|m| m.starts_with("///"));
    let line = matches
        .iter()
        .position(|m| m.starts_with("//") && !m.starts_with("///"));
    assert!(
        doc < line,
        "`//` is tried before `///`, so no doc comment is ever matched"
    );
}

#[test]
fn every_grammar_include_names_a_rule_that_exists() {
    // A dangling `include` is silent: the rule simply never fires. An
    // orphaned rule is the same bug seen from the other end.
    let g = read_json("vscode-extension/syntaxes/jwc.tmLanguage.json");
    let repo = g["repository"].as_object().expect("repository");

    let mut includes = Vec::new();
    collect_includes(&g, &mut includes);
    let dangling: Vec<&String> = includes.iter().filter(|i| !repo.contains_key(*i)).collect();
    assert!(dangling.is_empty(), "includes name no rule: {dangling:?}");

    // A rule reached only from inside another rule still counts as used —
    // `block-comment` includes itself, because block comments nest.
    let unused: Vec<&String> = repo.keys().filter(|k| !includes.contains(k)).collect();
    assert!(unused.is_empty(), "rules nothing includes: {unused:?}");
}

fn collect_includes(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Object(map) => {
            for (k, v) in map {
                match (k.as_str(), v.as_str()) {
                    ("include", Some(s)) => out.push(s.trim_start_matches('#').to_string()),
                    _ => collect_includes(v, out),
                }
            }
        }
        Value::Array(items) => items.iter().for_each(|v| collect_includes(v, out)),
        _ => {}
    }
}

/// Enough of a program for a fragment to be checked inside.
const PREAMBLE: &str = r#"database App : Postgres;
schema s of App;
schema notes of App;
table T of App.s {
    id bigint primary key identity;
    name varchar(80)?;
    title varchar(200);
    archived boolean default false;
    created_at timestamptz default now();
    updated_at timestamptz default now();
    index on (created_at, id);
}
table Notes of App.notes {
    id bigint primary key identity;
    title varchar(200);
    created_at timestamptz default now();
    updated_at timestamptz default now();
    index on (created_at, id);
}
class NoteCreate { title varchar(200) required, minLength(2); }
class NoteEdit { title varchar(200)?; }
server { cursor_secret = "0123456789abcdef0123456789abcdef"; }
middleware RequireAuth provides account_id: bigint { context.account_id = 1; }
"#;

/// What each snippet has to be surrounded by to be a whole program. A
/// snippet whose wrapper is `Alone` is one already.
#[derive(Clone, Copy)]
enum Wrap {
    Bare,
    Alone,
    Decl,
    Route,
    Expr,
    Service,
    Import,
}

fn wrapper(name: &str) -> Wrap {
    match name {
        "Namespace" | "Database" => Wrap::Bare,
        "Schema" | "Server block" | "Table" => Wrap::Alone,
        "Import" => Wrap::Import,
        "Route GET list" | "Route GET one" | "Route POST" | "Route PATCH" | "Route DELETE" => {
            Wrap::Route
        }
        "Select page" | "Select first" | "Select where" | "Insert" | "Update" | "Delete" => {
            Wrap::Expr
        }
        "Transaction" => Wrap::Service,
        _ => Wrap::Decl,
    }
}

/// `${1:name}` types its default, `$1` and `$0` are cursor stops, and `\$`
/// is the literal dollar every JWC binding starts with.
fn expand(body: &[Value]) -> String {
    let text: Vec<&str> = body.iter().map(|l| l.as_str().expect("string")).collect();
    let text = text.join("\n");
    // `${1:name}` types its default and `$1` mirrors it — the editor keeps
    // the two in step as you type, so a snippet can name a table once.
    let mut defaults: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' && chars.peek() == Some(&'$') {
            out.push(chars.next().expect("peeked"));
            continue;
        }
        if c != '$' {
            out.push(c);
            continue;
        }
        match chars.peek() {
            Some('{') => {
                chars.next();
                let mut inner = String::new();
                for c in chars.by_ref() {
                    if c == '}' {
                        break;
                    }
                    inner.push(c);
                }
                if let Some((n, default)) = inner.split_once(':') {
                    defaults.insert(n.to_string(), default.to_string());
                    out.push_str(default);
                }
            }
            Some(c) if c.is_ascii_digit() => {
                let mut n = String::new();
                while let Some(c) = chars.peek() {
                    if !c.is_ascii_digit() {
                        break;
                    }
                    n.push(*c);
                    chars.next();
                }
                if let Some(d) = defaults.get(&n) {
                    out.push_str(d);
                }
            }
            _ => out.push('$'),
        }
    }
    out
}

#[test]
fn every_snippet_is_a_program_the_compiler_accepts() {
    let snippets = read_json("vscode-extension/snippets/jwc.code-snippets");
    let snippets = snippets.as_object().expect("snippet map");
    assert!(!snippets.is_empty(), "no snippets");

    let mut failed = Vec::new();
    for (name, snippet) in snippets {
        let body = snippet["body"]
            .as_array()
            .unwrap_or_else(|| panic!("{name}: no body"));
        let text = expand(body);
        let dir = tempfile::tempdir().expect("tempdir");
        let target = match wrapper(name) {
            Wrap::Import => {
                std::fs::create_dir(dir.path().join("db")).expect("mkdir");
                std::fs::write(
                    dir.path().join("db/notes.jwc"),
                    format!("namespace db.notes;\n{PREAMBLE}"),
                )
                .expect("write");
                std::fs::write(
                    dir.path().join("app.jwc"),
                    format!("namespace app;\n{text}\nfunction main() {{ serve(); }}\n"),
                )
                .expect("write");
                dir.path().to_path_buf()
            }
            wrap => {
                let source = match wrap {
                    Wrap::Bare => format!("{text}\n"),
                    Wrap::Alone => {
                        format!("database App : Postgres;\nschema notes of App;\n{text}\n")
                    }
                    Wrap::Decl => format!("{PREAMBLE}\n{text}\n"),
                    Wrap::Route => format!("{PREAMBLE}\nroutes \"/api/v1/notes\" {{\n{text}\n}}\n"),
                    Wrap::Expr => format!(
                        "{PREAMBLE}\nroutes \"/x\" {{\n route GET \"{{id: bigint}}\" {{\n \
                         let cursor = request.query(\"c\");\n let size = 20;\n \
                         let req = request.body() as NoteCreate;\n return json({});\n }}\n}}\n",
                        text.trim().trim_end_matches(';')
                    ),
                    Wrap::Service => {
                        format!("{PREAMBLE}\nservice Svc {{\n function f() {{\n{text}\n }}\n}}\n")
                    }
                    Wrap::Import => unreachable!(),
                };
                let f = dir.path().join("a.jwc");
                std::fs::write(&f, source).expect("write");
                f
            }
        };
        let out = Command::new(env!("CARGO_BIN_EXE_jwc"))
            .arg("check")
            .arg(&target)
            .output()
            .expect("run jwc check");
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stdout).to_string()
                + &String::from_utf8_lossy(&out.stderr);
            failed.push(format!("{name}:\n{}", err.trim()));
        }
    }
    assert!(
        failed.is_empty(),
        "{} of {} snippets do not compile:\n\n{}",
        failed.len(),
        snippets.len(),
        failed.join("\n\n")
    );
}
