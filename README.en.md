# Waypinyin

> **公开源码版 / Public source edition:** GitHub 仓库仅包含源码与原创演示词库，
> 不包含历史第三方词库及其 Git 历史。默认可构建并运行演示；完整词库通过
> `WAYPINYIN_SOURCE_DATA` 在本地提供。详见 [公开源码说明](docs/PUBLIC_SOURCE.md)。
> The public repository builds a small original demo dictionary, not the full
> daily-use corpus. See [source/data separation](docs/PUBLIC_SOURCE.md).

Offline Chinese Pinyin input through IBus, primarily for GNOME Wayland.
See [中文说明](README.md) for input features, fuzzy Pinyin and import formats.

This is pre-release software. Original code is MIT licensed. Third-party
historical full-lexicon provenance is still being audited. The public source
excludes that data and its old history, and builds an original MIT demo instead.
**The historical complete corpus and its compiled images are not cleared for
redistribution.** See
[data/NOTICE](data/NOTICE), [data/sources.json](data/sources.json) and
[release blockers](docs/RELEASE.md).

## Build and install

Requires Rust 1.89.0 (the currently tested minimum), Cargo, Python 3.11+, GNU make,
a C compiler/linker and development packages for Fontconfig and the GUI stack.
Runtime requires IBus, a Wayland or X11 desktop, OpenGL/EGL, Fontconfig and a
Chinese font (for example Noto CJK). The file picker uses the desktop portal.

```sh
make build
./build/waypinyin --help
WAYPINYIN_LEXICON="$PWD/build/lexicon.bin" ./build/waypinyin --check-data
make install PREFIX=/usr/local DESTDIR="$PWD/build/stage"
```

`make install` only installs existing artifacts; it does not compile or access the network.
Run `make build` with the final installation paths first.

`DESTDIR` stages files only. To install on the host, omit `DESTDIR` and run the
install step with the required directory permissions. Keep build and install
`PREFIX`/`DATADIR` identical. Custom `BINDIR` and `MANDIR` are supported. IBus
component discovery for a custom data prefix depends on the desktop environment;
`PREFIX=/usr` is the distribution default. The installer does not restart IBus
or change your input sources. Restart the session, then add Waypinyin to GNOME's
keyboard input sources. Do not use `cargo install` alone: it omits the lexicon
and IBus registration files.

`cargo build` only writes the generated image to Cargo's `OUT_DIR`. Debug builds
can find that image automatically. Release builds use the configured installed
data directory; for development examples pass `WAYPINYIN_LEXICON` explicitly:

```sh
make check
WAYPINYIN_LEXICON="$PWD/build/lexicon.bin" cargo run --release --example quality
WAYPINYIN_LEXICON="$PWD/build/lexicon.bin" cargo run --release --example sentence_quality
```

`CARGO_BUILD_FLAGS='--locked --offline'` supports building against an already
populated Cargo cache. Distribution builders must supply dependencies through
their system Cargo registry. Do not download dependencies in the build sandbox.
Use `CARGO_BUILD_FLAGS='--offline --no-default-features'` to omit egui's bundled
fonts; the settings window then requires a usable Fontconfig match.

## Data and privacy

Configuration: `$XDG_CONFIG_HOME/waypinyin/config.toml` (default `~/.config`).
Imported dictionaries and learning: `$XDG_DATA_HOME/waypinyin/` (default
`~/.local/share`). Empty or relative XDG values are ignored. Learning logs may
contain personal input choices. When the data directory changes from the
historic default, the old learning log is copied only if the new one does not
exist; the original is retained. Migration failures are reported and disable
learning for that session rather than overwriting data.

Only use trusted compiled lexicon images. Do not modify or truncate images
while processes map them. Installed binaries do not need the source/build tree.
The image uses native little-endian records and must be generated for the target
architecture. Cross compilation and big-endian targets are not supported yet.

Validation: [docs/VALIDATION.md](docs/VALIDATION.md).
Contributing: [CONTRIBUTING.md](CONTRIBUTING.md). Packaging:
[docs/PACKAGING.md](docs/PACKAGING.md). Security: [SECURITY.md](SECURITY.md).


## Linux performance

The read-only lexicon mapping uses sequential readahead during validation and
random-access advice during lookup. Inotify watches cache imported dictionaries
and live preferences, with periodic reconciliation and polling fallbacks.
Candidate generation reuses segmentation and immutable plans, and reduces beam
search and phrase-enumeration allocations while retaining search and ordering
rules. See [performance notes](docs/PERFORMANCE.md) for reproducible probes and local
measurements.


Only the current five-candidate page is materialized before presentation. A
single idle worker preloads initial-letter pages and likely next-key pages into
a bounded cache. Hits share the first page and defer continuation copying until
paging; personal and imported candidates are still merged live. Foreground
requests never wait for the worker. See the first-page probe in the performance
notes for the cache policy and measurements.


## Frequent-input learning

Confirmed candidate commits now count the actual Pinyin or abbreviation used
(e.g. `nihao` and `nh` separately). Canceled drafts and unconverted Return commits
are excluded. Up to 128 active patterns share the local `user.txt` learning log;
the top 16 repeated patterns and their prefixes receive preload priority. Existing
word-learning records seed warming too. Hotness halves every 512 learning-clock
steps; candidate ordering still uses live learning/import overlays.

Nothing is uploaded. Password/PIN purposes and private/hidden IBus hints disable
learning and new preload requests; mixed-context compositions remain unlearned.
New logs and compaction files use owner-only permissions. The append log may have
more historical rows than the active-pattern limit until startup compaction.
