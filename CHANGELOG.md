# Changelog

## Unreleased

- Replace installed files atomically so a running engine can keep its executable
  and mapped dictionary while an update is staged.

- Validate an Arch demo package offline, including real IBus, installation,
  packaging-revision upgrade, reinstallation and removal with user files preserved.

- Verify byte-identical OpenCC/FrequencyWords provenance and retain pinned notices.
- Collect dependency and bundled-font notices, including upstream supplements.
- Support Rust 1.89 and distribution zbus 5.19 with unchanged IBus signatures.
- Exercise engine discovery, preedit, candidates and commits on private IBus buses.

- Separate build and install for distribution staging, support custom directory
  verification and absolute desktop launch paths, and add an Arch review template.

- Add a history-free public source export, original demo vocabulary and external
  build-time data selection while preserving the complete local corpus.

- Add prefix-aware staged installation and portable IBus metadata.
- Add command help, version and headless dictionary validation.
- Respect XDG user data directories and preserve legacy learning logs on migration.
- Validate dictionary strings, indexes and section bounds before use.
- Make equal-letter dictionary edge ordering deterministic.
- Add explicit data provenance inventory, release gate, packaging drafts and CI.
- License original project code and documentation under MIT; retain separate
  third-party data terms and unresolved release blockers.

## 0.1.0 — development baseline

Offline IBus Pinyin engine with candidate paging, learning, sentence composition,
fuzzy Pinyin, dictionary import and a graphical settings window. No public
release or distribution inclusion is asserted.
