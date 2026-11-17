# Public source and local complete data

Canonical repository: https://github.com/ffgan/waypinyin

The public repository contains original source, tests, documentation, license
reference texts, a locally authored sentence sample and a small original MIT
demonstration vocabulary, plus the reviewed OpenCC conversion table and
FrequencyWords counts with their original notices. It does **not** contain the
seven remaining historical dictionary files with unresolved provenance, compiled
images or the old Git history.
The excluded records remain in `data/sources.json` as attribution/audit metadata,
without their contents. The original local checkout and complete corpus remain
intact. The public tree is suitable for source review and demonstration, not a
claim that the current complete dictionary is cleared for distribution.

## Building

`make build` in the public checkout automatically builds the demonstration
lexicon and emits a Cargo warning. `waypinyin --version` and `--check-data` identify
this as a demo build. The fixture has eighteen hand-written vocabulary records
and artificial weights. It is not suitable for daily Chinese input.

For a locally held complete corpus, set `WAYPINYIN_SOURCE_DATA` to an absolute
directory containing `dict.txt`, `extra.txt`, `terms.txt`, `frost.txt`, `cedict.txt`,
`freq.txt`, `ts-characters.txt`, `pinyin.txt` and `gb2312.txt`:

```sh
WAYPINYIN_SOURCE_DATA=/absolute/path/to/local/data make build
WAYPINYIN_SOURCE_DATA=/absolute/path/to/local/data cargo test --locked
```

No network download is performed by the build. Changing the source directory
rebuilds the image. This environment variable selects build inputs;
`WAYPINYIN_LEXICON` selects an existing runtime image. Supplying local data does
not grant permission to distribute the resulting image or binary.

Compiler inputs use UTF-8. Vocabulary rows are `word<TAB>syllables<TAB>weight`
with apostrophe-separated lowercase syllables; frequency rows are `word count`.
Character coverage is read from `gb2312.txt`. The other source files may be
comment-only, as in the demo fixture. The compiler retains historical weighting
rules; see `src/lexicon_build.rs` before preparing a different corpus.

## Tests and packaging

Common parser, configuration, image validation, protocol and basic Pinyin/session
smoke tests run with the demo. The larger `src/tests.rs` corpus regression module
runs only with complete external/local inputs. The 200-word and 75-sentence
quality examples are meaningful only with that corpus, not with the demo.
CI verifies the demo build and explicitly exercises it; a source-only public
CI run cannot establish complete-corpus input quality.

Debian/Fedora files remain review drafts. The Arch template has passed local
offline demo-package and lifecycle checks, as recorded in VALIDATION.md. A demo-source archive can pass the
data gate, but this is not clearance for external data, bundled dependency
assets, binary releases or official distribution inclusion.

## Publishing without old objects

From the original local tree, export into an empty separate directory:

```sh
python3 scripts/export_public.py --output /absolute/path/to/new-public-checkout
```

The export is allowlisted, excludes unresolved dictionary bodies, validates
hashes and copies no `.git`, build output, archive or personal log. Initialize
and push Git only from that exported directory. Do not add the public remote to
the historical local repository or push its branches/tags. Future updates must
repeat the export and review the changed source against the public checkout;
never merge historical private commits into the public branch.
