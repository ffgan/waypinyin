//! Deterministic long/invalid-input scaling, without speculative preloading.
use std::sync::Arc;
use std::time::Instant;
use waypinyin::{InputPreferences, Lexicon, Session, UserDict};

fn thread_cpu_ns() -> u128 {
    let mut value = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: clock_gettime initializes the writable timespec on success.
    assert_eq!(
        unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut value) },
        0
    );
    value.tv_sec as u128 * 1_000_000_000 + value.tv_nsec as u128
}

fn main() {
    let lex = Arc::new(Lexicon::load());
    if std::env::args().nth(1).as_deref() == Some("diagnose") {
        let pattern = std::env::args()
            .nth(2)
            .unwrap_or_else(|| "jintianwomenyiqiqugongyuan".into());
        assert!(!pattern.is_empty());
        let mut session =
            Session::with_preferences(lex, UserDict::default(), InputPreferences::default());
        let mut records = Vec::with_capacity(256);
        for key in pattern.chars().cycle().take(256) {
            let cpu = thread_cpu_ns();
            let start = Instant::now();
            std::hint::black_box(session.handle(key as u32, 0));
            let wall = start.elapsed().as_nanos();
            records.push((wall, thread_cpu_ns() - cpu));
        }
        // Printing occurs after the entire composition, outside all key timers.
        for (index, (wall, cpu)) in records.iter().enumerate() {
            println!("key_{} wall_ns={wall} cpu_ns={cpu}", index + 1);
        }
        return;
    }
    for (name, pattern) in [
        ("ambiguous", "sh"),
        ("abbreviation", "mtxwsdwmzgsmkjmtlyxgzjh"),
        ("full_pinyin", "jintianwomenyiqiqugongyuan"),
        ("invalid", "vvvvvvvv"),
    ] {
        for length in [8, 16, 32, 64, 128, 256] {
            let raw: String = pattern.chars().cycle().take(length).collect();
            let mut session = Session::with_preferences(
                lex.clone(),
                UserDict::default(),
                InputPreferences::default(),
            );
            let mut times = Vec::new();
            let start = Instant::now();
            for key in raw.chars() {
                let t = Instant::now();
                std::hint::black_box(session.handle(key as u32, 0));
                times.push(t.elapsed().as_nanos());
            }
            let total = start.elapsed().as_nanos();
            let candidates = session.view().candidates.len();
            times.sort_unstable();
            println!(
                "{name}_{length} count={} total_ns={total} p50_ns={} p95_ns={} max_ns={} candidates={candidates}",
                times.len(), times[times.len() / 2], times[times.len() * 95 / 100], times.last().unwrap()
            );
        }
    }
}
