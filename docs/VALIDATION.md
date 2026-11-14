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
