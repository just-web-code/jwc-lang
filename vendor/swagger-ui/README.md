# swagger-ui-dist, vendored

**5.33.1**, from `npm pack swagger-ui-dist`. Three files, gzipped at level
9 and compiled into the binary with `include_bytes!`:

| | raw | stored |
|---|---:|---:|
| `swagger-ui-bundle.js` | 1,549 KB | 420 KB |
| `swagger-ui-standalone-preset.js` | 261 KB | 78 KB |
| `swagger-ui.css` | 182 KB | 26 KB |

524 KB against a 19.8 MB release binary, and the server sends them with
`content-encoding: gzip` exactly as stored, so nothing is decompressed on
the way out.

Vendored rather than fetched from a CDN for the reason the module has
always given: a reference page that loads its renderer from unpkg is blank
on an air-gapped box and pins a third-party script into a developer's
browser session.

`LICENSE` is Apache-2.0, as published. The two `*.LICENSE.txt` files are
the dependency notices webpack emitted beside the bundles; they are kept
because the bundles reference them.

## Upgrading

```
npm pack swagger-ui-dist
tar xzf swagger-ui-dist-<v>.tgz -C /tmp/swui --strip-components=1
for f in swagger-ui-bundle.js swagger-ui-standalone-preset.js swagger-ui.css; do
    gzip -9 -c /tmp/swui/$f > vendor/swagger-ui/$f.gz
done
cp /tmp/swui/LICENSE /tmp/swui/*.LICENSE.txt vendor/swagger-ui/
```

Then update the version here and in `src/swagger.rs`, and run
`cargo test --lib swagger`.
