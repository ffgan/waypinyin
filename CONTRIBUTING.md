# Contributing

Waypinyin is a Rust IBus Pinyin engine, primarily tested on GNOME Wayland.
Original contributions are accepted under the MIT license. Third-party data
must retain its own notices and must include verifiable source and redistribution
terms. Do not submit private learning logs or dictionaries without permission.

Before submitting a patch, run `make check` and test the affected input behavior.
Explain the concrete problem, resulting behavior, and validation performed.
For ranking changes, run both quality examples with the same dictionary and
compare results and latency. Add regression cases for behavior changes; avoid
unrelated formatting or algorithm changes.

Data changes must update `data/sources.json`, preserve attribution in `data/NOTICE`,
and document the exact source revision and transformation. Updating a checksum
alone does not resolve a license or provenance issue.

The canonical repository and issue tracker are at https://github.com/ffgan/waypinyin and
https://github.com/ffgan/waypinyin/issues. Submit pull requests against the public branch.
No contributor agreement or copyright assignment is required. Never include
excluded historical dictionaries or old private Git objects in a pull request.
