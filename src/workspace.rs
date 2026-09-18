//! A project's v1 sources, parsed together.
//!
//! `project.rs` (the 0.9.x loader) walks upward for `jwcproj.json` and reads
//! the old language. This is the same idea for v1, kept separate until the
//! v0.25.0 cutover.

use crate::diag::{Diagnostic, Severity};
use crate::token::Span;
use crate::ParsedFile;
use std::path::{Path, PathBuf};

/// A location: which file, and where in it. Spans alone are ambiguous once
/// more than one file is in play.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Loc {
    pub file: usize,
    pub span: Span,
}

pub struct Workspace {
    pub root: PathBuf,
    pub files: Vec<ParsedFile>,
    /// `jwcproj.json`'s `dependencies` keys. An import resolves to a
    /// namespace or to one of these (names.md §6.2.1).
    pub packages: std::collections::BTreeSet<String>,
    /// `jwcproj.json` itself, when there is one (packages.md §1).
    pub manifest: Option<Manifest>,
}

/// The project's own identity, as distinct from its dependencies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub name: String,
    pub version: String,
    /// `jwcproj.json`'s `"jwc"` — the language version the source is
    /// written for (packages.md §1). Absent means the project does not
    /// say, and nothing is checked.
    pub language: Option<String>,
    /// `"app"` (deployed) or `"pkg"` (imported). Anything else is read as
    /// an app: the content model only *restricts*, so an unknown value
    /// must not silently unlock declarations a package may not have.
    pub kind: Kind,
    pub path: PathBuf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    App,
    Package,
}

impl Workspace {
    /// Parse every `.jwc` file under `root` (or `root` itself when it is a
    /// file). Files are sorted so diagnostics and generated SQL come out in
    /// a stable order — `gen-sql` being byte-reproducible depends on it.
    pub fn load(root: impl AsRef<Path>) -> std::io::Result<Workspace> {
        Self::load_with(root, &std::collections::BTreeMap::new())
    }

    /// The same, with unsaved buffers taking precedence over what is on
    /// disk.
    ///
    /// The language server holds the editor's text, which by definition is
    /// not the file yet — a server that read the file would report
    /// diagnostics for the last save while the user looked at the next
    /// edit. A path in `overlay` that is not on disk is a new, unsaved file
    /// and is parsed too.
    pub fn load_with(
        root: impl AsRef<Path>,
        overlay: &std::collections::BTreeMap<PathBuf, String>,
    ) -> std::io::Result<Workspace> {
        let root = root.as_ref().to_path_buf();
        let mut paths = Vec::new();
        if root.is_file() {
            paths.push(root.clone());
        } else {
            walk(&root, &mut paths)?;
        }
        for p in overlay.keys() {
            if !paths.contains(p) {
                paths.push(p.clone());
            }
        }
        paths.sort();
        let mut files = Vec::with_capacity(paths.len());
        for p in paths {
            match overlay.get(&p) {
                Some(text) => files.push(crate::parse_str(&p, text)),
                None => files.push(crate::parse_file(&p)?),
            }
        }
        let packages = read_packages(&root);
        let manifest = read_manifest(&root);
        // Every command that compiles, formats, runs or serves comes
        // through here, so this is the one place the check can sit and
        // not be forgotten by one of them. `InvalidData` is the closest
        // `io::ErrorKind` to "the project is not for this compiler"; what
        // the reader sees is the message.
        language_check(&root)?;
        Ok(Workspace {
            root,
            files,
            packages,
            manifest,
        })
    }

    pub fn parse_errors(&self) -> Vec<String> {
        let mut out = Vec::new();
        for f in &self.files {
            for d in f.errors() {
                out.push(f.source.render(d));
            }
        }
        out
    }

    pub fn has_parse_errors(&self) -> bool {
        self.files.iter().any(|f| f.has_errors())
    }

    pub fn render(&self, loc: Loc, d: &Diagnostic) -> String {
        match self.files.get(loc.file) {
            Some(f) => f.source.render(d),
            None => format!("{}[{}]: {}\n", d.severity, d.code, d.message),
        }
    }

    /// `file:line` for a location — what `gen-sql --explain` prints above
    /// each statement (schema.md §9.1).
    pub fn file_line(&self, loc: Loc) -> String {
        let Some(f) = self.files.get(loc.file) else {
            return "<unknown>".into();
        };
        let (line, _) = f.source.line_col(loc.span.start);
        let rel = f
            .source
            .path
            .strip_prefix(&self.root)
            .unwrap_or(&f.source.path);
        format!("{}:{line}", rel.display())
    }

    pub fn count_errors(diags: &[(Loc, Diagnostic)]) -> usize {
        diags
            .iter()
            .filter(|(_, d)| d.severity == Severity::Error)
            .count()
    }
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let p = entry?.path();
        let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if name.starts_with('.') || name == "target" || name == "node_modules" {
            continue;
        }
        if p.is_dir() {
            walk(&p, out)?;
        } else if p.extension().and_then(|s| s.to_str()) == Some("jwc") {
            out.push(p);
        }
    }
    Ok(())
}

/// `jwcproj.json`'s `dependencies` keys, from the manifest at or above the
/// loaded root.
///
/// Best-effort: a project with no manifest has no package imports, which
/// makes every package import an `E0201` rather than a silent pass. A
/// manifest that does not parse is the same as none — the message the
/// reader needs is about the import, and `jwc check` on a broken manifest
/// has a louder problem than this pass.
/// `jwcproj.json`, from `root` or the nearest ancestor holding one.
fn read_manifest(root: &Path) -> Option<Manifest> {
    let mut dir = if root.is_file() {
        root.parent()
    } else {
        Some(root)
    };
    while let Some(d) = dir {
        let path = d.join("jwcproj.json");
        if path.is_file() {
            let text = std::fs::read_to_string(&path).ok()?;
            let json: serde_json::Value = serde_json::from_str(&text).ok()?;
            return Some(Manifest {
                name: json
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
                version: json
                    .get("version")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
                language: json.get("jwc").and_then(|v| v.as_str()).map(str::to_string),
                kind: match json.get("type").and_then(|v| v.as_str()) {
                    Some("pkg") => Kind::Package,
                    _ => Kind::App,
                },
                path,
            });
        }
        dir = d.parent();
    }
    None
}

/// Refuse a project written for a different release of the language.
///
/// `jwc fmt` calls this too: it does not build a `Workspace`, and it
/// parses with *this* compiler's grammar — rc.2's `--` comments and `$x`
/// locals are rc.3 errors, so formatting a project written for another
/// release is how a file gets mangled rather than formatted.
pub fn language_check(path: &Path) -> std::io::Result<()> {
    let Some(m) = read_manifest(path) else {
        return Ok(());
    };
    match language_mismatch(&m) {
        Some(why) => Err(std::io::Error::new(std::io::ErrorKind::InvalidData, why)),
        None => Ok(()),
    }
}

/// The migration diagnostics: what source written for an older release
/// produces under this one. `E0900`–`E0903` are the 0.9 → 1.0 cutover,
/// `E0906`–`E0907` the changes inside the candidate series.
pub fn is_migration_code(code: &str) -> bool {
    matches!(
        code,
        "E0900" | "E0901" | "E0902" | "E0903" | "E0906" | "E0907"
    )
}

/// The one thing to say before a wall of migration diagnostics from a
/// project whose manifest names no `jwc` version, or `None`.
///
/// The `jwc` field exists so that a project compiled by the wrong release
/// is told so before its diagnostics. A project that predates the field
/// is exactly the project that needs it and exactly the one that cannot
/// have it — MyWallet, written for 0.9.901, answered 290 errors under
/// rc.6 and not one mentioned a version. When the field is absent *and*
/// the source raises the diagnostics only an old dialect produces, that
/// is the version gap showing, and it is said once, before the list.
pub fn undated_migration_note(ws: &Workspace) -> Option<String> {
    let manifest = ws.manifest.as_ref()?;
    if manifest.language.is_some() {
        return None;
    }
    let mut codes: Vec<&str> = ws
        .files
        .iter()
        .flat_map(|f| f.diags.iter().map(|d| d.code))
        .filter(|c| is_migration_code(c))
        .collect();
    if codes.is_empty() {
        return None;
    }
    codes.sort_unstable();
    codes.dedup();
    let mine = env!("CARGO_PKG_VERSION");
    Some(format!(
        "note: {} names no `jwc` version, and the diagnostics below ({}) are          what source written for an older release looks like under jwc {mine}.\n\
         \x20     Add `\"jwc\": \"<the release it was written for>\"` and compile          with that release, or move the source to this one and add          `\"jwc\": \"{mine}\"`.\n",
        manifest.path.display(),
        codes.join(", ")
    ))
}

/// Why this compiler cannot be used on the project, or `None`.
///
/// The episode this exists for: an application written against one
/// release, compiled by another, and answering fifteen diagnostics that
/// were about the version gap and read as though they were about the
/// code. Nothing in the project said which release it was for, so there
/// was nothing to compare and no way to say so.
///
/// A bare version means exactly that version, as it does for a
/// dependency: `rc.N` and `rc.N+1` carry whatever review turned up and
/// promise nothing to each other (SEMVER.md), so "close enough" is not a
/// useful default. A range says so out loud — `"^1.0"`.
fn language_mismatch(m: &Manifest) -> Option<String> {
    let req = m.language.as_deref()?.trim();
    if req.is_empty() || req == "*" {
        return None;
    }
    let mine = env!("CARGO_PKG_VERSION");
    let here = m.path.display();

    let Ok(mine_parsed) = semver::Version::parse(mine) else {
        return None;
    };

    // `1.0.0-rc.4` is a version, not a range: exactly it.
    let matched = match semver::Version::parse(req) {
        Ok(exact) => exact == mine_parsed,
        Err(_) => match semver::VersionReq::parse(req) {
            Ok(range) => range.matches(&mine_parsed),
            Err(e) => {
                return Some(format!(
                    "{here}: `jwc` is `{req}`, which is neither a version \
                     nor a range ({e}). Name the version the source is \
                     written for: `\"jwc\": \"{mine}\"`."
                ))
            }
        },
    };

    if matched {
        return None;
    }

    // A range with no pre-release of its own does not match one, by the
    // semver rule — `^1.0` is not satisfied by `1.0.0-rc.4`. That is the
    // answer people reach for first, so it is worth saying rather than
    // leaving them to read it as a bug here.
    let prerelease_note = if !mine_parsed.pre.is_empty() && !req.contains('-') {
        "\n\nThis compiler is a pre-release, and a range that does not name \
         one never matches it — so name the version rather than a range \
         while the 1.0 candidates are running."
    } else {
        ""
    };

    Some(format!(
        "this is jwc {mine}, and {here} says the project is written \
         for `{req}`.\n\n\
         Install the version it asks for, or — once the source has been \
         moved to this one — change `jwc` in that file to `{mine}`. \
         Diagnostics from the wrong compiler read as though they were \
         about the code.{prerelease_note}"
    ))
}

fn read_packages(root: &Path) -> std::collections::BTreeSet<String> {
    let mut dir = if root.is_file() {
        root.parent()
    } else {
        Some(root)
    };
    while let Some(d) = dir {
        let manifest = d.join("jwcproj.json");
        if manifest.is_file() {
            let Ok(text) = std::fs::read_to_string(&manifest) else {
                return Default::default();
            };
            let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
                return Default::default();
            };
            return json
                .get("dependencies")
                .and_then(|d| d.as_object())
                .map(|o| o.keys().cloned().collect())
                .unwrap_or_default();
        }
        dir = d.parent();
    }
    Default::default()
}
