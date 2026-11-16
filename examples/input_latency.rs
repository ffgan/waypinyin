//! Candidate-equivalence trace and release latency probe; see docs/PERFORMANCE.md.
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;
use waypinyin::{suggest, InputPreferences, Lexicon, Session, UserDict};

fn cases() -> Vec<(&'static str, &'static str)> {
    let mut cases = Vec::new();
    for line in include_str!("../tests/common.tsv").lines() {
        cases.push(("short", line.split_once('\t').unwrap().0));
    }
    for line in include_str!("../tests/sentences.tsv")
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
    {
        for raw in line.split('\t').skip(1) {
            cases.push(("sentence", raw));
        }
    }
    cases
}

fn session(lex: &Arc<Lexicon>) -> Session {
    Session::with_user(Arc::clone(lex), UserDict::default())
}

fn visible(mut view: waypinyin::View) -> waypinyin::View {
    let end = (view.cursor / view.page_size + 1) * view.page_size;
    view.candidates.truncate(end);
    view.spans.truncate(end);
    view
}

fn shown(effect: waypinyin::Effect) -> waypinyin::Effect {
    use waypinyin::Effect;
    match effect {
        Effect::Update(v) => Effect::Update(visible(v)),
        Effect::Forward(v) => Effect::Forward(visible(v)),
        Effect::Commit { text, next } => Effect::Commit {
            text,
            next: visible(next),
        },
        Effect::Ignore => Effect::Ignore,
    }
}

fn trace(lex: &Arc<Lexicon>) {
    for (_, raw) in cases().into_iter().chain([
        ("edge", "ni'hao"),
        ("edge", "lue"),
        ("edge", "zan"),
        ("edge", "sou"),
        ("edge", "shuo"),
        ("edge", "nihap"),
        ("edge", "xuexiwan"),
        ("edge", "henjianlou"),
        ("edge", "zhongguo"),
        ("edge", "x'x'x'x'x'x'x'x"),
    ]) {
        let mut s = session(lex);
        for c in raw.chars() {
            println!("{raw}\tkey {c:?}\t{:?}", shown(s.handle(c as u32, 0)));
        }
        println!("{raw}\tview\t{:?}", visible(s.view()));
        for _ in 0..3 {
            println!("{raw}\tpage\t{:?}", shown(s.page(1)));
        }
        println!("{raw}\tpage back\t{:?}", shown(s.page(-1)));
        for key in [0xff51, 0xff08, 'a' as u32, 0xff53, 0xffff] {
            println!("{raw}\tedit {key}\t{:?}", shown(s.handle(key, 0)));
        }
        println!("{raw}\treset\t{:?}", shown(s.reset()));
        for c in raw.chars() {
            s.handle(c as u32, 0);
        }
        println!("{raw}\treplay\t{:?}", visible(s.view()));
        println!("{raw}\tsuggest\t{:?}", suggest(raw, lex));
    }
    // Dense spellings: traverse every page, including partial/phrase tails.
    for raw in ["zhi", "nihao", "ni'hao", "xuexiwan"] {
        let mut s = session(lex);
        for c in raw.chars() {
            s.handle(c as u32, 0);
        }
        loop {
            let before = visible(s.view());
            s.page(1);
            let after = visible(s.view());
            let old_len = before.candidates.len();
            assert_eq!(&after.candidates[..old_len], before.candidates.as_slice());
            assert_eq!(&after.spans[..old_len], before.spans.as_slice());
            println!(
                "{raw}\tall pages\t{:?}\t{:?}\t{}\t{:?}",
                &after.candidates[old_len..],
                &after.spans[old_len..],
                after.cursor,
                after.reading
            );
            if after == before {
                break;
            }
        }
    }
    let mut s = session(lex);
    for _ in 0..4 {
        for c in "nihao".chars() {
            s.handle(c as u32, 0);
        }
        println!("learn\t{:?}", shown(s.pick(1)));
        for c in "nihao".chars() {
            s.handle(c as u32, 0);
        }
        println!("learned\t{:?}", visible(s.view()));
        println!("forget\t{:?}", shown(s.handle('1' as u32, 1 << 2)));
        s.clear();
    }
    let off = InputPreferences {
        enabled: false,
        ..Default::default()
    };
    for raw in ["zan", "sou", "zhang", "nihao"] {
        let mut s = Session::with_preferences(Arc::clone(lex), UserDict::default(), off);
        for c in raw.chars() {
            println!("off {raw}\t{:?}", shown(s.handle(c as u32, 0)));
        }
    }
}

fn measure<T>(timings: &mut BTreeMap<String, Vec<u128>>, group: &str, f: impl FnOnce() -> T) {
    let start = Instant::now();
    let result = f();
    let elapsed = start.elapsed().as_nanos();
    std::hint::black_box(&result);
    timings.entry(group.into()).or_default().push(elapsed);
}

fn bench(lex: &Arc<Lexicon>) {
    let mut timings = BTreeMap::new();
    let mut slowest = (0u128, "", 0usize);
    for (group, raw) in cases() {
        if group == "short" {
            measure(&mut timings, "suggest", || suggest(raw, lex));
        }
        let mut s = session(lex);
        for (i, c) in raw.chars().enumerate() {
            let start = Instant::now();
            std::hint::black_box(s.handle(c as u32, 0));
            let elapsed = start.elapsed().as_nanos();
            timings
                .entry(format!("{group}_key"))
                .or_insert_with(Vec::new)
                .push(elapsed);
            if elapsed > slowest.0 {
                slowest = (elapsed, raw, i + 1);
            }
        }
        measure(&mut timings, "view", || s.view());
        measure(&mut timings, "page", || s.page(1));
        for key in [0xff51, 0xff08, 'a' as u32, 0xff53, 0xffff] {
            measure(&mut timings, "edit", || s.handle(key, 0));
        }
        // Warm this exact spelling, then measure a full cached replay.
        s.clear();
        for c in raw.chars() {
            s.handle(c as u32, 0);
        }
        s.clear();
        for c in raw.chars() {
            measure(&mut timings, "cached_key", || s.handle(c as u32, 0));
        }
    }
    for (group, mut times) in timings {
        times.sort_unstable();
        println!(
            "{group} count={} p50_ns={} p95_ns={} max_ns={}",
            times.len(),
            times[times.len() / 2],
            times[times.len() * 95 / 100],
            times.last().unwrap()
        );
    }
    println!(
        "slowest ns={} input={} prefix_bytes={}",
        slowest.0, slowest.1, slowest.2
    );
}

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "bench".into());
    assert!(matches!(mode.as_str(), "trace" | "bench"));
    let lex = Arc::new(Lexicon::load());
    if mode == "trace" {
        trace(&lex);
    } else {
        bench(&lex);
    }
}
