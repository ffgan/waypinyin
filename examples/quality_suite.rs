//! Expanded synthetic holdout runner. Does not load personal learning records.
use std::sync::Arc;
use std::time::Instant;
use waypinyin::{InputPreferences, Lexicon, Session, UserDict};

fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let path = args.first().expect("quality_suite FIXTURE [dev|test]");
    let source = std::fs::read_to_string(path).unwrap();
    let lex = Arc::new(Lexicon::load());
    for line in source
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        let fields: Vec<_> = line.split('\t').collect();
        let (expected, codes) = if fields.len() == 6 {
            if args.get(1).is_some_and(|split| split != fields[0]) {
                continue;
            }
            (fields[2], &fields[3..])
        } else {
            (fields[1], &fields[..1])
        };
        for raw in codes {
            let mut session = Session::with_preferences(
                lex.clone(),
                UserDict::default(),
                InputPreferences::default(),
            );
            let mut times = Vec::new();
            for key in raw.chars() {
                let start = Instant::now();
                session.handle(key as u32, 0);
                session.view();
                times.push(start.elapsed().as_nanos());
            }
            let v = session.view();
            times.sort_unstable();
            println!(
                "{raw}\t{expected}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                usize::from(
                    v.candidates
                        .first()
                        .is_some_and(|s| expected.split('|').any(|e| e == s))
                ),
                usize::from(
                    v.candidates
                        .iter()
                        .take(5)
                        .any(|s| expected.split('|').any(|e| e == s))
                ),
                times.len(),
                times[times.len() * 95 / 100],
                times.last().unwrap(),
                v.candidates
                    .iter()
                    .take(5)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(" | "),
                times
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(",")
            );
        }
    }
}
