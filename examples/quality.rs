use waypinyin::{suggest, Lexicon};
fn main() {
    let lex = Lexicon::load();
    let mut top = 0;
    let mut page = 0;
    let mut times = Vec::new();
    for line in include_str!("../tests/common.tsv").lines() {
        let (raw, w) = line.split_once('\t').unwrap();
        let start = std::time::Instant::now();
        let list = suggest(raw, &lex);
        times.push(start.elapsed().as_micros());
        top += usize::from(list.first().is_some_and(|s| s == w));
        page += usize::from(list.iter().take(7).any(|s| s == w));
    }
    times.sort();
    println!(
        "cases={} top1={} top7={} p95_us={}",
        times.len(),
        top,
        page,
        times[times.len() * 95 / 100]
    );
}
