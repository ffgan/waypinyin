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
