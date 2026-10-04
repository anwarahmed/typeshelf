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
- Speed, accuracy, daily volume and most-missed keys
- Themes, including one that follows your terminal's own colors

## Install

typeshelf is built from source on your machine, so it needs a Rust toolchain (1.88 or
newer), git and a C compiler. The steps below set those up and then install typeshelf to `~/.local/bin`.

### macOS

1. Install Apple's command line tools (git and a C compiler). Skip this if
   `xcode-select -p` already prints a path.

   ```sh
   xcode-select --install
   ```

2. Install Rust, then open a new terminal so `cargo` is found.

   ```sh
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   ```

3. Install typeshelf.

   ```sh
   curl -fsSL https://raw.githubusercontent.com/anwarahmed/typeshelf/main/install.sh | sh
   ```

4. Put `~/.local/bin` on your `PATH` (macOS doesn't by default), then open a new
   terminal.

   ```sh
   echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.zshrc
   ```

### Linux

1. Install git, curl and a C compiler.

   ```sh
   sudo apt install build-essential git curl      # Debian, Ubuntu
   sudo dnf install gcc git curl                  # Fedora
   sudo pacman -S --needed base-devel git curl    # Arch
   ```

2. Install Rust, then open a new terminal so `cargo` is found. (Your distribution's
   `rust` package works too if `rustc --version` reports 1.88 or newer; many stable
   distributions ship something older.)

   ```sh
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   ```

3. Install typeshelf.

   ```sh
   curl -fsSL https://raw.githubusercontent.com/anwarahmed/typeshelf/main/install.sh | sh
   ```

4. Most distributions already have `~/.local/bin` on the `PATH`. If the installer says
   it isn't, add it and open a new terminal.

   ```sh
   echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.bashrc
   ```

### Then

```sh
typeshelf
```

The first build takes a minute or two. To check the install: `typeshelf --version`.

### Updates

typeshelf keeps itself current. Each time it starts it checks GitHub for a newer
version and, if there is one, rebuilds and restarts — a minute or two, and only when
something changed. Without a network connection it just starts.

- `typeshelf update` runs the same check on demand.
- Turn the automatic check off under Settings → "Update on start", or with
  `TYPESHELF_NO_UPDATE=1`.

### From a clone of the repo

```sh
git clone https://github.com/anwarahmed/typeshelf.git && cd typeshelf
./install.sh              # build and copy to ~/.local/bin
./install.sh --link       # symlink to this checkout's build instead (for development)
./install.sh --uninstall  # remove it; settings and progress are kept
```

Set `TYPESHELF_BIN_DIR` to install somewhere other than `~/.local/bin`. A linked
install does not update itself; use `git pull` and `cargo build --release`.

### Uninstall

```sh
rm ~/.local/bin/typeshelf
```

Your settings, progress and cached books stay where they are; see [Files](#files) if you
want to remove those too.

## Use

```
typeshelf                 open the library
typeshelf <file>          add a text or markdown file to your texts and open it
typeshelf sync            download every book for offline use (about 330 MB)
typeshelf update          check for a newer version now and install it
```

Press `?` on any screen for its keys. The short version:

| Where    | Keys                                                                          |
|----------|-------------------------------------------------------------------------------|
| Anywhere | `1` `2` `3` library / stats / settings · `j` `k` move · `g` `G` ends · `ctrl-c` quit |
| Library  | `enter` open · `/` search · `tab` switch shelf · `s` sort · `c` continue last book   |
| Book     | `enter` type chapter · `c` continue · `r` reset progress · `esc` back                |
| Typing   | `backspace` fix · `ctrl-w` delete word · `ctrl-r` restart page · `ctrl-n` / `ctrl-p` next / previous page · `esc` back |

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

The same paths are used on macOS; `XDG_CONFIG_HOME`, `XDG_DATA_HOME` and
`XDG_CACHE_HOME` are honored. If you already have a clone of the books repo, point
`TYPESHELF_BOOKS` (or `books_dir` in the settings file) at it and nothing is downloaded.

## Troubleshooting

typeshelf keeps a log of what it did (books opened, downloads, where a page was left,
errors, crashes with a backtrace) — never what you typed. It lives at
`~/.local/state/typeshelf/typeshelf.log`, or `logs/typeshelf.log` in the checkout when
you run a build straight from `target/`. Set `TYPESHELF_LOG` to put it elsewhere.

## Development

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt
```

See [CLAUDE.md](CLAUDE.md) for the architecture and the decisions behind it.

## License

The code is released under the [MIT License](LICENSE).

The books are not part of this repository and are not covered by that license. The app
embeds only a catalog of titles and authors, and downloads a book's text from
[classic-books-markdown](https://github.com/mlschmitt/classic-books-markdown) when you
open it. That collection describes its titles as public domain; it includes works from
as late as the 1960s whose status depends on your country, so check before
redistributing any of the texts yourself.
