# Security

Waypinyin 0.1.0 is pre-release software. Security fixes currently target the
latest development version; no stable support period has been promised.

For a vulnerability, use GitHub private vulnerability reporting at
https://github.com/ffgan/waypinyin/security/advisories/new.
If reporting is unavailable, contact ffgandev@gmail.com privately. Do not publish learning logs, private input,
or third-party dictionaries in a public report. Include the version, operating
system, reproduction steps and a minimal synthetic input where possible.

Imported dictionaries are parsed locally. The compiled lexicon image is trusted
installation data, not an import format. Its loader checks structure and UTF-8,
but images must not be modified or truncated while any process maps them.
Personal choices are stored locally in the XDG data directory; no telemetry or
cloud candidate service is implemented.


The learning log also stores confirmed input spellings and frequency counters
for preload priority. It does not log individual keys or canceled compositions.
IBus password/PIN purposes and private/hidden hints disable learning and new
speculation. A composition that entered a protected context stays unlearned
until it is cleared or committed, even if the context switches back meanwhile.
These protections depend on the client's content-type metadata. Newly created
learning logs and compaction files use mode 0600.
