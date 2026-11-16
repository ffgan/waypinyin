//! Visible first-page latency, with and without idle time for speculative work.
use std::sync::Arc;
use std::time::{Duration, Instant};
use waypinyin::{Lexicon, Session, UserDict};

fn report(name: &str, mut times: Vec<u128>) {
    times.sort_unstable();
    println!(
        "{name} count={} p50_ns={} p95_ns={} max_ns={}",
        times.len(),
        times[times.len() / 2],
        times[times.len() * 95 / 100],
        times.last().unwrap()
    );
}

fn thread_cpu_ns() -> u128 {
    let mut value = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    assert_eq!(
        unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut value) },
        0
    );
    value.tv_sec as u128 * 1_000_000_000 + value.tv_nsec as u128
}

fn main() {
    let idle_ms = std::env::args()
        .nth(1)
        .map(|s| s.parse().unwrap())
        .unwrap_or(0);
    let diagnostic = std::env::args().nth(2).as_deref() == Some("cpu");
    let lex = Arc::new(Lexicon::load());
    let mut first = Vec::new();
    let mut keys = Vec::new();
    let mut replay = Vec::new();
    let mut expected = 0;
    let mut correct = 0;
    let mut slowest = (0u128, "", 0usize, 0u128);
    for line in include_str!("../tests/common.tsv").lines() {
        let (raw, word) = line.split_once('\t').unwrap();
        let mut session = Session::with_user(lex.clone(), UserDict::default());
        if idle_ms > 0 {
            std::thread::sleep(Duration::from_millis(idle_ms));
        }
        for (index, key) in raw.chars().enumerate() {
            let cpu = diagnostic.then(thread_cpu_ns);
            let start = Instant::now();
            let effect = session.handle(key as u32, 0);
            let elapsed = start.elapsed().as_nanos();
            let cpu_elapsed = cpu.map_or(0, |before| thread_cpu_ns() - before);
            std::hint::black_box(effect);
            keys.push(elapsed);
            if elapsed > slowest.0 {
                slowest = (elapsed, raw, index + 1, cpu_elapsed);
            }
            if index == 0 {
                first.push(elapsed);
            }
            if idle_ms > 0 {
                std::thread::sleep(Duration::from_millis(idle_ms));
            }
        }
        let view = session.view();
        correct += usize::from(view.candidates.first().is_some_and(|s| s == word));
        expected += 1;
        session.clear();
        for key in raw.chars() {
            let start = Instant::now();
            let effect = session.handle(key as u32, 0);
            replay.push(start.elapsed().as_nanos());
            std::hint::black_box(effect);
            if idle_ms > 0 {
                std::thread::sleep(Duration::from_millis(idle_ms));
            }
        }
    }
    eprintln!(
        "slowest ns={} input={} prefix={} cpu_ns={}",
        slowest.0, slowest.1, slowest.2, slowest.3
    );
    report("first_key", first);
    report("first_page_key", keys);
    report("replay_page_key", replay);
    println!("quality cases={expected} top1={correct} idle_ms={idle_ms}");
}
