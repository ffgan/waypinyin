# License texts

Original Waypinyin code, documentation, icon and locally authored sentences:
`../LICENSE` (MIT). Other data is not relicensed by that grant.

Canonical third-party license texts were retrieved on 2026-11-14:

- Apache-2.0: https://www.apache.org/licenses/LICENSE-2.0.txt
- GPL-3.0-only: https://www.gnu.org/licenses/gpl-3.0.txt
- LGPL-2.1-or-later: https://www.gnu.org/licenses/lgpl-2.1.txt
- LGPL-3.0-only: https://www.gnu.org/licenses/lgpl-3.0.txt
- CC-BY-3.0: https://raw.githubusercontent.com/spdx/license-list-data/main/text/CC-BY-3.0.txt
- CC-BY-SA-4.0: https://raw.githubusercontent.com/spdx/license-list-data/main/text/CC-BY-SA-4.0.txt
- pinyin-data-MIT: https://raw.githubusercontent.com/mozillazg/pinyin-data/master/LICENSE

These are reference texts, not proof of the terms applying to the current data.
See `data/sources.json` for provenance gaps. Upstream notices must still be
recovered for the exact versions used. Dependency crate licenses and embedded
font notices must also be reviewed for the actual release feature set; run
`scripts/dependency_inventory.py` to inspect Cargo declarations.

`OpenCC-Apache-2.0.txt` is the exact LICENSE from the pinned OpenCC revision
recorded in `data/sources.json`, matched to the unchanged conversion table.
Run `python3 scripts/verify_upstream.py` to recheck the pinned upstream bytes.

`dependency-supplements.json` pins notices omitted from the published crate
archives to their `.cargo_vcs_info.json` source commits. The inventory helper
checks the local notice hashes and copies notices from each selected dependency
into `build/dependency-notices/`; the installer ships those verbatim files.
The hexf-parse supplement retains its upstream CC0 package declaration plus
the CC0 legal text. Upgrading any listed crate requires reviewing its new notices.
