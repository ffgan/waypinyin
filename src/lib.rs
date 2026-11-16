//! Pinyin input method. [`session`] owns the composition, [`lexicon`] ranks
//! candidates, and [`ibus`] only translates [`session::Effect`] onto the bus.

pub(crate) mod ibus;
#[allow(dead_code)]
pub(crate) mod image;
pub(crate) mod lexicon;
pub(crate) mod session;

pub use lexicon::{suggest, Lexicon};
pub use session::{Effect, Session, UserDict, View};

pub async fn run() -> zbus::Result<()> {
    ibus::serve().await
}

#[cfg(all(test, waypinyin_full_lexicon))]
mod tests;

pub mod imports;
pub mod settings;

pub mod preferences;
pub use preferences::InputPreferences;

pub(crate) mod paths;

#[cfg(test)]
mod smoke_tests {
    use super::*;
    #[test]
    fn basic_pinyin_and_session_commit() {
        let lex = std::sync::Arc::new(Lexicon::load());
        assert_eq!(
            suggest("nihao", &lex).first().map(String::as_str),
            Some("你好")
        );
        assert_eq!(
            suggest("zhongguo", &lex).first().map(String::as_str),
            Some("中国")
        );
        let mut session = Session::with_user(lex, UserDict::default());
        for key in "nihao".chars() {
            session.handle(key as u32, 0);
        }
        assert!(
            matches!(session.handle(' ' as u32, 0), Effect::Commit {text, ..} if text == "你好")
        );
    }
}

pub(crate) mod watch;

pub(crate) mod preload;
