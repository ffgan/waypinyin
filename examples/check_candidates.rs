use std::sync::Arc;
use waypinyin::{Lexicon, Session, UserDict};
fn main() {
    let lex = Arc::new(Lexicon::load());
    for raw in std::env::args().skip(1) {
        let mut session = Session::with_user(Arc::clone(&lex), UserDict::default());
        let start = std::time::Instant::now();
        for c in raw.chars() {
            session.handle(c as u32, 0);
        }
        let view = session.view();
        println!(
            "{}\t{}\t{}",
            raw,
            view.candidates
                .iter()
                .take(7)
                .cloned()
                .collect::<Vec<_>>()
                .join(" | "),
            start.elapsed().as_micros()
        );
    }
}
