# Linux performance

The runtime keeps the lexicon in a read-only file-backed `mmap`, sharing the
kernel page cache across input contexts. `OnceLock` validates and publishes the
image once per process. Validation checks all sections, UTF-8 and indexes before
queries use the mapped records.

Validation traverses the image tables, so the mapping uses `MADV_SEQUENTIAL`
during that pass and switches to `MADV_RANDOM` for candidate lookup, following
[madvise(2)](https://man7.org/linux/man-pages/man2/madvise.2.html). These hints
are best effort: an unsupported hint does not skip validation or prevent input.
There is no eager population or locking of the whole dictionary in RAM.

Imported dictionaries and live input preferences use nonblocking inotify
watches. The directory watch handles atomic file replacement as well as direct
writes and deletion. Missing directories use the nearest existing ancestor, so
first-run input does not repeatedly attempt to open the missing file. The watch
is installed before loading data; events arriving during loading are processed
at the next composition boundary. File descriptors close with their owner and
are created with `CLOEXEC`.

Any queued event invalidates the corresponding cache, including overflow,
unmount and watch removal. Refresh replaces the watch and reloads the data;
there is no unbounded event-draining loop on the input thread. An unchanged
composition boundary takes one nonblocking read per watch. Every two seconds,
at the next composition, metadata is reconciled and the watch renewed to cover
remote writes, symlink targets outside the directory and ancestor replacement.
Unchanged imported records are retained without reparsing on reconciliation.
If a watch cannot be established, dictionary metadata polling and preference
file reads remain available. Metadata reconciliation has the original size and
mtime detection limits for changes made outside the watched directory.

Configuration changes take effect when the next composition starts, clearing
cached spelling plans. Changes during a composition wait for the next one.
Release events and modifier shortcuts return before checking files.

The learning journal already uses an open `O_APPEND` file and flushes buffered
records on commits; it does not fsync on every key. The IBus connection uses the
Tokio/zbus asynchronous event loop. The measured changes focus on dictionary
page faults and redundant filesystem work at composition boundaries.

## Reproduce

Build the same probe before and after the change, keeping the baseline binary
outside Cargo's output directory. Use the same compiler, release profile and
full lexicon image for both. The original demo fixture is useful for functional
tests but too small to represent production startup.

```sh
cargo build --locked --offline --release --example kernel_perf
cp target/release/examples/kernel_perf /tmp/waypinyin-kernel-before
# Apply the change, then rebuild the same example.
cargo build --locked --offline --release --example kernel_perf
python3 scripts/kernel_perf.py \
  --before /tmp/waypinyin-kernel-before \
  --after target/release/examples/kernel_perf \
  --image /absolute/path/to/lexicon.bin \
  --work-dir target --runs 7 --iterations 1000 --dictionaries 100
```

The driver isolates settings and imported dictionaries, copies the supplied
image, alternates execution order and reports raw samples plus medians. Warm
startup explicitly primes the private copy. Cold startup requests
`POSIX_FADV_DONTNEED` only for that copy. Use a disk-backed `--work-dir`:
`/tmp` on tmpfs cannot establish disk-cold behavior. Check
`cold_faults_observed` and the per-sample major-fault counts before interpreting
cold results. This is a page-cache experiment, not a flush of hardware caches.
Global page caches and the source image are untouched.

The input probe repeats the same one-letter composition with eight warmups and
clears each preedit. It measures the entire first-key path, including candidate
work, without writing personal learning records. It is a focused refresh probe,
not representative natural-language input. Use the existing `quality` and
`sentence_quality` examples and regression suite for broader candidate behavior.
For syscall inspection, use a disposable configuration and trace the probe with
`strace -c -P /absolute/config/waypinyin/config.toml ... input 100`; do not use
traced runs for latency comparisons.

## Local measurements, 2026-11-16

Fedora 45 x86_64, Linux 7.3.3, Rust 1.99.1, release builds, NVMe/Btrfs. The
baseline was the worktree at the start of this continuation: imported-dictionary
inotify was already present, preferences were read at every composition start,
and `MADV_RANDOM` was applied before validation. The image was 141,873,016 bytes,
SHA-256 `12f8997fdddf0320b52d00e1e409e20658076c336ec0e006762c22847e2e0dc7`.
The [raw samples](benchmarks/kernel-2026-11-16.json) retain timings and probe
binary hashes. Seven alternating runs per variant used the same image and 100 tiny imported
TSV files, with 1,000 compositions in each input run.

- Disk-cold load median: 506,293 to 152,807 microseconds, about 70% lower.
  All baseline samples were 501–514 ms; all changed samples were 147–159 ms.
  Major-fault medians fell from 1,086 to 35; every cold sample had major faults.
- Warm load median: 37,081 to 35,370 microseconds. This small difference is not
  established as a general performance gain.
- Median of input-run P95 values: 128,592 to 125,957 nanoseconds. This small
  difference is likewise not established as an end-to-end latency gain.
- A separate path-filtered syscall trace, with eight warmups and 100 timed
  compositions, showed configuration opens falling from 109 to one. The file
  was loaded during initialization and then stayed cached.

These are local observations, with other host workloads present, not portable
latency guarantees. Functional checks cover corrupt images, unchanged caches,
same-size rewrites with restored mtime, atomic settings saves, deletion, missing
and replaced ancestors, composition-boundary application and plan invalidation.

Validation completed for the full-data worktree (`cargo test --locked --offline`),
the new session-boundary test, and the original demo fixture with
`--no-default-features`. Strict Clippy, formatting, diff whitespace and Python
syntax checks passed. The optional external native-dictionary fixture test was
skipped because its input files were not supplied.

## Candidate generation and input latency

The candidate engine computes each query's best syllable segmentation once and
shares it across morphology, sentence composition and phrase generation. Source
classification is also a single pass, sharing positive-word checks and direct
reading matches across its flags and final source list. Session
cache entries and candidate pools share an immutable `Arc<Plan>`; cursors,
deduplication state, user overlays and emitted pages remain independent. The
32-entry cache limit and preference-change invalidation rules are unchanged.

Sentence search keeps readings as syllable IDs until output, moves completed
parent beams instead of cloning them, and allocates new paths only after their
replacement checks pass. Node readings are reused, last characters are retained,
and an at-most-4,096-entry query-local context-score cache avoids repeated scoring.
Fixed arrays replace the short transition-length vectors. Search budgets,
traversal order, floating-point operation order and tie-breaking stay unchanged.

Mapped dictionary strings are borrowed in merge heaps and deduplication sets;
output candidates still own their text. Ordinary inputs no longer build an
extra merge heap to compute an unused learning-score floor. One-byte inputs
skip partial-prefix discovery because they cannot contain a shorter nonempty
prefix. Partial-candidate normalized scores are computed once before sorting.
Umlaut-alias mismatches reject incompatible second letters before allocating.

Callgrind inspection also identified allocation/hash work in phrase enumeration.
For up to 25 syllables, five-bit character indices fit in a `u128`, preserving the
old vector's lexicographic tie order without allocating every neighbor index.
Longer phrases retain vector indices. Candidate limits and the number of
characters considered per syllable are unchanged. No public API, image format,
configuration option or runtime dependency was added for these changes.

### Reproduce the input comparison

Build `input_latency` with the original candidate engine and retain the binary,
then rebuild it after the change:

```sh
cargo build --locked --offline --release --example input_latency
cp target/release/examples/input_latency /tmp/waypinyin-input-before
# Apply the candidate-engine changes and rebuild the same probe.
cargo build --locked --offline --release --example input_latency
python3 scripts/input_perf.py \
  --before /tmp/waypinyin-input-before \
  --after target/release/examples/input_latency \
  --image /absolute/path/to/lexicon.bin \
  --runs 7 --output /tmp/waypinyin-input-results.json
```

The driver creates isolated settings and a small imported dictionary, captures
three baseline traces and a changed trace, warms both variants and alternates
seven measurement pairs. Trace mode compares the complete public `suggest()`
results, every corpus input prefix's `Effect`, view fields, editing, replay,
learning, deletion and disabled fuzzy preferences. Four selected spellings are
paged to exhaustion, asserting that previously emitted candidate prefixes do
not change. No learning file is written. Traces are not used as latency samples.

The baseline already has a nondeterministic `suggest("xianqchifan")` order in
one part of its list. The driver records the actual baseline variants and only
accepts an exact variant observed during that run; it does not sort candidates
or discard arbitrary differences. All other trace lines must match exactly.
If another baseline variant appears only in the changed run, comparison fails
and requires investigation rather than silently widening the acceptance rule.

Bench mode reports nanoseconds for full short-word `suggest()` queries, short
and sentence `handle()` calls, subsequent `view()`, edits, first forward page
and replayed keys. The `cached_key` group is a warmed replay with the existing
32-entry policy: long inputs can evict earlier prefixes, so this group is not a
claim that every key hits the cache. Timed `handle()` calls already generate the
first page returned in their effect; an extra `view()` is measured separately.
Formatting and metric collection happen outside each operation's timed region.
These probes time candidate/session computation, not physical-key-to-desktop-frame
latency or D-Bus scheduling. Each group reports P50, P95 and maximum; the slowest key's spelling and prefix
position are also retained. Compare medians across runs, not single maxima.

### Equivalence checks

The test-only `lexicon_reference.rs` freezes the original beam search, phrase
enumerator and umlaut matcher. It verifies exact sentence outputs and readings
for 1,174 distinct sentence-corpus prefixes plus boundary cases; exact phrase
ordering/scores through the 4,096-candidate limit and the packed/vector boundary;
and matching across all built-in syllables and their prefixes with fuzzy mode
on and off. A separate test verifies the deferred learning-score floor. The
reference algorithms do not compile into production or demo binaries.

Session tests verify shared plans with independent pagination, continued pool
validity after cache invalidation, and configuration changes at composition
boundaries. The full release regression suite and demo system-font build remain
part of validation, alongside strict Clippy and formatting checks.

### Input measurements, 2026-11-16

The final seven alternating pairs used the same Fedora/Rust environment and
141,873,016-byte full image as the kernel measurements above. Each run includes
200 short words and 75 sentence variants (1,202 short-word key events and 1,529
sentence key events). The [raw samples](benchmarks/input-2026-11-16.json) retain
image/probe/source hashes, per-run timings, slowest inputs and observed trace
variants. Values below are medians of the seven per-run P95 measurements.

- Short-word keys: 0.347 to 0.273 milliseconds, 21.3% lower.

- Sentence keys: 26.453 to 12.889 milliseconds, 51.3% lower.

- Full short-word candidate enumeration: 0.697 to 0.571 milliseconds, 18.1% lower.

- Warmed replay keys: 13.849 to 7.873 milliseconds, 43.2% lower.

- Editing operations: 5.605 to 4.384 milliseconds, 21.8% lower.

- Forward page: 13.325 to 13.385 microseconds.

- Subsequent view: 0.631 to 0.592 microseconds.


Short-word and sentence-key P95 met the 20% reduction targets in this local
cohort. Forward-page P95 was effectively flat (about 0.5% higher); its P50 was
lower. None of the measured groups had a persistent median regression over 5%.
Host scheduling affects individual maxima; these results establish no hard
latency bound or desktop-frame latency guarantee.

Of 7,565 trace records, 7,564 were stable across baseline runs and matched
exactly. The remaining record was the existing `xianqchifan` `suggest()` order
variation; the changed result exactly matched an observed baseline variant.
The separate original-algorithm tests also passed, retaining search budgets,
scoring order and candidate coverage.

## First-page loading and speculative preload

The visible page has five candidates. Session rendering now materializes that
page alone, instead of generating a second page before publishing the first.
Paging and selection extend the same candidate stream on demand. Phrase
combinations also use a resumable cursor: requesting a few candidates no longer
allocates all 4,096 combinations. The full `suggest()` API still enumerates its
complete result, and the original phrase-order oracle continues to check it.

A single process-wide `waypinyin-warm` worker prepares dictionary-only first
pages. IBus starts it before engine registration and requests warming again on
focus. `Session::with_user()` enables it automatically; library callers using
fixed preferences can opt in with `Session::preload()`. That method returns
readiness without waiting for a page. Empty input requests the 26 initial-letter
pages; other input prepares its own page, next-letter insertions and adjacent
backspace/delete variants. The highlighted reading and up to five first-page
readings prioritize next letters, followed by valid syllable continuations and
all remaining letters. Priorities change speculative work order, not candidates.

The worker uses Linux `SCHED_IDLE`, falling back to nice 19 if the policy cannot
be set. Scheduling is changed only inside the worker thread, as described by
[sched_setscheduler(2)](https://man7.org/linux/man-pages/man2/sched_setscheduler.2.html).
It sleeps on a condition variable when no jobs remain. A new eligible request
replaces pending work; generation checks discard obsolete results after an
in-flight job completes. The foreground only submits a compact request and uses
`try_lock` for cache access. Queue construction, old-job destruction, dictionary
search and cache eviction occur on the worker. Misses, lock contention or thread
creation failure use the normal synchronous candidate path.

The cache holds at most 128 entries within an eight-MiB accounted-owned-payload
budget. Accounting is not a bound on total process RSS, mapped pages, thread
stacks or temporary search memory. Queries longer than 128 bytes are not
speculated. Single-letter pages receive eviction preference, and each active
queue without a personal profile contains at most 29 distinct query variants;
personalized queues are capped at 128. Keys include the full spelling
and matching preferences. Time/date shortcuts are excluded from the shared cache.

On a dictionary-only hit, the first five `Choice` records are shared through an
`Arc<Vec<_>>`; neither their strings nor the remaining merge state are copied
until needed. Returning the view copies only the small displayed result into the
public output. The continuation pool is cloned on the first later-page request,
with copy-on-write output preserving the original shared page. Personal learning
and imported dictionaries are evaluated on the foreground for every query. If
they contribute candidates, the session uses the prepared immutable plan while
merging fresh overlays instead of serving the dictionary-only snapshot.

### First-page probe

Build the same `first_screen` probe before and after the change and retain both
binaries. Use the same full dictionary image:

```sh
cargo build --locked --offline --release --example first_screen
cp target/release/examples/first_screen /tmp/waypinyin-first-screen-before
# Apply the first-page/preload changes and rebuild the same probe.
cargo build --locked --offline --release --example first_screen
python3 scripts/first_screen_perf.py \
  --before /tmp/waypinyin-first-screen-before \
  --after target/release/examples/first_screen \
  --image /absolute/path/to/lexicon.bin \
  --runs 7 --idle-ms 2 --output /tmp/waypinyin-first-screen.json
```

The driver isolates settings and imported dictionaries, alternates run order and
checks the first-candidate quality counts in each pair. Each run types the 200
common words, measuring the initial key, all 1,202 per-key first pages, and cached
replays. Timing covers `handle()` through its returned view; sleep and effect
destruction are outside the timed region. The two conditions are zero deliberate
idle time and two milliseconds between keys (also before each composition).
The idle gap gives the worker time to speculate; it is not included in latency.
A gap does not guarantee that every prediction is ready. Measurements exclude
physical-key dispatch, D-Bus transport and desktop drawing.

The `input_latency trace` adapter now compares the visible accumulated prefix
through the current page. It excludes the old hidden lookahead page from both
versions, keeping exact comparison of displayed text, spans, cursor, reading,
commit effects and edits. Exhaustive selected-word paging still compares the
complete stream. The pre-existing mixed-spelling `suggest()` order variation is
handled only by matching an actually observed baseline variant.


### First-page measurements, 2026-11-16

Seven alternating pairs per condition used the same full lexicon and release
profile as above, with 200 words and 1,202 timed key events per run. Every run
retained 200/200 first candidates. The [raw samples](benchmarks/first-screen-2026-11-16.json)
include both conditions, hashes, visible-trace comparison and supplemental CPU
and resource diagnostics. The statistics below are medians across the seven
per-run measurements.

With two milliseconds of deliberate inter-key idle time (outside timing):

- Initial-key first-page P95: 147.656 to 18.414 microseconds, 87.5% lower.
- All first-page key P50: 96.160 to 5.891 microseconds, 93.9% lower.
- All first-page key P95: 399.275 to 69.450 microseconds, 82.6% lower.
- Replayed first-page key P95: 67.055 to 13.896 microseconds, 79.3% lower.

With no deliberate idle time, all first-page key P95 changed from 277.669 to
253.964 microseconds (8.5% lower), while replay P95 fell from 32.811 to 19.236
microseconds. The initial-key P50 fell from 48.671 to 7.545 microseconds, but
initial-key P95 improved only modestly (79.609 to 73.678 microseconds). This
condition includes cache misses and demonstrates why readiness and idle time
must be reported with preload results.

The visible trace retained 7,564 exact stable records plus the one known
baseline `suggest()` variation. Complete phrase enumeration and selected-word
paging still matched the original order. Full release validation passed 82
library tests and three CLI tests; the demo system-font build passed 31 library
tests and three CLI tests. One optional external-fixture test was skipped in
each build. Strict Clippy, formatting and Python syntax checks also passed.

Some maximum-latency samples had large pauses in both variants in the same
pairs. Supplemental `cpu` probe runs separated wall time from thread CPU time;
for example, a 286.980 ms operation used 28.899 ms of thread CPU. These are not
hard latency bounds. The optional `first_screen 2 cpu` diagnostic mode adds
clock calls and is not used for the comparison figures above. An additional
whole-probe resource run measured approximately 142,504 KiB peak RSS before and
144,848 KiB after. It also recorded more CPU time after preload (0.37 versus
4.81 seconds of combined user/system CPU across the entire run), reflecting
speculative work on the idle worker. Preload invests extra CPU to reduce the
next key's work; this experiment does not establish power or energy savings.


## Frequent-input preload priority

Confirmed candidate commits now append `P<TAB>tick<TAB>input-code<TAB>count`
records to the existing user dictionary journal. Counts fold during replay and
survive snapshot compaction. `R<TAB>input-code` removes an evicted pattern. The
active table is capped at 128 entries, with least useful/old entries removed
first; the journal itself is compacted on load using the existing policy. Empty,
invalid, over-128-byte, clock/date and unconverted raw-input cases are excluded.
Partial commits count only the consumed prefix, without a trailing separator.
Only actual commits update statistics: ordinary key events add no journal writes.

Pattern weight halves every 512 steps of the existing learning clock. At least
two uses are needed for priority; ties prefer recent use and then spelling.
The 16 hottest patterns seed startup/focus preloading. Existing learned words
and legacy exact-input records fill spare seed slots, so old installations can
benefit without collecting new history first. Profiles are refreshed on commits,
not sorted again at each key event.

At idle startup, favorite initial letters come first, followed by the other
initial letters, complete favorites and breadth-wise favorite prefixes. During
composition, matching favorites' next prefixes and full spellings precede generic
next-key guesses. Queue expansion remains on the idle worker, with a hard cap
of 128 jobs. The foreground passes a small shared profile and a weak reference
to its current plan. Favorite-prefix pages receive eviction preference over
generic speculative pages while preserving the entry/payload budgets. Profiles
only affect preparation order, not result ranking; user/import overlays stay live.

`UserDict::frequent_inputs()` exposes the top repeated recorded codes. Hosts can
call `Session::set_learning_enabled(false)` to disable learning and new preload
requests for a private context. The IBus adapter wires its `ContentType` property
to this behavior, using PASSWORD/PIN purpose and PRIVATE/HIDDEN_TEXT hints. The
property shape is verified against the [upstream IBus engine implementation](https://raw.githubusercontent.com/ibus/ibus/master/src/ibusengine.c).
The whole in-flight composition is marked unlearnable once it touches such a
context, preventing a later return to a normal field from recording its draft.
Existing word/context learning is disabled there as well. Client metadata is
required for recognizing these contexts.

Statistics remain in `$XDG_DATA_HOME/waypinyin/user.txt` (by default
`~/.local/share/waypinyin/user.txt`). New files and compaction temporaries use
mode 0600; existing files retain their permissions until replaced. No network
service is used for profiles. These changes have functional preload-readiness
coverage; the earlier latency cohorts above predate this personalization policy
and are not a new measured speedup claim for recorded habits.
