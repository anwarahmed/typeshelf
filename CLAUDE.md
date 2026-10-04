# typeshelf

A keyboard-driven terminal app (TUI) for practicing typing by retyping classic books,
modeled on the web app [TypeLit.io](https://www.typelit.io/). Targets macOS and Linux.

## Commands

```sh
cargo run --release                          # run (debug builds parse big books slowly)
cargo test                                   # unit tests (normalize, parse, paginate, engine)
cargo clippy --all-targets -- -D warnings    # CI fails on any warning
cargo fmt                                    # rustfmt.toml: max_width 160
cargo run --release -- build-catalog .books-cache   # regenerate assets/catalog.json
cargo run --release -- dump <file.md>        # print how a file parses (chapters, groups, text)
```

`build-catalog` and `dump` are maintainer commands, deliberately left out of `--help`.

`install.sh` (POSIX sh, macOS + Linux) downloads the latest release binary into
`~/.local/bin` (`TYPESHELF_BIN_DIR` overrides) and verifies its checksum. `--source`
builds instead (the checkout it is in, else a fresh clone), and it falls back to that
when no binary exists for the platform. `--link` symlinks to the checkout's build,
`--uninstall` removes it. It never installs Rust and never edits shell profiles.
`TYPESHELF_RELEASE_URL` points it at another download base (used to test it against a
local directory with `file://`).

## Resources

- Book source: https://github.com/mlschmitt/classic-books-markdown — 756 public-domain
  books, ~330 MB, one `Author/Title.md` per book. `assets/catalog.json` was built from
  commit `944882722e9a96fa6cf2739dc2ea757e101f412f`; downloads are pinned to the `rev`
  stored in the catalog.
- Reference product: https://www.typelit.io/ (library → book → chapter → page, type
  over the text, WPM and accuracy per page, saved progress).
- UI: [ratatui](https://ratatui.rs) 0.30 with its re-exported crossterm
  (`ratatui::crossterm`); do not add a separate crossterm dependency.
- `.books-cache/` (gitignored) is a local clone of the book repo for catalog builds and
  parser checks: `git clone --depth 1 https://github.com/mlschmitt/classic-books-markdown .books-cache`

## Architecture

Single binary crate, no async. One file per concern in `src/`:

| File           | Role |
|----------------|------|
| `main.rs`      | CLI args, terminal setup/teardown, event loop |
| `app.rs`       | `App` state, all key handling, navigation between screens |
| `ui.rs`        | All drawing. Pure functions of `&App`; never mutates state |
| `engine.rs`    | `Session`: the typing state machine for one page (marks, cursor, stats) |
| `parse.rs`     | Markdown → `Book { chapters: [Chapter { title, group, lines }] }` |
| `paginate.rs`  | Chapter lines → `Page`s of about N characters |
| `normalize.rs` | `normalize_text` (rewrite typography) and `fold` (char → key) |
| `library.rs`   | Embedded catalog, download + disk cache, user texts, catalog builder |
| `store.rs`     | `Settings` and `State` (progress, history, missed keys) as JSON on disk |
| `theme.rs`     | Color themes |
| `update.rs`    | Startup self-update: compare version with the latest GitHub release, download, verify, re-exec |
| `log.rs`       | Append-only trace log and the `log::info!` / `warning!` / `error!` macros |

Flow: `Library` (catalog) → open book → `parse_book` → pick chapter → `paginate` →
`Session` per page → on completion `App::complete_page` writes a `Record` and progress.

### Patterns to keep

- **State / view split.** `app.rs` owns state and input; `ui.rs` only reads. Lists are
  stateless: the visible window is derived from the selection each frame
  (`ui::window`), there is no stored scroll offset.
- **The engine is pure.** `Session` takes timestamps as arguments (`now_ms`) and has no
  I/O, so it is unit-tested directly. Keep terminal and clock out of it.
- **Event loop.** `main::run` polls with a 200 ms timeout (keeps the clock, live stats
  and download status moving) and drains all queued keys before redrawing so fast
  typing never lags. Downloads run on a thread and report over an mpsc channel
  collected in `App::tick`.
- **Themes carry roles, not colors.** Widgets use `Styles` (`base`, `dim`, `accent`,
  `error`, `fixed`, `good`, `sel`) built from the theme. Text is never colored with
  the accent except for keys, titles of panels and marks.
- **Tests live beside the code** in `#[cfg(test)] mod tests`. Parser changes should be
  checked against the whole corpus too (see "Verifying changes").

## Decisions and why

- **Rust + ratatui.** The user asked for a TUI that runs on macOS and Linux with a
  polished keyboard-centric interface; a single static binary with no runtime was the
  deciding factor. (A web version was started first and discarded when the user
  clarified they wanted a TUI.)
- **Books are not bundled.** The corpus is ~330 MB. Only the catalog (114 KB of
  metadata) is embedded via `include_str!`; a book is fetched from
  `raw.githubusercontent.com` the first time it is opened and cached under
  `~/.cache/typeshelf/books/`. `typeshelf sync` fetches everything; `TYPESHELF_BOOKS` /
  `books_dir` point at an existing clone instead.
- **Book identity is its repo path** (`Author/Title.md`); user texts are `my/<slug>.md`.
  This is the key in `State::progress` and `Record::book`.
- **XDG-style paths on both platforms** (`~/.config`, `~/.local/share`, `~/.cache`),
  not `~/Library/...` on macOS, so the app behaves the same everywhere and needs no
  `dirs` crate.
- **Parsing: any heading starts a section.** The corpus shares a header (`# Title:`,
  `## Author:`, `## Year:`, dashed rule) but uses heading levels inconsistently (only
  `##`, only `###`, only `#`, or none — 88 books have no headings). A heading with no
  body text becomes a *group label* for the chapters after it ("ACT I", "BOOK II"),
  including when it is at the same level as the chapters. A book that ends up as one
  chapter over 45k characters is split into "Part N" chunks.
- **Prose vs verse.** Consecutive non-blank lines are joined into one paragraph only if
  they look hard-wrapped (median line length ≥ 58, no markdown hard breaks);
  otherwise each line is kept, which preserves poetry and plays.
- **Headings are not typed.** They are shown as the chapter title only.
- **Typeability.** `normalize_text` rewrites what has a plain multi-character
  equivalent (`…` → `...`, `æ` → `ae`, curly quotes → straight). `fold` maps a displayed
  character to the key that types it (`é` → `e`, `—` → `-`); characters with no key
  (`None`) are auto-skipped by the cursor in both directions. Typed input is folded
  too, so an international layout or smart punctuation still matches.
- **Progress is a character offset per chapter**, not a page number. Pages depend on
  the page-length setting; `Page::end` offsets let progress survive changing it.
  `BookProgress` also stores chapter lengths so the library can show progress without
  loading the book. Paginating never changes a chapter's total length (a split
  consumes the space it breaks at) — a test asserts this.
- **Resuming mid-page.** Leaving an unfinished page (`esc`, `ctrl-c`, `ctrl-n/p`)
  stores `page start + cursor` as the chapter offset (`App::save_partial`). Opening the
  chapter lands on the page containing that offset and `Session::resume_at` pre-marks
  the cells before it. Those cells are excluded from the session's stats and can't be
  backspaced over. `ctrl-r` restarts the page from its top.
- **Two offsets per chapter.** `BookProgress::off` is the furthest point reached (only
  moves forward; drives progress bars and "done"). `BookProgress::cur` is where the
  cursor was last left, and is what a chapter reopens at — it moves back when a
  finished chapter is typed again. Keeping only `off` broke resuming in a chapter that
  was already complete. A `cur` at the chapter's end means "start from the top".
- **Mistakes.** Default is free typing: a wrong key is marked and the cursor moves on;
  backspace fixes it. `stop_on_error` makes the cursor wait instead. Characters fixed
  after a mistake are shown in the `fixed` color. A page completes when the cursor
  reaches the end, corrected or not.
- **Metrics.** WPM = correct characters / 5 / active minutes. Active time sums the gaps
  between keystrokes, each capped at 5 s, so stepping away does not tank the number.
  Accuracy = correct keystrokes / all keystrokes (backspaces are free).
- **Releases are versioned; a release is cut by merging a version bump.** The user
  chose this over "every merge to main" because the repo is public: other people get a
  stable target, bug reports name a version, and nothing half-finished ships. To
  release: bump `version` in `Cargo.toml` (and `Cargo.lock`, via any cargo command) in
  a PR. On merge, `.github/workflows/release.yml` sees there is no `v<version>` tag,
  builds four binaries, and publishes a GitHub release with them, `SHA256SUMS` and the
  rendered `PKGBUILD`. Merges that don't change the version release nothing. The same
  workflow runs build-only on PRs that touch packaging, to prove all targets compile.
- **Release assets are bare binaries**, named `typeshelf-<rust target>`, not archives:
  `x86_64-` and `aarch64-unknown-linux-musl` (static, so one file runs on every
  distro), `aarch64-` and `x86_64-apple-darwin`. Bare files mean the updater and
  `install.sh` need no tar/gzip code. Renaming assets breaks installed copies' updates,
  the install script, the Homebrew formula and the AUR package at once.
- **Self-update on start** (asked for by the user). `update::before_start` asks the
  GitHub API for the latest release (3 s timeout, silent when offline); if its version
  is higher than `CARGO_PKG_VERSION` it downloads the asset for this platform, checks
  it against `SHA256SUMS`, renames it over the running binary and re-execs with
  `TYPESHELF_NO_UPDATE=1` so it can't loop. Any failure keeps the current version.
  It deliberately does **not** update: builds run from a checkout's `target/` (incl.
  `install.sh --link`), Homebrew installs (path contains `Cellar`), copies whose
  directory isn't writable (pacman-owned `/usr/bin`), platforms with no asset, or when
  the setting / env var is off. It never downgrades. `build.rs` still stamps the commit,
  but only for `--version` and the log.
- **Packaging.** Three channels, all fed by the release:
  - *Install script* - the universal path, above.
  - *Homebrew* - a separate repo, `anwarahmed/homebrew-tap` (Homebrew requires the
    `homebrew-` name). Its own workflow regenerates `Formula/typeshelf.rb` from the
    latest release on a schedule, and tests `brew install` on macOS and Linux. It lives
    apart because the formula needs the release's checksums, which only exist after
    the merge, and `main` here only accepts PRs.
  - *AUR* - package `typeshelf-bin`. `packaging/aur/render.sh` fills `PKGBUILD.in` and
    `SRCINFO.in` per release. `.SRCINFO` has its own template because releases build on
    Ubuntu, which has no `makepkg`; if you change one template change the other, and
    check with `makepkg --printsrcinfo | diff - .SRCINFO` on Arch. The workflow pushes
    to the AUR only if the `AUR_SSH_PRIVATE_KEY` secret exists.
  Considered and not done: Nix flake, crates.io, `.deb`/`.rpm`, Snap, Flatpak.
- **Levels.** TypeLit has ranks; here `State::level` derives a level from total
  characters typed (level `n` at `500 * n * (n - 1)`). It is computed from history,
  never stored. `State::averages` gives per-book and per-chapter speed for the book
  screen, mirroring TypeLit's stats "on every page, chapter, and book".
- **Imported texts get a header.** `Library::import` prepends `# Title: <file name>`
  when the file has none, so the title survives the slugged file name on disk.
- **Cursor** is the real terminal cursor (`Frame::set_cursor_position`), so it keeps
  the user's shape and blink unless overridden in settings.
- **Default theme "terminal"** uses only ANSI palette colors and no background, so it
  matches the terminal's theme (the user runs Omarchy, which themes the terminal) and
  works in terminals without truecolor, e.g. macOS Terminal.app.
- **Charts** (stats screen) are single-series in the accent color, with the recent
  pages table beside them as the exact-value view.
- **Minimum Rust is 1.88** (`rust-version` in `Cargo.toml`): the code uses let-chains
  and ratatui 0.30 needs it. An older toolchain gets Cargo's clear "requires rustc
  1.88" message instead of a syntax error. CI's `test` jobs use latest stable, so a
  separate `msrv` job builds and tests on exactly 1.88; raise both together.
- **No argument-parsing or regex crates**; the CLI is three commands and the markdown
  cleanup is a small hand-written scanner in `parse::clean_inline`.

## Logs

When something misbehaves, read the log first. A build run from `target/` writes
`logs/typeshelf.log` in this checkout (gitignored); an installed binary writes
`~/.local/state/typeshelf/typeshelf.log`; `TYPESHELF_LOG` overrides both. It rotates to
`.log.1` past 1 MB.

It records: startup (version, OS, TERM, settings), every book open (chapters, parse
time, saved progress), downloads, each typing session start (chapter, page, saved
cursor, resume cell), pages left part-way and completed, setting changes, every footer
message shown to the user (`toast:` lines, which is where errors surface), and panics
with a backtrace. It never records typed keys or book text. When adding a feature
that changes saved state or can fail, log the decision inputs, as `start_typing` does.

The user's own progress is in `~/.local/share/typeshelf/state.json` — the bug where
resuming failed was found by comparing that file with what the code assumed.

## Licensing and legal

- Code: MIT (`LICENSE`, `license` in `Cargo.toml`), copyright Anwar Ahmed. All
  dependencies are permissive (MIT / Apache-2.0 / ISC / Zlib / BSD / Unicode).
- Books are not redistributed: only `assets/catalog.json` (titles, authors, years,
  sizes) is in the repo; texts are fetched from the upstream repo at run time. That
  repo has no license file and asserts "all titles from the public domain"; some titles
  (1930s-60s science fiction, Christie, Wodehouse, Huxley) are public domain in the US
  but not everywhere. Do not commit book texts (`.books-cache/` stays gitignored).
- TypeLit: no code, text, assets, branding or data from typelit.io was used; it was
  consulted only as a description of the idea (homepage, FAQ, about page). The name
  "TypeLit" must only appear as a factual reference with the non-affiliation note in
  the README, never as this app's name or in its UI.

## Verifying changes

There is no automated UI test. Drive the real app in a detached tmux session with
throwaway data directories, then read the screen:

```sh
X=$(mktemp -d)
tmux new-session -d -s ts -x 120 -y 34 \
  "XDG_CONFIG_HOME=$X/c XDG_DATA_HOME=$X/d XDG_CACHE_HOME=$X/k ./target/release/typeshelf"
tmux send-keys -t ts '/' ; tmux send-keys -t ts -l 'pride' ; tmux send-keys -t ts Enter Enter
tmux capture-pane -t ts -p        # add -e to see colors
tmux kill-session -t ts
```

Also resize to something tiny (`tmux resize-window -t ts -x 8 -y 3`) — layout math must
use saturating arithmetic and never panic.

For parser changes, run `dump` over the corpus and look for leftover markup, and
re-run `build-catalog` (chapter counts and lengths in the catalog come from the parser).

## Known gaps and ideas

- A page left unfinished saves the cursor position but no `Record`: its speed and
  accuracy only enter the stats for the part typed in the sitting that finishes it.
- Progress is saved on leaving a page or quitting with `ctrl-c`, not on every key, so a
  killed terminal loses the current page's position.
- Help is not reachable from the typing screen (`?` is a typed character); the typing
  keys are listed in the footer and in the help of the other screens.
- Search is a plain substring match on title and author and is accent-sensitive.
- `books_dir` can only be set by editing the config file or via `TYPESHELF_BOOKS`.
- TypeLit features not built: achievements, named ranks, non-English libraries
  (the book repo is English only), accounts and cross-device sync, visual effects.
- Not built: accounts or
  sync between machines, per-key heatmap on a keyboard layout, light/dark auto-switch.
