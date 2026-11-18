//! Foreground cost with empty, short and full Unicode surrounding contexts.
use std::sync::Arc;
use std::time::Instant;
use waypinyin::{InputPreferences, Lexicon, Session, UserDict};

fn main() {
    let lex = Arc::new(Lexicon::load());
    for length in [0, 16, 64] {
        let context: String = "我们正在处理工作任务请你有空的时候帮助确认文件内容然后讨论工作计划"
            .chars()
            .cycle()
            .take(length)
            .collect();
        let mut timings = [Vec::new(), Vec::new()];
        for (group, raw) in include_str!("../tests/common.tsv")
            .lines()
            .map(|l| (0, l.split_once('\t').unwrap().0))
            .chain(
                include_str!("../tests/sentences.tsv")
                    .lines()
                    .filter(|l| !l.starts_with('#') && !l.is_empty())
                    .flat_map(|l| l.split('\t').skip(1).map(|raw| (1, raw))),
            )
        {
            let mut session = Session::with_preferences(
                lex.clone(),
                UserDict::default(),
                InputPreferences::default(),
            );
            session.set_surrounding_text(&context, length as u32, length as u32);
            for ch in raw.chars() {
                let start = Instant::now();
                session.handle(ch as u32, 0);
                session.view();
                timings[group].push(start.elapsed().as_nanos());
            }
        }
        for (name, mut times) in ["short", "sentence"].into_iter().zip(timings) {
            times.sort_unstable();
            println!(
                "context_{length}_{name} count={} p50_ns={} p95_ns={} max_ns={}",
                times.len(),
                times[times.len() / 2],
                times[times.len() * 95 / 100],
                times.last().unwrap()
            );
        }
    }
}
