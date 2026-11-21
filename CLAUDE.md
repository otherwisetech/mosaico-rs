# mosaico-rs — for agents and contributors

Same conventions as rama-rs, diskmap and genealogy-rs.

## Layout

```
crates/core  mosaico-core: scan → meta (dates, previews) → timeline; thumb; plan → apply/undo
             (journal). Pure Rust parsers in tiff.rs (IFD walker) and bmff.rs (MOV/MP4/HEIC/CR3).
             fixtures.rs builds tiny EXIF/JPEG/MOV/HEIF files for tests. No UI.
crates/i18n  mosaico-i18n: embedded translation catalogs, t!/tn!/tv! macros, i18n guard tests
crates/app   `mosaico` desktop app (GPUI + gpui-component): app.rs (phases, start/scan, About),
             timeline.rs (sidebar + grid), organize.rs (panel, preview, apply, done, undo),
             thumbs.rs (thumbnail requests + LRU image cache), otherwise.rs (otherwise-link: links
             in, registry people read-only, About's other apps), people_view.rs (People list,
             selection bar with person tags); core/src/people.rs: the tags (`people.json`)
```

## Rules

- **The user's photos are at stake.** All file-changing code lives in `core/src/apply.rs` and
  keeps these invariants; any change to it needs a test in `crates/core/tests/organize.rs`:
  plans are read-only; never overwrite (temp file + hard-link/no-clobber placement); verify
  copies (size + blake3); the only file ever deleted is a move's verified source (and, on undo,
  a copy that still matches what was written while its original exists); journal every step
  before doing it. No delete feature.
- The UI never touches the disk except through `plan::build` / `apply::apply` / `apply::undo`,
  and only after the preview screen and a confirmation dialog.
- **i18n is mandatory.** No user-facing string literal in `app/src` or `core/src`; use
  `i18n::t!` / `tn!` and add the key to **every** `crates/i18n/locales/<lang>/` catalog in the
  same change. Untranslatable literals end their line with `// i18n-ok`. Month names are the
  dynamic `common.month.{1..12}` keys (listed in `DYNAMIC_PREFIXES`).
- No C dependencies (no libheif/libraw/ffmpeg). HEIC without an embedded JPEG gets a placeholder.
  The one exception is SQLite, bundled by otherwise-link (app crate only; core stays C-free).
- Person tags reference shared-registry entity ids and live only in mosaico's `people.json`
  (atomic write; a file that doesn't parse is never overwritten). mosaico never writes the
  registry: no upsert, no pull/ack. otherwise-link is pinned by git tag and must only add its
  own crates to Cargo.lock; tests use temp dirs (`Root::at`).
- Comments: sparse, only non-obvious why.
- GPUI pins: `Cargo.lock` pins the same zed / gpui-component revs as rama-rs. Never
  `cargo update` the GPUI git deps; restore the lock if a partial build prunes them.

## Validate

```
cargo test -p mosaico-i18n -p mosaico-core -p mosaico-app
cargo clippy --workspace --all-targets
cargo run --release -p mosaico-core --example dryrun -- <folder>   # read-only, never applies
```
