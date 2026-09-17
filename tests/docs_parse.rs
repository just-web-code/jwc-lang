//! Every ```jwc block in the README and `docs/spec/v1/` must be real JWC.
//!
//! Nothing checked the documentation against the compiler, so it drifted:
//! the README's headline example and three reference pages showed forms the
//! parser had never accepted. A reader copying the first example on the
//! front page got a syntax error.
//!
//! Only parsing is asserted, never checking: documentation deliberately
//! references tables and classes it does not define.
//!
//! `docs/archive-0.9/` is **not** checked. It documents the language that
//! the v0.25.0 cutover removed, and checking it against this compiler would
//! assert that a dead grammar still parses.
//!
//! ## Illustrative blocks
//!
//! Some blocks are prose, not programs — operator tables, `{ ... }`
//! elisions, bare expression lists with trailing comments. Only a **bare**
//! ```` ```jwc ```` fence is compiled, so mark those with
//! ```` ```jwc no-compile ````. The marker sits in the fence's info string,
//! which the docs site ignores, so it costs the reader nothing while
//! keeping the exemption explicit in source.

use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn markdown_files() -> Vec<PathBuf> {
    let root = repo_root();
    let mut out = vec![root.join("README.md")];
    let mut stack = vec![root.join("docs")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                // Vendored site build output, not authored docs.
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                // `archive-0.9` documents the language the cutover
                // removed; checking it here would assert that a dead
                // grammar still parses.
                if matches!(
                    name,
                    "node_modules" | "build" | ".docusaurus" | "archive-0.9"
                ) {
                    continue;
                }
                stack.push(p);
            } else if p.extension().and_then(|e| e.to_str()) == Some("md") {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

/// Extract bare ```jwc fenced blocks with the 1-based line each starts on.
fn jwc_blocks(text: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut lines = text.lines().enumerate();
    while let Some((i, line)) = lines.next() {
        // Only a bare ```jwc fence. An info string (```jwc no-compile,
        // ```jwc title="x") marks a block that is shown, not compiled.
        if line.trim() != "```jwc" {
            continue;
        }
        let mut body = String::new();
        for (_, l) in lines.by_ref() {
            if l.trim_start().starts_with("```") {
                break;
            }
            body.push_str(l);
            body.push('\n');
        }
        out.push((i + 2, body));
    }
    out
}

/// Every ```jwc fence, whatever its info string — `no-compile` included.
/// `jwc_blocks` deliberately skips those because they are not programs;
/// the binding scan below wants them precisely because nothing else reads
/// them.
fn all_jwc_blocks(text: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut lines = text.lines().enumerate();
    while let Some((i, line)) = lines.next() {
        let t = line.trim();
        if t != "```jwc" && !t.starts_with("```jwc ") {
            continue;
        }
        let mut body = String::new();
        for (_, l) in lines.by_ref() {
            if l.trim_start().starts_with("```") {
                break;
            }
            body.push_str(l);
            body.push('\n');
        }
        out.push((i + 2, body));
    }
    out
}

/// v1 spec blocks, checked with the v1 front-end. Same excerpt problem:
/// a clause shown on its own is not a program, so try the positions an
/// excerpt can legally occupy.
fn parses_somewhere(body: &str) -> bool {
    const HEADER: &str = concat!(
        "database App : Postgres;\n",
        "schema s of App;\n",
        "table T of App.s { id bigint primary key identity; }\n",
    );
    let contexts = [
        body.to_string(),
        format!("{HEADER}{body}\n"),
        format!("{HEADER}function f() {{\n{body}\n}}\n"),
        format!("{HEADER}function f() {{\nreturn {body};\n}}\n"),
        format!("{HEADER}table U of App.s {{\n{body}\n}}\n"),
        format!("{HEADER}class C {{\n{body}\n}}\n"),
        format!("{HEADER}middleware M {{\n{body}\n}}\n"),
        format!("{HEADER}routes \"/x\" {{\nroute GET \"\" {{\n{body}\n}}\n}}\n"),
        format!("{HEADER}view V of App.s {{\n{body}\n}}\n"),
    ];
    contexts
        .iter()
        .any(|src| !jwc::parse_str("<doc>", src).has_errors())
}

#[test]
fn every_documented_jwc_example_parses() {
    let root = repo_root();
    let mut broken = Vec::new();
    let mut checked = 0usize;

    for file in markdown_files() {
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        for (line, body) in jwc_blocks(&text) {
            if body.trim().is_empty() {
                continue;
            }
            checked += 1;
            let ok = parses_somewhere(&body);
            if !ok {
                let rel: &Path = file.strip_prefix(&root).unwrap_or(&file);
                broken.push(format!("{}:{line}", rel.display()));
            }
        }
    }

    // A floor, not a target: it exists so a broken block-scanner reads as
    // a failure rather than as "nothing to check". It dropped when the
    // 0.9.x docs were archived and the spec became the corpus.
    assert!(
        checked > 20,
        "expected to find the documented examples, saw {checked}"
    );
    assert!(
        broken.is_empty(),
        "{} documented example(s) don't parse — a reader copying these gets a \
         syntax error. Fix the example, or if it is deliberately an excerpt \
         (elisions, operator tables), change its fence to ```jwc no-compile:\n  {}",
        broken.len(),
        broken.join("\n  ")
    );
}

/// `spec-coverage.json` matches the sample it claims to describe.
///
/// ROADMAP §10 lists this file as the mitigation for "the sample stops
/// keeping up with the spec": a construct not tied to a clause is supposed
/// to fail the build. Nothing ran the generator — not CI, not a test — so
/// the file was a snapshot of whenever it was last produced by hand, and
/// it had drifted from the sample it names. A mitigation nothing executes
/// is not one.
#[test]
fn the_spec_coverage_map_is_current() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let map = root.join("docs/spec/v1/spec-coverage.json");
    let before = std::fs::read_to_string(&map).expect("spec-coverage.json");

    let out = std::process::Command::new("python3")
        .arg(root.join("docs/spec/v1/check_sample.py"))
        .output();
    let Ok(out) = out else {
        eprintln!("SKIPPED the_spec_coverage_map_is_current — no python3");
        return;
    };

    // The generator rewrites the file in place, so restore it before
    // asserting: a failing test must not leave the tree dirty.
    let after = std::fs::read_to_string(&map).expect("spec-coverage.json");
    std::fs::write(&map, &before).expect("restore");

    assert!(
        out.status.success(),
        "check_sample.py rejected the sample:\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        before == after,
        "spec-coverage.json is stale — run `python3 docs/spec/v1/check_sample.py` \
         and commit the result"
    );
}

/// The agent guide's examples do not merely parse — they **check**.
///
/// It is written to be pasted into a coding agent's context, so every
/// program in it is something an agent will copy verbatim. Parsing is not
/// enough: `as many` on a `select` parses (it reads as `as <class>`) and
/// is `E0301`, and a query with `page` and no `server { cursor_secret }`
/// parses and is `E1205`. Both were in the first draft of this page.
///
/// Each block is a whole program on its own, so it is checked on its own.
#[test]
fn every_agent_guide_example_type_checks() {
    for page in ["ai-agent-guide.md", "language.md"] {
        check_page(page);
    }
}

/// Every ```jwc``` block on a reference page, compiled as its own program.
///
/// Both pages say they are the language in one file, and both are copied
/// verbatim — by a person into an editor and by an agent into a context
/// window. A page whose examples do not check is worse than no page,
/// because the reader trusts it first and debugs the language second.
fn check_page(page: &str) {
    let root = repo_root();
    let path = root.join("docs/docs/reference").join(page);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{page}: {e}"));

    let mut broken: Vec<String> = Vec::new();
    let mut checked = 0usize;

    for (line, body) in jwc_blocks(&text) {
        if body.trim().is_empty() {
            continue;
        }
        checked += 1;

        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("a.jwc"), &body).expect("write");
        let ws = match jwc::workspace::Workspace::load(dir.path()) {
            Ok(ws) => ws,
            Err(e) => {
                broken.push(format!("{page}:{line}: {e}"));
                continue;
            }
        };
        if ws.has_parse_errors() {
            broken.push(format!("{page}:{line}: {}", ws.parse_errors().join(" ")));
            continue;
        }
        let built = jwc::model::build(&ws);
        let sym = jwc::symbols::build(&ws, &built.model);
        let checked_out = jwc::check::check(&ws, &sym, &built.model);
        let wired = jwc::wiring::wire(&ws, &sym);
        let errors: Vec<String> = built
            .diags
            .iter()
            .chain(&sym.diags)
            .chain(&checked_out.diags)
            .chain(&wired.diags)
            .filter(|(_, d)| d.severity == jwc::diag::Severity::Error)
            .map(|(_, d)| format!("{}: {}", d.code, d.message))
            .collect();
        if !errors.is_empty() {
            broken.push(format!("{page}:{line}: {}", errors.join("; ")));
        }
    }

    assert!(
        checked >= 5,
        "expected {page}'s programs, saw {checked} — the block scanner \
         or the page's fences changed"
    );
    assert!(
        broken.is_empty(),
        "{} example(s) in {page} do not check. A reader copies these \
         verbatim:\n  {}",
        broken.len(),
        broken.join("\n  ")
    );
}

/// Every jwc block in the docs, `no-compile` included, is scanned for a
/// column that does not name its binding.
///
/// `every_documented_jwc_example_parses` above only parses, and a bare
/// column parses perfectly — `E0904` is a resolver error. So when rc.3 made
/// every column name its binding, the tutorial's queries stayed as they
/// were and nothing went red; three releases later the first page a reader
/// meets still showed `where slug == @slug`. The `no-compile` fence, which
/// exists for excerpts with elisions, had quietly become a place where code
/// was never checked at all.
///
/// A fragment cannot produce `E0904`: it has to resolve before the binding
/// rule is reached, so a block that fails to parse or has no schema simply
/// yields nothing here. That makes this safe to run over every fence,
/// which is the point — the blocks nothing else checks are exactly the
/// blocks that rot.
/// The sources a snippet names but does not declare, as declarations.
///
/// `check::select` returns on `E0502` (unknown source) before it ever
/// visits the query's fields, so a snippet that queries `App.notes.Notes`
/// without declaring it is skipped rather than checked — which is how
/// `docs/docs/data/writes.md` kept a bare `as { id, title, updated_at }`
/// even after the tutorial's was found. The columns do not matter: the
/// projection's binding rule fires on the qualifier alone, before any
/// column lookup. Only reaching the projection does.
fn synthesized_sources(src: &str) -> String {
    let mut dbs: Vec<String> = Vec::new();
    let mut schemas: Vec<(String, String)> = Vec::new();
    let mut tables: Vec<(String, String, String)> = Vec::new();

    for (i, _) in src.match_indices('.') {
        let before = &src[..i];
        let after = &src[i + 1..];
        let db: String = before
            .chars()
            .rev()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        let rest: String = after
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '.')
            .collect();
        let parts: Vec<&str> = rest.split('.').collect();
        if db.is_empty() || parts.len() < 2 || parts[0].is_empty() || parts[1].is_empty() {
            continue;
        }
        if !db.chars().next().is_some_and(|c| c.is_uppercase()) {
            continue;
        }
        if !parts[1].chars().next().is_some_and(|c| c.is_uppercase()) {
            continue;
        }
        let (sch, tab) = (parts[0].to_string(), parts[1].to_string());
        if !src.contains(&format!("database {db}")) && !dbs.contains(&db) {
            dbs.push(db.clone());
        }
        if !src.contains(&format!("schema {sch} of"))
            && !schemas.contains(&(db.clone(), sch.clone()))
        {
            schemas.push((db.clone(), sch.clone()));
        }
        if !src.contains(&format!("table {tab} of"))
            && !src.contains(&format!("view {tab} of"))
            && !tables.contains(&(db.clone(), sch.clone(), tab.clone()))
        {
            tables.push((db, sch, tab));
        }
    }

    let mut out = String::new();
    for db in &dbs {
        out.push_str(&format!("database {db} : Postgres;\n"));
    }
    for (db, sch) in &schemas {
        out.push_str(&format!("schema {sch} of {db};\n"));
    }
    for (db, sch, tab) in &tables {
        out.push_str(&format!(
            "table {tab} of {db}.{sch} {{ id bigint primary key identity; }}\n"
        ));
    }
    out
}

/// Every jwc block in the docs, `no-compile` included, is scanned for a
/// column that does not name its binding.
///
/// `every_documented_jwc_example_parses` above only parses, and a bare
/// column parses perfectly — `E0904` is a resolver error. So when rc.3 made
/// every column name its binding, the tutorial's queries stayed as they
/// were and nothing went red; three releases later the first page a reader
/// meets still showed `where slug == @slug`. The `no-compile` fence, which
/// exists for excerpts with elisions, had quietly become a place where code
/// was never checked at all.
///
/// A fragment cannot produce `E0904`: it has to resolve before the binding
/// rule is reached, so a block that fails to parse simply yields nothing
/// here. That makes this safe to run over every fence, which is the point —
/// the blocks nothing else checks are exactly the blocks that rot.
#[test]
fn every_documented_column_names_its_binding() {
    const HEADER: &str = concat!(
        "database App : Postgres;\n",
        "schema s of App;\n",
        "table T of App.s { id bigint primary key identity; }\n",
    );
    let root = repo_root();
    let mut bare: Vec<String> = Vec::new();
    let mut resolved = 0usize;

    for file in markdown_files() {
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        let blocks = all_jwc_blocks(&text);
        if blocks.is_empty() {
            continue;
        }
        let rel: &Path = file.strip_prefix(&root).unwrap_or(&file);

        // A page is read top to bottom: the schema block declares what the
        // service block below it queries. Checked one at a time the service
        // block has no tables, resolution stops at the missing schema, and
        // the binding rule is never reached — which is how the tutorial's
        // queries stayed unqualified for three releases. So try the page as
        // one program first, then each block on its own for the pages whose
        // examples stand alone.
        let joined: String = blocks
            .iter()
            .map(|(_, b)| b.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let mut sources = vec![
            joined.clone(),
            format!("{HEADER}{joined}\n"),
            format!("{}{joined}\n", synthesized_sources(&joined)),
        ];
        for (_, body) in &blocks {
            sources.push(body.clone());
            sources.push(format!("{HEADER}{body}\n"));
            sources.push(format!("{}{body}\n", synthesized_sources(body)));
            sources.push(format!("{HEADER}function f() {{\n{body}\n}}\n"));
            sources.push(format!(
                "{HEADER}service S {{\nfunction f() {{\n{body}\n}}\n}}\n"
            ));
        }

        let mut seen: Vec<String> = Vec::new();
        for src in &sources {
            let dir = tempfile::tempdir().expect("tempdir");
            std::fs::write(dir.path().join("a.jwc"), src).expect("write");
            let Ok(ws) = jwc::workspace::Workspace::load(dir.path()) else {
                continue;
            };
            if ws.has_parse_errors() {
                continue;
            }
            resolved += 1;
            let built = jwc::model::build(&ws);
            let sym = jwc::symbols::build(&ws, &built.model);
            let out = jwc::check::check(&ws, &sym, &built.model);
            for (loc, d) in built.diags.iter().chain(&sym.diags).chain(&out.diags) {
                if d.code != "E0904" && d.code != "E0905" {
                    continue;
                }
                // The span points into the synthesized program, not the
                // page, so quote the offending line — that is what makes
                // the failure findable in the markdown.
                let at = (loc.span.start as usize).min(src.len());
                let line = src[..at]
                    .lines()
                    .next_back()
                    .map(|l| {
                        let rest: &str = src[at..].lines().next().unwrap_or("");
                        format!("{l}{rest}").trim().to_string()
                    })
                    .unwrap_or_default();
                let hit = format!("{}: {} — `{line}`", d.code, d.message);
                if !seen.contains(&hit) {
                    seen.push(hit);
                }
            }
        }
        for hit in seen {
            bare.push(format!("{}: {hit}", rel.display()));
        }
    }

    // A floor, not a target: without it a fence scanner that stops matching
    // makes this pass by checking nothing, which is the failure mode the
    // guard exists to prevent.
    assert!(
        resolved > 50,
        "expected the documented examples to resolve, saw {resolved} — the \
         block scanner or the fences changed"
    );
    assert!(
        bare.is_empty(),
        "{} documented example(s) use a column that does not name its \
         binding (queries.md §2.4). A reader copies these verbatim and gets \
         E0904:\n  {}",
        bare.len(),
        bare.join("\n  ")
    );
}

/// Every `jwcproj.json` shown in the docs carries the `jwc` field, at this
/// release.
///
/// rc.4 made the field the way a project says which compiler it is written
/// for, and three pages kept showing a manifest without one — including the
/// first manifest a reader ever sees, on hello-world. Copying it produces a
/// project that says nothing about its language version, which is the exact
/// situation the field was added to end.
///
/// It pins the version, so it moves with `Cargo.toml` like the sample does.
#[test]
fn every_documented_manifest_names_this_release() {
    let root = repo_root();
    let mine = env!("CARGO_PKG_VERSION");
    let mut stale: Vec<String> = Vec::new();
    let mut found = 0usize;

    for file in markdown_files() {
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        for (line, body) in json_blocks(&text) {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(&body) else {
                continue;
            };
            // A package manifest has no `entry` — it is imported, not
            // deployed — so requiring one skipped `packages.md`'s
            // `"type": "pkg"` example entirely. `name` + `version` with
            // either marker is what distinguishes a manifest from the other
            // JSON on these pages.
            let is_manifest = v.get("name").is_some()
                && v.get("version").is_some()
                && (v.get("entry").is_some() || v.get("type").is_some());
            if !is_manifest {
                continue;
            }
            found += 1;
            let rel: &Path = file.strip_prefix(&root).unwrap_or(&file);
            match v.get("jwc").and_then(|j| j.as_str()) {
                Some(got) if got == mine => {}
                Some(got) => stale.push(format!("{}:{line}: says {got}", rel.display())),
                None => stale.push(format!("{}:{line}: no `jwc` field", rel.display())),
            }
        }
    }

    // Without a floor, a fence scanner that stops matching makes this pass
    // by checking nothing.
    assert!(
        found >= 4,
        "expected the documented manifests, saw {found} — the json fence \
         scanner or the manifest shape changed"
    );
    assert!(
        stale.is_empty(),
        "{} documented manifest(s) do not name {mine}. They move with \
         Cargo.toml:\n  {}",
        stale.len(),
        stale.join("\n  ")
    );
}

/// ```json fences, with or without an info string (```json title="…").
fn json_blocks(text: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut lines = text.lines().enumerate();
    while let Some((i, line)) = lines.next() {
        let t = line.trim();
        if t != "```json" && !t.starts_with("```json ") {
            continue;
        }
        let mut body = String::new();
        for (_, l) in lines.by_ref() {
            if l.trim_start().starts_with("```") {
                break;
            }
            body.push_str(l);
            body.push('\n');
        }
        out.push((i + 2, body));
    }
    out
}

/// Every release version written out in the docs is *this* release.
///
/// A bump moves `Cargo.toml`; twenty-odd other places spell the same
/// version by hand, and only a reader's eye connected them. Two had gone
/// stale without anyone noticing: SEMVER.md illustrated compatibility with
/// the previous pair of releases, and the README and CONTRIBUTING both
/// announced rc.1 as the *next* milestone five releases after it shipped.
///
/// `ROADMAP.md` is deliberately not read. Its milestone names are section
/// titles, and shipping a release does not rename them.
///
/// Four shapes name another release on purpose, each recognised from the
/// words immediately before it rather than listed by path:
///
/// - `^1.0.0-rc.1` — a range bound; a fixed fact about what the field admits
/// - `as of 1.0.0-rc.1` — when something became true
/// - `ROADMAP v1.0.0-rc.1` — a milestone's name, owned by that file
/// - ``written for `1.0.0-rc.4` `` — the compiler's own wording for a *gap*
///
/// A bare `` `rc.N` `` is the `rc.N → rc.N+1` sentence, so it may also name
/// the release before this one.
#[test]
fn every_documented_version_names_this_release() {
    let root = repo_root();
    let mine = env!("CARGO_PKG_VERSION");
    // The policy and the contributor guide pin the version too, and neither
    // is under `docs/`.
    let mut files = markdown_files();
    files.push(root.join("SEMVER.md"));
    files.push(root.join("CONTRIBUTING.md"));

    let mut stale: Vec<String> = Vec::new();
    let mut found = 0usize;

    for file in files {
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        let rel: &Path = file.strip_prefix(&root).unwrap_or(&file);
        for (n, line) in text.lines().enumerate() {
            let at =
                |what: &str| format!("{}:{}: {what}\n      {}", rel.display(), n + 1, line.trim());

            for (col, token) in release_tokens(line) {
                if names_another_release_on_purpose(line, col) {
                    continue;
                }
                found += 1;
                let got = token.strip_prefix('v').unwrap_or(token);
                if got != mine {
                    stale.push(at(&format!("says {got}")));
                }
            }

            for written in bare_candidates(line) {
                match this_candidate() {
                    Some(now) if written == now || written + 1 == now => {}
                    Some(_) => stale.push(at(&format!("says `rc.{written}`"))),
                    // Not a candidate release; the sentence does not apply.
                    None => {}
                }
            }
        }
    }

    // Without a floor, a scanner that stops matching makes this pass by
    // checking nothing.
    assert!(
        found >= 15,
        "expected the documented version pins, saw {found} — the scanner or \
         the way the docs write a version changed"
    );
    assert!(
        stale.is_empty(),
        "{} documented version(s) do not name {mine}. They move with \
         Cargo.toml:\n  {}",
        stale.len(),
        stale.join("\n  ")
    );
}

/// This release's candidate number, if it is one: `1.0.0-rc.6` → `6`.
fn this_candidate() -> Option<u32> {
    env!("CARGO_PKG_VERSION").split_once("-rc.")?.1.parse().ok()
}

/// `1.0.0-rc.6` and `v1.0.0-rc.6`, each with the byte it starts at.
fn release_tokens(line: &str) -> Vec<(usize, &str)> {
    runs(line)
        .into_iter()
        .filter(|(_, t)| is_release(t.strip_prefix('v').unwrap_or(t)))
        .collect()
}

/// Maximal runs of the characters a version is written with. A sigil like
/// `^` is not one of them, which is what leaves it visible to the check
/// below.
fn runs(line: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, c) in line.char_indices() {
        match (c.is_ascii_alphanumeric() || c == '.' || c == '-', start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                out.push((s, &line[s..i]));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        out.push((s, &line[s..]));
    }
    out
}

fn is_release(t: &str) -> bool {
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let Some((core, n)) = t.split_once("-rc.") else {
        return false;
    };
    let parts: Vec<&str> = core.split('.').collect();
    parts.len() == 3 && parts.iter().all(|p| digits(p)) && digits(n)
}

/// Whether the words immediately before a version say it is a fact about
/// some other release rather than a pin on this one.
fn names_another_release_on_purpose(line: &str, at: usize) -> bool {
    let before = &line[..at];
    if before.ends_with('^') || before.ends_with('~') {
        return true;
    }
    let lead = before.trim_end_matches(['`', '\'', '"', '*', ' ']);
    lead.ends_with("as of") || lead.ends_with("ROADMAP") || lead.ends_with("written for")
}

/// `` `rc.6` `` — the bare form. Only inside backticks, so prose about an
/// older release is not swept up with it.
fn bare_candidates(line: &str) -> Vec<u32> {
    line.split('`')
        .skip(1)
        .step_by(2)
        .filter_map(|t| t.strip_prefix("rc."))
        .filter_map(|n| n.parse().ok())
        .collect()
}
