use std::sync::Arc;
use waypinyin::{Lexicon, Session, UserDict};
fn main() {
    let lex = Arc::new(Lexicon::load());
    let mut first = 0;
    let mut page = 0;
    let mut count = 0;
    let mut times = Vec::new();
    for line in include_str!("../tests/sentences.tsv")
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
    {
        let cols: Vec<_> = line.split('\t').collect();
        for raw in &cols[1..] {
            let mut session = Session::with_user(Arc::clone(&lex), UserDict::default());
            for c in raw.chars() {
                let t = std::time::Instant::now();
                session.handle(c as u32, 0);
                session.view();
                times.push(t.elapsed().as_micros());
            }
            let v = session.view();
            count += 1;
            first += usize::from(v.candidates.first().is_some_and(|s| s == cols[0]));
            page += usize::from(v.candidates.iter().take(7).any(|s| s == cols[0]));
            println!(
                "{raw}\t{}\t{}",
                cols[0],
                v.candidates.first().map_or("", String::as_str)
            );
        }
    }
    times.sort();
    eprintln!(
        "cases={count} first={first} top7={page} key_p95_us={} key_max_us={}",
        times[times.len() * 95 / 100],
        times.last().unwrap()
    );
}
