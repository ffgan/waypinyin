//! Profile one uncached long-input key without a speculative worker.
use std::sync::Arc;
use waypinyin::{Effect, Lexicon, Session, UserDict};

#[no_mangle]
#[inline(never)]
pub fn waypinyin_profile_key(session: &mut Session, key: u32) -> Effect {
    session.handle(key, 0)
}

fn main() {
    let raw = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "mtxwsdwmzgsmkjmtlyxgzjh".into());
    assert!(!raw.is_empty() && raw.bytes().all(|ch| ch.is_ascii_lowercase()));
    let lex = Arc::new(Lexicon::load());
    let mut session = Session::with_preferences(lex, UserDict::default(), Default::default());
    for key in raw[..raw.len() - 1].bytes() {
        session.handle(u32::from(key), 0);
    }
    std::hint::black_box(waypinyin_profile_key(
        &mut session,
        u32::from(*raw.as_bytes().last().unwrap()),
    ));
    println!("{:?}", session.view().candidates.first());
}
