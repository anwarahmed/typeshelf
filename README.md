# typeshelf

Practice typing in your terminal by retyping classic books, a page at a time: a
keyboard-driven TUI for macOS and Linux. The idea comes from
[TypeLit.io](https://www.typelit.io/); typeshelf is an independent project, not
affiliated with or endorsed by TypeLit, and shares no code or content with it.

- 756 public-domain books from
  [classic-books-markdown](https://github.com/mlschmitt/classic-books-markdown),
  downloaded on demand and cached
- Your own texts: `typeshelf notes.txt`
- Progress saved down to the character; reopen a chapter and the cursor is where you left it
- Speed and accuracy per page, chapter and book; daily volume, most-missed keys and levels
- Themes, including one that follows your terminal's own colors

## Install

Pick whichever suits your machine. Each installs the `typeshelf` command.

### Any Linux or macOS

```sh
curl -fsSL https://raw.githubusercontent.com/anwarahmed/typeshelf/main/install.sh | sh
```

This downloads the latest release for your system into `~/.local/bin` and checks it
against the release's checksums. Nothing else needs to be installed first. The Linux binaries are statically linked, so the same file
runs on any distribution, on x86-64 and ARM.

If the script says `~/.local/bin` is not on your `PATH` (it isn't by default on macOS),
add it and open a new terminal:

```sh
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.zshrc    # zsh (the macOS default)
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.bashrc   # bash
```

### Homebrew (macOS and Linux)

```sh
brew install anwarahmed/tap/typeshelf
```

The tap picks up a new release within a few hours of it being published.

### Arch Linux (pacman package)

Each release includes a `PKGBUILD`, so typeshelf can be installed as a regular pacman
package. This needs `base-devel` (`sudo pacman -S --needed base-devel`).

```sh
mkdir -p typeshelf-bin && cd typeshelf-bin
curl -fsSLO https://github.com/anwarahmed/typeshelf/releases/latest/download/PKGBUILD
makepkg -si
```

It is not in the AUR yet (new AUR account registration is currently closed). A copy
installed this way does not update itself; run the same three commands again to move
to a newer release.

### From source

Needs Rust 1.88 or newer, git and a C compiler.

```sh
git clone https://github.com/anwarahmed/typeshelf.git && cd typeshelf
./install.sh --source
```

That builds the checkout and copies the result to `~/.local/bin`. For development, use
`./install.sh --link` instead: it symlinks to the checkout's build, so each
`cargo build --release` is picked up without reinstalling.

`TYPESHELF_BIN_DIR` changes where any of the script's modes install to.

### Updating

| Installed with | How it updates |
|----------------|----------------|
| The install script, including `--source` | By itself. Each time typeshelf starts it checks for a newer release and, if there is one, downloads it and restarts — a few seconds. Offline, it just starts. |
| Homebrew | `brew upgrade typeshelf` |
| pacman package | Run the three install commands again |
| `--link`, or running from a checkout | `git pull && cargo build --release` |

For a copy that updates itself, `typeshelf update` checks on demand. To stop the
automatic check, turn off Settings → "Update on start" or set `TYPESHELF_NO_UPDATE=1`.
Updates are verified against the release's SHA-256 checksums, and never move to an
older version.

### Uninstalling

Use the line that matches how you installed it:

```sh
rm ~/.local/bin/typeshelf         # install script (or ./install.sh --uninstall)
brew uninstall typeshelf          # Homebrew
sudo pacman -R typeshelf-bin      # pacman package
```

Your settings, progress and cached books stay where they are; see [Files](#files) if you
want to remove those too.

## Use

```
typeshelf                 open the library
typeshelf <file>          add a text or markdown file to your texts and open it
typeshelf sync            download every book for offline use (about 330 MB)
typeshelf update          check for a newer release now and install it
typeshelf --help          list commands and environment variables
typeshelf --version       print the version and the commit it was built from
```

Press `?` for the keys of the screen you are on (everywhere except while typing, where
`?` is a character to type). The short version:

| Where    | Keys |
|----------|------|
| Lists and menus | `1` `2` `3` library / stats / settings · `j` `k`, arrows or the mouse wheel move · `g` `G` first / last · `ctrl-d` `ctrl-u` jump · `q` back |
| Library  | `enter` open · `/` search · `tab` switch shelf · `s` sort · `c` continue last book · `d` delete one of your texts · `q` quit |
| Book     | `enter` type chapter · `c` continue where you left off · `r` reset progress · `esc` back |
| Typing   | `backspace` fix · `ctrl-w` or `alt-backspace` delete word · `ctrl-r` restart page · `ctrl-n` / `ctrl-p` next / previous page · `esc` back to chapters |
| Anywhere | `ctrl-c` quit |

Leaving a page part-way keeps your place: the chapter reopens with the cursor where
you stopped.

While typing: curly quotes, dashes and accented letters are typed with their plain
keys (`é` is `e`, `—` is `-`), and characters with no key at all (Greek, `£`) are
skipped for you. Line breaks take `enter` (or `space`, unless you turn that off).

## Files

| What          | Where                                   |
|---------------|-----------------------------------------|
| Settings      | `~/.config/typeshelf/config.json`       |
| Progress      | `~/.local/share/typeshelf/state.json`   |
| Your texts    | `~/.local/share/typeshelf/texts/`       |
| Book cache    | `~/.cache/typeshelf/books/`             |
| Log           | `~/.local/state/typeshelf/typeshelf.log` |

The same paths are used on macOS; `XDG_CONFIG_HOME`, `XDG_DATA_HOME`,
`XDG_CACHE_HOME` and `XDG_STATE_HOME` are honored. If you already have a clone of the books repo, point
`TYPESHELF_BOOKS` (or `books_dir` in the settings file) at it and nothing is downloaded.

## Troubleshooting

typeshelf keeps a log of what it did (books opened, downloads, update checks, where a
page was left, errors, crashes with a backtrace) — never what you typed. It lives at
`~/.local/state/typeshelf/typeshelf.log`, or `logs/typeshelf.log` in the checkout when
you run a build straight from `target/`. Set `TYPESHELF_LOG` to put it elsewhere.

## Development

```sh
cargo run --release
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt
```

Changes reach `main` through pull requests. To publish a release, bump `version` in
`Cargo.toml` in a pull request; when it merges, the binaries are built and released
automatically. [RELEASING.md](RELEASING.md) is the full procedure, including the checklist
a release pull request has to carry. See [CLAUDE.md](CLAUDE.md) for the architecture and
the decisions behind it.

## License

The code is released under the [MIT License](LICENSE).

The books are not part of this repository and are not covered by that license. The app
embeds only a catalog (titles, authors, years and lengths), and downloads a book's text from
[classic-books-markdown](https://github.com/mlschmitt/classic-books-markdown) when you
open it. That collection describes its titles as public domain; it includes works from
as late as the 1960s whose status depends on your country, so check before
redistributing any of the texts yourself.
