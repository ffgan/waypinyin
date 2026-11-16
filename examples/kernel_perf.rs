//! Linux startup and composition-refresh probe. See docs/PERFORMANCE.md.
use std::sync::Arc;
use std::time::Instant;
use waypinyin::{Lexicon, Session, UserDict};

fn usage() -> libc::rusage {
    let mut usage = std::mem::MaybeUninit::uninit();
    // SAFETY: getrusage initializes the output on success.
    assert_eq!(
        unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) },
        0
    );
    unsafe { usage.assume_init() }
}

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "startup".into());
    assert!(matches!(mode.as_str(), "startup" | "input"));
    let before = usage();
    let start = Instant::now();
    let lex = Arc::new(Lexicon::load());
    let elapsed = start.elapsed();
    let after = usage();
    println!(
        "load_us={} minor_faults={} major_faults={}",
        elapsed.as_micros(),
        after.ru_minflt - before.ru_minflt,
        after.ru_majflt - before.ru_majflt
    );
    if mode == "startup" {
        return;
    }
    let iterations = std::env::args()
        .nth(2)
        .map(|s| s.parse::<usize>().expect("positive iteration count"))
        .unwrap_or(1000);
    assert!(iterations > 0);
    let mut session = Session::with_user(lex, UserDict::default());
    for _ in 0..8 {
        std::hint::black_box(session.handle('n' as u32, 0));
        session.clear();
    }
    let mut timings = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let start = Instant::now();
        std::hint::black_box(session.handle('n' as u32, 0));
        timings.push(start.elapsed().as_nanos());
        session.clear();
    }
    timings.sort_unstable();
    println!(
        "compositions={iterations} p50_ns={} p95_ns={}",
        timings[iterations / 2],
        timings[iterations * 95 / 100]
    );
}
