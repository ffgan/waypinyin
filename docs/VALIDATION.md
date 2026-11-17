# Validation record — 2026-11-14

Local environment: Fedora 45 x86_64, rustc/cargo 1.99.1. This records checks
actually performed during upstream preparation; it is not distribution approval.

## Passed

- `cargo fmt --all -- --check`, `git diff --check`, and
  `cargo clippy --locked --offline --all-targets -- -D warnings`.
- `cargo test --locked --offline`: 64 library tests and 2 CLI integration tests
  passed. One external native-dictionary test was skipped because its optional
  files were not supplied. Existing input regression tests were retained.
- `cargo check --locked --offline --no-default-features`: system-font build.
- Standard release build and installations staged for `/usr/local` and `/usr`.
  Installed files, generated IBus paths and desktop metadata were validated.
- A real installation into a project-local absolute prefix passed `--check-data`
  with `WAYPINYIN_LEXICON` unset, current directory `/tmp`, and the entire Cargo
  `target/` directory temporarily hidden. It did not need the build-tree image.
  No installation into system directories was performed.
- Header, section bounds, indexes, UTF-8 and string-offset corruption tests;
  explicit missing dictionary override and invalid command-line tests.
- XDG absolute/relative/empty path handling; migration preserves existing data,
  retains the old file and reports failure without creating a destination.
- Two fresh dictionary compiler processes produced byte-identical images:
  SHA-256 `58d36908ff619f807bcaae1ce1316d819904c808ce0ce081fb15b97d9bd75b24`.
- Review source archives generated twice from identical inputs had identical
  SHA-256 values. An extracted archive rebuilt offline after forcing its build
  script and library to recompile, using the populated local dependency cache.
- All ten data hashes matched the provenance inventory. The release gate rejected
  normal `make dist` due to nine unresolved source records, as intended.
- Python helper scripts passed syntax compilation. The Fedora spec passed
  `rpmspec --parse`; this only checks parseability, not a complete RPM build.

## Input results

The pre-change debug dictionary image (retained from the existing worktree) and
new deterministic image were loaded by the same release quality examples.

- Common-word corpus: 200/200 top choice and 200/200 top seven for both images.
- Sentence corpus: 74/75 top choice and 74/75 top seven for both images.
  Every reported per-sentence candidate was identical.
- Common-word query P95: 299 microseconds for both runs. Sentence per-key P95:
  12,675 before and 12,665 microseconds after. These single-machine timings are
  a sanity check, not a statistically established performance improvement.

## Blocked or not performed

A disposable Debian trixie container's package indexes offered Rust/Cargo
1.85.1 and zbus 5.3.0; the current project declares Rust 1.99.1 and zbus 6.
The eframe/rfd package queries returned no candidate. This suite cannot satisfy
the current draft directly. No assertion is made about Debian unstable or a
future Ubuntu suite without checking their registries.

The Fedora host lacks the complete Cargo RPM macros and system crate registry.
A parseable spec is insufficient: `%cargo_prep`, generated crate requirements,
`mock` builds and `rpmlint` remain unverified. Full isolated source/binary package
builds, lintian, package install/upgrade/remove testing, native aarch64 builds,
full ELF reproducibility and GNOME Wayland desktop acceptance were not performed.

The remote CI workflow has been added but has not run on a hosted repository.
Data provenance, dependency/font notices and publication identity remain release
blockers listed in [RELEASE.md](RELEASE.md). No release, tag, registry upload,
official package submission or git commit was made.


## Public source preparation — 2026-11-15

The publication tree is exported into a separate directory without `.git` or
historical dictionary bodies. Its inventory has one original sentence data file
and nine original demo input files, with no unresolved included data records.
The retained excluded-file metadata does not contain dictionary entries.
The original complete local checkout and its history remain unchanged in place.

The demo build passes parser/configuration/image/IBus signature/session smoke
and CLI tests, strict Clippy and staged `/usr` installation. The complete local
corpus regression suite also passes after adding the source-selection interface.
`--version` and `--check-data` distinguish demo from external/full-data builds.
No tagged binary release or distribution inclusion is claimed.

### Linux kernel performance — 2026-11-16

Imported dictionaries and live preferences use nonblocking, close-on-exec
inotify directory watches. Unchanged input no longer scans dictionary metadata
or repeatedly opens the configuration file. Periodic dictionary reconciliation
retains unchanged parsed records. Missing directories watch existing ancestors;
errors retain polling/read fallbacks. Settings still take effect at composition
boundaries and invalidate cached plans.

Lexicon mapping now uses sequential-access advice during full validation and
random-access advice during lookup. All image validation remains enabled.

See [PERFORMANCE.md](PERFORMANCE.md) for the reproducible probe, cache controls,
local measurements and limits. In seven local disk-cold samples per variant,
load medians fell from 506 ms to 153 ms. This is a local startup observation,
not a guarantee about arbitrary input latency.


### Candidate latency optimization — 2026-11-16

- `cargo test --locked --offline --release`: 76 library tests and three CLI
  integration tests passed; the optional external native-dictionary test was
  skipped because its fixtures were not supplied.
- The test-only original algorithms matched the optimized sentence beam over
  1,174 corpus prefixes and additional boundaries; phrase enumeration matched
  through 4,096 outputs and the packed/vector boundary. Umlaut matching and the
  deferred learning floor also matched their references.
- Shared-plan pagination and configuration-boundary invalidation tests passed.
- The original demo fixture with `--no-default-features` passed 26 library tests
  and three CLI tests, with the same optional external-fixture test skipped.
- Strict all-target Clippy, formatting and diff whitespace checks passed. Python
  probe syntax and trace comparator acceptance/rejection checks passed.

The [performance notes](PERFORMANCE.md) describe isolated alternating probes,
raw samples and the pre-existing mixed-spelling `suggest()` ordering variation.

The final seven alternating input-probe pairs measured P95 reductions of 21.3%
for short-word keys, 51.3% for sentence keys, 18.1% for full candidate enumeration
and 43.2% for warmed replays. The 7,564 stable trace records matched exactly;
one pre-existing order variation matched an observed baseline variant. Raw
samples and source/probe hashes are retained in `benchmarks/input-2026-11-16.json`.


### First-page preload — 2026-11-16

The first view now materializes five candidates instead of two pages. A lazy
phrase cursor retains complete ordering without eagerly enumerating 4,096
combinations. One idle worker prepares dictionary-only pages under entry/owned
payload limits; configuration keys are separated and clock shortcuts excluded.
First-page data is shared, with continuation copying deferred until paging.
Personal/imported results remain live foreground overlays.

- Full release suite: 82 library and three CLI tests passed; one optional
  external-fixture test skipped. Demo/system-font suite: 31 library and three
  CLI tests passed, with the same fixture skip.
- Tests cover queue bounds/priorities, cache budgets/eviction, unchanged caller
  scheduling policy, preference separation, nonblocking cache access, deferred
  continuation/copy-on-write, learning overlays and lazy phrase equivalence.
- Visible-page traces: 7,564 stable records matched exactly; one existing public
  `suggest()` ordering variation matched an observed original variant. Hidden
  lookahead is intentionally excluded from the comparison; all visible fields
  and selected exhaustive page streams remain covered.
- Seven alternating pairs per idle-gap condition retained 200/200 first-choice
  words. With two-ms inter-key idle time, first-page P50 fell to 5.891 microseconds
  and P95 to 69.450 microseconds; initial-key P95 was 18.414 microseconds.
  No-idle and supplemental scheduling/resource observations are retained too.

See [PERFORMANCE.md](PERFORMANCE.md) and
`benchmarks/first-screen-2026-11-16.json` for the conditions and raw evidence.


### Frequent-input history — 2026-11-16

Confirmed-input frequency records now round-trip in the existing user journal,
including exact abbreviations, bounded eviction and compaction. Tests cover draft
and Return exclusion, partial-confirmation separators, decay, malformed records,
legacy seed compatibility, private/mixed-context suppression and IBus content
metadata. Preloader tests verify favorite prefix/full-query ordering, eviction
preference and actual idle preparation of a frequent query before it is typed.

The shared-page test was made race-safe: it verifies an actual cache hit rather
than assuming a readiness check guarantees the next nonblocking lookup succeeds.
The cache is intentionally allowed to miss under lock contention or eviction.

Full release and demo/system-font regression checks, CLI tests and strict Clippy
passed. One optional external native-dictionary fixture remains unavailable.


### Distribution normalization follow-up — 2026-11-16

The native locked dependency graph now supports Rust 1.89.0 and zbus 5.19.
On Rust 1.89.0, all 92 full-corpus library tests and three CLI tests passed;
one external native-dictionary test remained skipped. The demo/system-font
variant passed 41 library tests and three CLI tests with the same fixture skip.
Strict all-target Clippy on 1.99.1, formatting and whitespace checks passed.

An actual IBus daemon on a private session bus discovered the generated engine
XML, started Waypinyin and delivered preedit, lookup-table and commit signals.
Typing `nihao` committed `你好`. Both the original zbus 6 build and the adapted
zbus 5 build passed this probe; no active desktop engine or user files changed.
This is protocol acceptance, not visual GNOME Wayland acceptance.

Release installation with system fonts passed under `/usr` and custom BINDIR
(containing spaces)/MANDIR layouts, with Cargo disabled during installation.
The compiled-artifact notice inventory contains 231 packages, including host
and build dependencies; each registry package has retained notice files.
The installer and verifier preserve/check those files. Supplemental notices
were re-fetched from pinned source revisions and matched their recorded hashes.
The dictionary remains byte-identical to the earlier baseline: SHA-256
`58d36908ff619f807bcaae1ce1316d819904c808ce0ce081fb15b97d9bd75b24`.
Cleared source archives generated twice from the same inputs matched exactly.

The unchanged OpenCC table and FrequencyWords counts were matched to fixed
upstream revisions and their notices retained. The complete historical corpus
still fails the release gate with seven unresolved records. A history-free
export includes three reviewed data files plus nine original demo inputs and
passes the data release gate. No historical dictionary input bytes changed.
The exact pinyin-data revision was recovered, but its merged sources require
additional attribution/license review. The maintainer could not establish the
origin of the 19 local terms; those remain unresolved.

Debian unstable was queried in a disposable container: Rust/Cargo 1.98.1,
zbus 5.19, rfd 0.18 and plist 1.8 were available, but eframe was not. Actual
`apt-get -s build-dep .` resolution rejected eframe 0.33 and rfd 0.15. The
Debian draft cannot be built from this suite's registry until these crates are
packaged or a compatible, tested dependency adjustment is supplied.

Fedora 45 official repositories provide Rust/Cargo 1.99.1, zbus 5.19 and
rfd 0.15.4; no rust-eframe package was returned. A disposable Fedora 45
container installed rust-packaging/cargo-rpm-macros and verified expansion of
`%cargo_generate_buildrequires -n`. This does not establish a successful RPM
build; the system-registry eframe gap remains.

The checks above ran locally. Hosted CI, native aarch64 builds, visual glyph
quality, portal interaction and GNOME Wayland focus/commit acceptance remain
unverified. No distribution submission, release tag or upload was made.


### Arch offline package validation — 2026-11-17

A cleared source archive was built with the Arch review recipe in an official
Arch base-devel container. Tooling and locked crate sources were prepared first;
release compilation, release tests and packaging ran with networking disabled.
The 41 demo library tests and three CLI tests passed, with one optional external
fixture skipped. All 231 compiled packages had retained notices. Native default
bundled-font installation was also checked separately: its 232-package inventory
included the upstream code, OFL and Ubuntu Font License notices.

The resulting x86_64 demo package installed and loaded `/usr/share/waypinyin`
without a dictionary override. A real private IBus daemon committed `你好` through
the installed binary. A metadata-only packaging revision was used to exercise a
real pacman upgrade from 0.1.0-1 to 0.1.0-2, followed by reinstallation and removal.
Disposable per-user configuration and learning sentinels survived every step.
Only disposable containers were used; the active host installation was untouched.

Namcap reported no errors. Maintainer, libgcc and hicolor dependency declarations
were corrected. Remaining warnings concern the loader and declared runtime dependencies. IBus,
Fontconfig and several window/GL libraries are used via D-Bus, external commands
or dynamic loading. Review the remaining library list alongside real graphical
acceptance before submission.

Local artifacts and raw logs are under `build/arch-audit` and
`build/release-audit` (ignored by Git). The package is a demonstration/review
artifact, not a released production dictionary or official Arch submission.


### User installation and atomic replacement — 2026-11-17

The full local corpus was rebuilt with default bundled fonts and installed to
`~/.local`. The 232 compiled packages had retained notices. Installed metadata,
notices and dictionary validation passed, and the installed new binary passed
the private IBus commit probe. The pre-existing host engine process was left
running; it keeps its previous executable/image until restarted.

Installation now replaces files through same-directory temporary files and
rename, including generated desktop and IBus metadata. A regression test holds
an installed ELF executable running and its dictionary mapped while reinstalling;
the process stays alive, the old mapping is intact, new files have new inodes,
and permissions/temp-file cleanup are correct. This test runs in `make check`
and CI. Previous binary/data inodes were retained locally for rollback; no
user configuration or learning files were overwritten.
