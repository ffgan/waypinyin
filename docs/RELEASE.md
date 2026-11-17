# Release readiness

## Current blockers

The public GitHub source edition omits unresolved historical data and all old
Git history; see [PUBLIC_SOURCE.md](PUBLIC_SOURCE.md). Publishing that source
does not publish a binary release or submit a distribution package.
The blockers below apply to the complete historical corpus and full releases.

- Seven current data files have unresolved provenance; see `data/sources.json`.
  Recover exact snapshots, upstream copyright notices, extraction scripts and
  mixed-data redistribution conditions. In particular, `gb2312.txt` has no established source/license; do not infer permission from access.
  `extra.txt` merges sources and cannot be cleared simply by labeling the
  whole project MIT. rime-frost's exact GPL variant also needs confirmation.
- The exact pinyin-data snapshot has been recovered. Its merged inputs include
  cc_cedict, zdic and overrides in addition to Unihan 18.0; the repository MIT
  declaration alone does not establish all underlying rights.
- Recover the historical transformation pipeline. Checksums pin local inputs;
  they do not identify upstream revisions or make the source pipeline complete.
- The canonical public source is https://github.com/ffgan/waypinyin; maintainer
  contact is ffgandev@gmail.com. The historical local repository intentionally
  has no public remote to prevent accidental publication of excluded history.
- Dependency notice files, including bundled-font notices, are now collected
  and installed for the selected target/features. Supplements pin omitted crate
  notices to the crate source commits. Review the actual linked-license expression
  for each binary package; the inventory deliberately includes build dependencies.
- Resolve system Rust/crate availability in the chosen Debian and Fedora
  suites, replace draft packaging identities/license expressions and verify
  native isolated builds. Missing dependencies must be packaged or compatibility
  work tested before submission.
- Complete GNOME Wayland manual acceptance and package lifecycle testing.
  Headless validation cannot establish desktop behavior or visual font quality.

Preserve existing input behavior while researching these issues. If evidence
cannot be obtained, report the blocker; do not silently remove/relicense data.

## Verified evidence recovered on 2026-11-16

The unchanged OpenCC table and FrequencyWords 2018 Chinese 50k counts match
fixed upstream commits byte for byte. Their Apache-2.0 and CC-BY-SA-4.0 notices
are retained in LICENSES. `data/sources.json` records the exact URLs, SHA-256
values and remaining evidence required per file. `scripts/verify_upstream.py`
re-fetches the pinned evidence explicitly; normal builds do not use the network.
The origin of the 19 terms remains unknown according to the maintainer's reply;
that record stays unresolved. No dictionary bytes were replaced or removed.

## Release procedure after blockers are resolved

1. Review the final diff, including original uncommitted feature work. Set the
   intended version consistently in Cargo, IBus XML, manual and package drafts.
   Update CHANGELOG and verify source/asset authorship.
2. Run `make check`, the two release quality examples, dependency notice review,
   `scripts/check_reproducible.py` and staged installation checks. Record the
   compiler, architecture, input hashes and quality/latency results. Require
   unchanged fixed-corpus candidate results for installation-only changes.
3. Build Debian and RPM packages using system registries in network-disabled
   build sandboxes. Review lintian/rpmlint findings, install/upgrade/remove
   behavior and user-file preservation. Native aarch64 validation remains
   required before offering that architecture.
4. In GNOME Wayland, check IBus discovery, full/initial/mixed Pinyin, candidate
   paging and commits, focus changes, settings, Chinese font rendering,
   dictionary import and learning persistence after restart.
5. Run `python3 scripts/check_sources.py --release`. Each unresolved item must
   have been closed with actual evidence, not just a status edit.
6. Set `SOURCE_DATE_EPOCH` to the reviewed release commit timestamp and run
   `make dist`. Inspect and rebuild the extracted archive with dependencies
   supplied ahead of time. Verify no private logs or unlicensed test fixtures
   are present. Archive generation produces a SHA-256 file.
7. Only after review and a separate publishing instruction, tag/publish the
   release and begin the distribution's review process with a maintainer.

`publish = false` prevents accidental Cargo registry publication while the data
audit is unresolved.

`python3 scripts/dist.py --allow-unresolved` makes a `-review.tar.gz` for local
inspection. It is not cleared for redistribution. Normal `make dist` fails while
provenance blockers exist. The source archive is reproducible for identical
inputs and `SOURCE_DATE_EPOCH`; the dictionary check is separate from full ELF
reproducibility, which still requires distribution build comparisons.
