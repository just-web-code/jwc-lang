use anyhow::Result;
use serde_json::Value;

/// The three files, gzipped at level 9, exactly as the browser receives
/// them. See `vendor/swagger-ui/README.md` for how they are refreshed.
pub const UI_JS: &[u8] = include_bytes!("../vendor/swagger-ui/swagger-ui-bundle.js.gz");
pub const UI_PRESET: &[u8] =
    include_bytes!("../vendor/swagger-ui/swagger-ui-standalone-preset.js.gz");
pub const UI_CSS: &[u8] = include_bytes!("../vendor/swagger-ui/swagger-ui.css.gz");

/// The version vendored, shown in the page footer so a bug report names it.
pub const UI_VERSION: &str = "5.33.1";

/// What the served page asks for, relative to `server { swagger }`.
pub const ASSETS: &[(&str, &[u8], &str)] = &[
    ("swagger-ui.css", UI_CSS, "text/css; charset=utf-8"),
    (
        "swagger-ui-bundle.js",
        UI_JS,
        "application/javascript; charset=utf-8",
    ),
    (
        "swagger-ui-standalone-preset.js",
        UI_PRESET,
        "application/javascript; charset=utf-8",
    ),
];

fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// The configuration both forms share.
///
/// `persistAuthorization` is the one departure from the defaults: a
/// reader who pasted a token and then reloaded has not changed their
/// mind, and losing it on every refresh is the thing people complain
/// about first.
const INIT: &str = r##"
      deepLinking: true,
      persistAuthorization: true,
      displayRequestDuration: true,
      filter: true,
      tryItOutEnabled: true,
      docExpansion: 'list',
      defaultModelsExpandDepth: 1,
      presets: [SwaggerUIBundle.presets.apis, SwaggerUIStandalonePreset],
      plugins: [SwaggerUIBundle.plugins.DownloadUrl],
      layout: 'StandaloneLayout',
"##;

/// The page the server returns at `server { swagger }`.
///
/// Assets come from the same origin under `prefix`, so the whole
/// reference moves with the service and nothing is fetched off the box.
///
/// The hrefs are absolute rather than relative because the page answers
/// at `/docs`, not `/docs/`, and `strict_slash` redirects the second to
/// the first (config §3.2). A relative `swagger-ui.css` there resolves to
/// `/swagger-ui.css` — the site root, where the API lives — and the
/// browser is handed JSON where it asked for a stylesheet.
pub fn page(title: &str, prefix: &str) -> String {
    let base = prefix.trim_end_matches('/');
    format!(
        "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <title>{t} — API</title>\
         <link rel=\"stylesheet\" href=\"{base}/swagger-ui.css\">\
         <style>body{{margin:0}}.swagger-ui .topbar{{background:#1b1b1f}}</style>\
         </head><body><div id=\"swagger-ui\"></div>\
         <script src=\"{base}/swagger-ui-bundle.js\"></script>\
         <script src=\"{base}/swagger-ui-standalone-preset.js\"></script>\
         <script>window.onload=function(){{SwaggerUIBundle({{\
         url:'{base}/openapi.json',dom_id:'#swagger-ui',{init}}});}};</script>\
         </body></html>\n",
        t = esc(title),
        init = INIT,
    )
}

/// The served page, titled from the document it will load.
///
/// Both backends render at build time and serve a `&'static str`, so the
/// title is read here rather than by the page at runtime.
pub fn render(doc: &Value, prefix: &str) -> String {
    page(
        doc.pointer("/info/title")
            .and_then(Value::as_str)
            .unwrap_or("API"),
        prefix,
    )
}

/// `jwc swagger --out api.html` — the same UI as one file.
///
/// Everything is inlined, including the document, so it opens from a
/// filesystem with no server at all. `Try it out` then needs a running
/// API and a CORS policy that admits `null` as an origin; the page says
/// so rather than leaving the reader to work it out from a failed fetch.
pub fn standalone(doc: &Value, title: &str) -> Result<String> {
    let css = gunzip(UI_CSS)?;
    let js = gunzip(UI_JS)?;
    let preset = gunzip(UI_PRESET)?;
    // `</script>` inside the document would close the tag that carries it.
    let spec = serde_json::to_string(doc)?.replace("</", "<\\/");

    Ok(format!(
        "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <title>{t} — API</title><style>{css}</style>\
         <style>body{{margin:0}}.swagger-ui .topbar{{background:#1b1b1f}}</style>\
         </head><body><div id=\"swagger-ui\"></div>\
         <script>{js}</script><script>{preset}</script>\
         <script>window.onload=function(){{SwaggerUIBundle({{\
         spec:{spec},dom_id:'#swagger-ui',{init}}});}};</script>\
         </body></html>\n",
        t = esc(title),
        init = INIT,
    ))
}

/// The vendored assets are stored compressed and served that way; only
/// the one-file form has to expand them.
fn gunzip(bytes: &[u8]) -> Result<String> {
    use std::io::Read as _;
    let mut out = String::new();
    flate2::read::GzDecoder::new(bytes).read_to_string(&mut out)?;
    Ok(out)
}

/// `jwc swagger [path]` — the reference on its own port.
///
/// The assets come from this process rather than a CDN, compressed
/// exactly as they are stored.
pub async fn serve(doc: Value, port: u16) -> Result<()> {
    use axum::response::{Html, IntoResponse};
    use axum::routing::get;

    let html = render(&doc, "");
    let json = format!("{}\n", serde_json::to_string_pretty(&doc)?);

    let mut app = axum::Router::new()
        .route("/", get(move || async move { Html(html) }))
        .route(
            "/openapi.json",
            get(move || async move {
                ([("content-type", "application/json; charset=utf-8")], json).into_response()
            }),
        );

    for (name, bytes, mime) in ASSETS {
        app = app.route(
            &format!("/{name}"),
            get(move || async move {
                (
                    [
                        ("content-type", *mime),
                        ("content-encoding", "gzip"),
                        ("cache-control", "public, max-age=31536000, immutable"),
                    ],
                    *bytes,
                )
                    .into_response()
            }),
        );
    }

    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    println!("API reference on http://{addr}  (Ctrl-C to stop)");
    axum::serve(listener, app).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn doc() -> Value {
        json!({
            "openapi": "3.1.0",
            "info": { "title": "Notes", "version": "1.0.0" },
            "paths": {
                "/notes": {
                    "post": {
                        "operationId": "postNotes",
                        "tags": ["notes"],
                        "responses": { "201": { "description": "Created" } },
                    }
                }
            },
        })
    }

    /// The served page points one directory down, not at a CDN. Those
    /// three names are what `serve.rs` and `cmd::swagger` must answer, so
    /// a rename that misses one is caught here.
    #[test]
    fn the_page_fetches_only_its_own_origin() {
        let html = page("Notes", "/docs");
        for needle in ["http://", "https://", "//unpkg", "//cdn"] {
            assert!(!html.contains(needle), "`{needle}` in {html}");
        }
        for (name, _, _) in ASSETS {
            assert!(
                html.contains(&format!("\"/docs/{name}\"")),
                "{name} in {html}"
            );
        }
        assert!(html.contains("url:'/docs/openapi.json'"), "{html}");
    }

    /// The page answers at `/docs`, not `/docs/`, so a relative href
    /// resolves to the site root — where the API is, not the assets.
    #[test]
    fn the_asset_links_are_absolute_under_the_prefix() {
        let html = page("Notes", "/docs");
        assert!(html.contains("href=\"/docs/swagger-ui.css\""), "{html}");
        assert!(!html.contains("href=\"swagger-ui.css\""), "{html}");

        // `jwc swagger` serves the reference at the root, where the empty
        // prefix leaves `/swagger-ui.css` — correct, and not a double slash.
        let own = page("Notes", "");
        assert!(own.contains("href=\"/swagger-ui.css\""), "{own}");
        assert!(!own.contains("//swagger-ui.css"), "{own}");
    }

    /// One file, no server, no network.
    ///
    /// The assertion is on the page's own tags, not on the substring: a
    /// 1.5 MB minified bundle builds DOM and carries licence URLs, so
    /// `src=` and `https://` appear inside it no matter what. What must
    /// not appear is a tag that goes and gets something.
    #[test]
    fn the_standalone_page_inlines_everything() {
        let html = standalone(&doc(), "Notes").expect("render");
        for needle in ["<script src", "<link ", "<iframe", "@import"] {
            assert!(!html.contains(needle), "`{needle}` in a one-file page");
        }
        // The UI and the document are both in there.
        assert!(html.contains("SwaggerUIBundle"), "no bundle");
        assert!(html.contains("postNotes"), "no document");
        assert!(html.len() > 1_000_000, "suspiciously small: {}", html.len());
    }

    /// `</script>` in a summary would end the tag the document sits in.
    #[test]
    fn the_inlined_document_cannot_close_its_own_tag() {
        let mut d = doc();
        d["paths"]["/notes"]["post"]["summary"] = json!("</script><img src=x onerror=alert(1)>");
        let html = standalone(&d, "Notes").expect("render");
        assert!(
            !html.contains("</script><img"),
            "tag closed by the document"
        );
        assert!(
            html.contains("<\\/script>"),
            "not escaped: {}",
            &html[..200]
        );
    }

    /// A title is program text and program text with a `<` in it must not
    /// become markup.
    #[test]
    fn the_title_is_escaped() {
        let html = page("<img src=x onerror=alert(1)>", "/docs");
        assert!(!html.contains("<img src=x"), "{html}");
        assert!(html.contains("&lt;img"), "{html}");
    }

    /// Stored compressed, served compressed. A file that is not valid
    /// gzip would reach the browser as bytes it cannot read.
    #[test]
    fn every_vendored_asset_is_gzip() {
        for (name, bytes, _) in ASSETS {
            assert_eq!(&bytes[..2], b"\x1f\x8b", "{name} is not gzip");
            assert!(gunzip(bytes).is_ok(), "{name} does not decompress");
        }
    }
}
