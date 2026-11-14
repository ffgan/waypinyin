use crate::lexicon::{choices, suggest, Lexicon};
use crate::session::{Effect, Session, UserDict, PAGE_SIZE};
use std::sync::Arc;

#[test]
fn lexicon_load_is_a_mapped_image() {
    let warm = Lexicon::load();
    assert_eq!(suggest("de", &warm).first().map(String::as_str), Some("的"));
    let started = std::time::Instant::now();
    for _ in 0..100 {
        std::hint::black_box(Lexicon::load());
    }
    let elapsed = started.elapsed();
    assert!(
        elapsed.as_millis() < 1000,
        "loading the lexicon 100 times took {elapsed:?}"
    );
}

fn committed(effect: Effect) -> String {
    match effect {
        Effect::Commit { text, .. } => text,
        other => panic!("expected a commit, got {other:?}"),
    }
}

#[test]
fn ranking_follows_lexicon_hits() {
    let lex = Lexicon::load();
    let top = |input: &str| {
        let list = suggest(input, &lex);
        list.into_iter().take(5).collect::<Vec<_>>()
    };

    let nihao = top("nihao");
    assert_eq!(nihao.first().map(String::as_str), Some("你好"));
    assert!(nihao.iter().all(|word| word != "你好吗"));

    let niha = top("niha");
    assert_eq!(niha.first().map(String::as_str), Some("你好"));

    let ni = top("ni");
    assert_eq!(ni.first().map(String::as_str), Some("你"));
    assert!(ni.iter().any(|word| word == "你们"));
    assert!(ni.iter().all(|word| !word.contains('妳') && word != "年"));

    let zhongguo = top("zhongguo");
    assert_eq!(zhongguo.first().map(String::as_str), Some("中国"));
    assert!(
        zhongguo.iter().all(|word| word != "中国政府"),
        "{zhongguo:?}"
    );
    assert_eq!(
        top("zhinengshouji").first().map(String::as_str),
        Some("智能手机")
    );

    let women = top("women");
    assert_eq!(women.first().map(String::as_str), Some("我们"));
    assert!(women.iter().all(|word| word != "我们约会吧"));

    let xian = top("xian");
    assert_eq!(xian.first().map(String::as_str), Some("先"));
    assert!(xian.iter().any(|word| word == "西安"));
    assert!(xian.iter().all(|word| word != "见"));

    let xian_split = top("xi'an");
    assert_eq!(xian_split.first().map(String::as_str), Some("西安"));
    assert!(xian_split.iter().all(|word| word != "现在" && word != "先"));

    let beijing = top("beijing");
    assert!(beijing.iter().any(|word| word == "背景"));
    assert!(beijing.iter().any(|word| word == "北京"));

    let n = top("n");
    assert!(n.iter().any(|word| word == "你"));
    assert!(n.iter().all(|word| word != "㕶"));

    assert_eq!(top("sh").first().map(String::as_str), Some("是"));
    assert_eq!(top("de").first().map(String::as_str), Some("的"));
    assert_eq!(
        suggest("woaini", &lex).first().map(String::as_str),
        Some("我爱你")
    );
    for input in [
        "wsm",
        "wshm",
        "weism",
        "weishm",
        "wshenm",
        "wsme",
        "weishenme",
        "ws",
    ] {
        assert_eq!(
            suggest(input, &lex).first().map(String::as_str),
            Some("为什么"),
            "{input}"
        );
    }
    assert_eq!(top("nhao").first().map(String::as_str), Some("你好"));
    assert_eq!(top("zguo").first().map(String::as_str), Some("中国"));
    assert_eq!(
        suggest("zguoren", &lex).first().map(String::as_str),
        Some("中国人")
    );
    let zongguo = top("zongguo");
    assert!(zongguo.iter().any(|word| word == "中国"), "{zongguo:?}");
    assert_eq!(top("zh").first().map(String::as_str), Some("这"));
    let today = top("rq");
    assert!(
        today
            .first()
            .is_some_and(|text| text.contains('年') && text.contains('月')),
        "rq: {today:?}"
    );
}

#[test]
fn idle_period_commits_chinese_punctuation() {
    let lex = Arc::new(Lexicon::load());
    let mut session = Session::new(lex);
    assert_eq!(committed(session.handle(u32::from(b'.'), 0)), "。");
}

#[test]
fn resultative_complement_stays_on_the_first_page() {
    let lex = Lexicon::load();
    let list = suggest("pawan", &lex);
    let at = list
        .iter()
        .position(|word| word == "爬完")
        .unwrap_or_else(|| panic!("爬完 missing from {list:?}"));
    assert!(at < PAGE_SIZE, "爬完 at {at}: {list:?}");
    assert_eq!(
        suggest("chiwan", &lex).first().map(String::as_str),
        Some("吃完")
    );
    assert_eq!(
        suggest("nihao", &lex).first().map(String::as_str),
        Some("你好")
    );
}

#[test]
fn degree_hao_keeps_the_adjective_on_the_first_page() {
    let lex = Lexicon::load();
    let list = suggest("haojianlou", &lex);
    let at = list
        .iter()
        .position(|word| word == "好简陋")
        .unwrap_or_else(|| panic!("好简陋 missing from {list:?}"));
    assert!(at < PAGE_SIZE, "好简陋 at {at}: {list:?}");
    assert_eq!(
        suggest("haokan", &lex).first().map(String::as_str),
        Some("好看")
    );
    assert_eq!(
        suggest("haopiaoliang", &lex).first().map(String::as_str),
        Some("好漂亮")
    );
    on_first_page(&lex, "henjianlou", "很简陋");
    on_first_page(&lex, "feichangjianlou", "非常简陋");
    on_first_page(&lex, "haochou", "好丑");
    on_first_page(&lex, "pashangqu", "爬上去");
    on_first_page(&lex, "pajinqu", "爬进去");
    on_first_page(&lex, "padeshang", "爬得上");
    on_first_page(&lex, "pabushang", "爬不上");
    on_first_page(&lex, "pabushangqu", "爬不上去");
    on_first_page(&lex, "jianloule", "简陋了");
    on_first_page(&lex, "jianlouzhe", "简陋着");
    on_first_page(&lex, "jianlouguo", "简陋过");
    on_first_page(&lex, "jianloude", "简陋的");
    on_first_page(&lex, "manmandi", "慢慢地");
    on_first_page(&lex, "payixia", "爬一下");
    on_first_page(&lex, "papa", "爬爬");
    on_first_page(&lex, "payipa", "爬一爬");
    on_first_page(&lex, "meipa", "没爬");
    on_first_page(&lex, "jianlouma", "简陋吗");
    assert_eq!(
        suggest("bushi", &lex).first().map(String::as_str),
        Some("不是")
    );
    assert_eq!(
        suggest("kankan", &lex).first().map(String::as_str),
        Some("看看")
    );
    assert_eq!(
        suggest("xiangyixiang", &lex).first().map(String::as_str),
        Some("想一想")
    );
}

fn on_first_page(lex: &Lexicon, spelling: &str, word: &str) {
    let list = suggest(spelling, lex);
    let at = list
        .iter()
        .position(|item| item == word)
        .unwrap_or_else(|| panic!("{word} missing from {list:?}"));
    assert!(at < PAGE_SIZE, "{word} at {at}: {list:?}");
}

#[test]
fn yibu_keeps_the_common_word() {
    let lex = Lexicon::load();
    let list = suggest("yibu", &lex);
    assert_eq!(list.first().map(String::as_str), Some("一步"));
    assert!(list.iter().any(|w| w == "一"));
    assert!(list.iter().any(|word| word == "异步"), "{list:?}");
}

#[test]
fn equal_scores_follow_subtitle_counts() {
    let lex = Lexicon::load();
    for (input, word) in [
        ("biaoshi", "表示"),
        ("anjian", "案件"),
        ("baoli", "暴力"),
        ("yishi", "意识"),
        ("yiju", "一句"),
    ] {
        assert_eq!(
            suggest(input, &lex).first().map(String::as_str),
            Some(word),
            "{input}"
        );
    }
    let list = suggest("biaoshi", &lex);
    let shown = list.iter().position(|word| word == "表示").expect("表示");
    let marker = list.iter().position(|word| word == "标示").expect("标示");
    assert!(shown < marker, "{list:?}");
}

#[test]
fn nihao_puts_the_word_first() {
    let lex = Lexicon::load();
    let list = suggest("nihao", &lex);
    assert_eq!(list.first().map(String::as_str), Some("你好"));
}

#[test]
fn partial_niha_predicts_nihao() {
    let lex = Lexicon::load();
    let list = suggest("niha", &lex);
    assert_eq!(list.first().map(String::as_str), Some("你好"));
}

#[test]
fn simplified_only() {
    let lex = Lexicon::load();
    for (input, traditional, simplified) in [
        ("guo", "國", "国"),
        ("men", "們", "们"),
        ("zhe", "這", "这"),
        ("lai", "來", "来"),
    ] {
        let list = suggest(input, &lex);
        assert!(
            list.iter().all(|word| !word.contains(traditional)),
            "{input} offered {traditional}: {list:?}"
        );
        assert!(
            list.iter().any(|word| word.contains(simplified)),
            "{input} missing {simplified}: {list:?}"
        );
    }
}

#[test]
fn single_syllable_starts_with_its_character() {
    let lex = Lexicon::load();
    let list = suggest("ni", &lex);
    assert_eq!(list.first().map(String::as_str), Some("你"));
}

#[test]
fn space_commits_the_first_candidate() {
    let lex = Arc::new(Lexicon::load());
    let mut session = Session::new(lex);
    assert!(matches!(
        session.handle(u32::from(b'n'), 0),
        Effect::Update(_)
    ));
    assert!(matches!(
        session.handle(u32::from(b'i'), 0),
        Effect::Update(_)
    ));
    assert!(matches!(
        session.handle(u32::from(b'h'), 0),
        Effect::Update(_)
    ));
    assert!(matches!(
        session.handle(u32::from(b'a'), 0),
        Effect::Update(_)
    ));
    assert!(matches!(
        session.handle(u32::from(b'o'), 0),
        Effect::Update(_)
    ));
    assert_eq!(committed(session.handle(0x20, 0)), "你好");
}

#[test]
fn exact_matches_precede_prefixes() {
    let lex = Lexicon::load();
    let nihao = choices("nihao", &lex);
    assert_eq!(nihao[0].word, "你好");
    assert_eq!(nihao[0].span, 5);
    let prefix = nihao.iter().position(|c| c.word == "你").unwrap();
    assert!(nihao[..prefix].iter().all(|c| c.tier == 0 && c.span == 5));
    assert_eq!(nihao[prefix].span, 2);

    let quoted = choices("ni'hao", &lex);
    let prefix = quoted.iter().find(|c| c.word == "你").unwrap();
    assert_eq!(&"ni'hao"[prefix.span..], "hao");

    let people = choices("zhongguoren", &lex);
    assert_eq!(people[0].word, "中国人");
    assert!(people
        .iter()
        .any(|choice| choice.word == "中国" && &"zhongguoren"[choice.span..] == "ren"));
    assert!(people
        .iter()
        .any(|choice| choice.word == "中" && &"zhongguoren"[choice.span..] == "guoren"));

    let brief = choices("zguoren", &lex);
    assert_eq!(brief[0].word, "中国人");
    assert!(brief
        .iter()
        .any(|choice| choice.word == "中国" && &"zguoren"[choice.span..] == "ren"));
}

#[test]
fn picking_a_prefix_keeps_the_remaining_pinyin() {
    let lex = Arc::new(Lexicon::load());
    let mut session = Session::new(lex);
    for ch in ['n', 'i', 'h', 'a', 'o'] {
        assert!(matches!(
            session.handle(u32::from(ch as u8), 0),
            Effect::Update(_)
        ));
    }
    let view = session.view();
    let index = view
        .candidates
        .iter()
        .position(|word| word == "你")
        .expect("你");
    assert!(
        index < PAGE_SIZE,
        "你 should be on the first page: {:?}",
        view.candidates
    );
    assert_eq!(committed(session.handle(0x31 + index as u32, 0)), "你");
    let view = session.view();
    assert_eq!(view.preedit, "hao");
    assert_eq!(view.candidates.first().map(String::as_str), Some("好"));
    assert_eq!(committed(session.handle(0x20, 0)), "好");
    assert!(session.view().preedit.is_empty());
}

fn type_in(session: &mut Session, keys: &str) {
    for ch in keys.chars() {
        assert!(
            matches!(session.handle(u32::from(ch as u8), 0), Effect::Update(_)),
            "{keys}"
        );
    }
}

fn pick_word(session: &mut Session, word: &str) -> String {
    let view = session.view();
    let index = view
        .candidates
        .iter()
        .position(|item| item == word)
        .unwrap_or_else(|| panic!("{word} missing from {:?}", view.candidates));
    committed(session.pick(index))
}

#[test]
fn dynamic_frequency_reorders_then_fades() {
    let lex = Arc::new(Lexicon::load());
    let mut session = Session::new(lex);
    type_in(&mut session, "beijing");
    let view = session.view();
    let original = view.candidates[0].clone();
    let other = view
        .candidates
        .iter()
        .zip(&view.spans)
        .find(|(word, span)| *word != &original && **span == view.preedit.len())
        .map(|(word, _)| word.clone())
        .expect("another full-length candidate");
    assert_eq!(pick_word(&mut session, &other), other);
    type_in(&mut session, "beijing");
    assert_eq!(session.view().candidates[0], other);
    session.elapse(32);
    assert_eq!(session.view().candidates[0], original);
}

#[test]
fn user_words_match_across_fuzzy_initials() {
    let lex = Arc::new(Lexicon::load());
    let user = UserDict::from_log("U\t1\tla\t拿\t8\t0\n");
    let mut prefs = crate::InputPreferences::default();
    prefs.groups[6] = true;
    let mut session = Session::with_preferences(lex, user, prefs);
    type_in(&mut session, "na");
    assert_eq!(session.view().candidates[0], "拿");
}

#[test]
fn legacy_spelling_boosts_only_the_exact_input() {
    let lex = Arc::new(Lexicon::load());
    let user = UserDict::from_log("nihao\t测频\t2\n");
    let mut session = Session::with_user(lex, user);
    type_in(&mut session, "nihao");
    assert_eq!(session.view().candidates[0], "测频");
    session.clear();
    type_in(&mut session, "nh");
    assert!(session.view().candidates.iter().all(|word| word != "测频"));
}

#[test]
fn two_picks_coin_a_word_and_ctrl_digit_deletes_it() {
    let lex = Arc::new(Lexicon::load());
    let mut session = Session::new(lex);
    type_in(&mut session, "nihao");
    assert_eq!(pick_word(&mut session, "你"), "你");
    assert_eq!(session.view().preedit, "hao");
    assert_eq!(pick_word(&mut session, "号"), "号");
    type_in(&mut session, "nihao");
    assert_eq!(session.view().candidates[0], "你号");
    session.clear();
    type_in(&mut session, "nh");
    assert!(
        session.view().candidates.iter().any(|word| word == "你号"),
        "{:?}",
        session.view().candidates
    );
    session.clear();
    type_in(&mut session, "nihao");
    assert!(matches!(session.handle(0x31, 1 << 2), Effect::Update(_)));
    assert!(session.view().candidates.iter().all(|word| word != "你号"));
}

#[test]
fn tombstone_drops_a_logged_word() {
    let lex = Arc::new(Lexicon::load());
    let user = UserDict::from_log("U\t4\tni'hao\t你号\t3\t1\nD\tni'hao\t你号\n");
    let mut session = Session::with_user(lex, user);
    type_in(&mut session, "nihao");
    assert!(session.view().candidates.iter().all(|word| word != "你号"));
}

#[test]
fn focus_loss_does_not_learn() {
    let lex = Arc::new(Lexicon::load());
    let mut session = Session::new(lex);
    type_in(&mut session, "beijing");
    let original = session.view().candidates[0].clone();
    let view = session.view();
    let index = view
        .candidates
        .iter()
        .zip(&view.spans)
        .position(|(word, span)| word != &original && *span == view.preedit.len())
        .expect("full-length alternate");
    for _ in 0..index {
        assert!(matches!(session.handle(0xff54, 0), Effect::Update(_)));
    }
    assert_eq!(committed(session.commit_preedit()), view.candidates[index]);
    type_in(&mut session, "beijing");
    assert_eq!(session.view().candidates[0], original);
}

#[test]
fn commit_closes_the_lookup_without_an_association_list() {
    let lex = Arc::new(Lexicon::load());
    let mut session = Session::new(Arc::clone(&lex));
    type_in(&mut session, "beijing");
    assert_eq!(pick_word(&mut session, "北京"), "北京");
    let view = session.view();
    assert!(view.preedit.is_empty());
    assert!(view.candidates.is_empty());
    assert!(!view.show_lookup);
    assert!(matches!(session.handle(0x31, 0), Effect::Ignore));
    assert!(matches!(session.handle(0x20, 0), Effect::Ignore));

    let twice = UserDict::from_log("B\t1\t北京\t大雪\t2\tda'xue\n");
    let mut twice = Session::with_user(lex, twice);
    type_in(&mut twice, "beijing");
    assert_eq!(pick_word(&mut twice, "北京"), "北京");
    let view = twice.view();
    assert!(view.candidates.is_empty(), "{:?}", view.candidates);
    assert!(!view.show_lookup);
}

#[test]
fn repeated_link_lifts_the_next_word_while_typing() {
    let lex = Arc::new(Lexicon::load());
    let mut bare = Session::new(Arc::clone(&lex));
    type_in(&mut bare, "daxue");
    let original = bare.view().candidates[0].clone();
    assert_ne!(original, "大雪");

    let once = UserDict::from_log("B\t1\t北京\t大雪\t1\tda'xue\n");
    let mut once = Session::with_user(Arc::clone(&lex), once);
    type_in(&mut once, "beijing");
    assert_eq!(pick_word(&mut once, "北京"), "北京");
    type_in(&mut once, "daxue");
    assert_eq!(once.view().candidates[0], original);

    let twice = UserDict::from_log("B\t1\t北京\t大雪\t2\tda'xue\n");
    let mut twice = Session::with_user(lex, twice);
    type_in(&mut twice, "beijing");
    assert_eq!(pick_word(&mut twice, "北京"), "北京");
    type_in(&mut twice, "daxue");
    assert_eq!(twice.view().candidates[0], "大雪");
}

#[test]
fn ctrl_digit_without_a_user_record_is_ignored() {
    let lex = Arc::new(Lexicon::load());
    let mut session = Session::new(lex);
    type_in(&mut session, "nihao");
    assert!(matches!(session.handle(0x31, 1 << 2), Effect::Ignore));
}

#[test]
fn public_lists_cover_frequency_gaps() {
    let lex = Lexicon::load();
    assert!(
        suggest("ai", &lex).iter().any(|word| word == "砹"),
        "rare GB2312 character missing"
    );
    assert_eq!(
        suggest("wohui", &lex).first().map(String::as_str),
        Some("我会")
    );
    assert_eq!(
        suggest("xieyao", &lex).first().map(String::as_str),
        Some("谢邀")
    );
    assert!(
        suggest("adingding", &lex)
            .iter()
            .any(|word| word == "吖丁啶"),
        "CC-CEDICT word missing"
    );
}

#[test]
fn open_lexicon_adds_modern_words() {
    let lex = Lexicon::load();
    assert_eq!(
        suggest("weixin", &lex).first().map(String::as_str),
        Some("微信")
    );
    assert_eq!(
        suggest("douyin", &lex).first().map(String::as_str),
        Some("抖音")
    );
    assert_eq!(
        suggest("haha", &lex).first().map(String::as_str),
        Some("哈哈")
    );
    assert_eq!(
        suggest("abalaqiya", &lex).first().map(String::as_str),
        Some("阿巴拉契亚")
    );
}

#[test]
fn computer_terms_stay_on_the_first_pages() {
    let lex = Lexicon::load();
    let page = |input: &str, word: &str| {
        suggest(input, &lex)
            .iter()
            .position(|item| item == word)
            .unwrap_or(usize::MAX)
    };
    let first = |input: &str| suggest(input, &lex).first().cloned().unwrap_or_default();

    for (input, word) in [
        ("shijianxunhuan", "事件循环"),
        ("tuichuma", "退出码"),
        ("yecuowu", "页错误"),
        ("yingzhongduan", "硬中断"),
        ("huancunhang", "缓存行"),
        ("zhongjianbiaoshi", "中间表示"),
        ("zhuangdaima", "桩代码"),
    ] {
        assert_eq!(first(input), word, "{input}");
    }
    for (input, word, ahead) in [
        ("duizhan", "堆栈", "对战"),
        ("queye", "缺页", "却也"),
        ("daduan", "大端", "打断"),
        ("xiaoduan", "小端", "小段"),
        ("fenzhi", "分治", "分支"),
        ("tulun", "图论", "凸轮"),
        ("yanzhi", "盐值", "研制"),
    ] {
        assert_eq!(first(input), ahead, "{input}");
        let at = page(input, word);
        assert!(at < PAGE_SIZE, "{word} at {at} for {input}");
    }
    for (input, word, ahead) in [
        ("zhan", "栈", "站"),
        ("zhen", "帧", "真"),
        ("hong", "宏", "红"),
        ("ku", "库", "哭"),
        ("zhuang", "桩", "撞"),
    ] {
        assert_eq!(first(input), ahead, "{input}");
        let at = page(input, word);
        assert!(
            at < PAGE_SIZE * 2,
            "{word} at {at} for {input}: {:?}",
            suggest(input, &lex)
        );
    }
}

#[test]
fn unweighted_dictionary_words_stay_typeable() {
    let lex = Lexicon::load();
    let list = suggest("heru", &lex);
    assert_eq!(list.first().map(String::as_str), Some("何如"));
    let at = list
        .iter()
        .position(|word| word == "合入")
        .unwrap_or(usize::MAX);
    assert!(at < PAGE_SIZE, "合入 at {at}: {list:?}");

    let faban = suggest("faban", &lex);
    let ban = faban.iter().position(|word| word == "法办").expect("法办");
    let fa = faban.iter().position(|word| word == "发版").expect("发版");
    assert!(ban < fa, "法办 at {ban}, 发版 at {fa}: {faban:?}");

    let maidian = suggest("maidian", &lex);
    let mai = maidian
        .iter()
        .position(|word| word == "卖点")
        .expect("卖点");
    let buried = maidian
        .iter()
        .position(|word| word == "埋点")
        .expect("埋点");
    assert!(mai < buried, "卖点 at {mai}, 埋点 at {buried}: {maidian:?}");
}

#[test]
fn adjacent_key_still_offers_the_word() {
    let lex = Lexicon::load();
    let missed = suggest("nihso", &lex);
    assert!(
        missed.iter().take(PAGE_SIZE).any(|word| word == "你好"),
        "{missed:?}"
    );
    let extra = suggest("nihhao", &lex);
    assert!(
        extra.iter().take(PAGE_SIZE).any(|word| word == "你好"),
        "{extra:?}"
    );
    let missing = suggest("zhonguo", &lex);
    assert!(
        missing.iter().take(PAGE_SIZE).any(|word| word == "中国"),
        "{missing:?}"
    );
    assert_eq!(
        suggest("nihao", &lex).first().map(String::as_str),
        Some("你好")
    );
    assert_eq!(
        suggest("jt", &lex).first().map(String::as_str),
        Some("今天")
    );
    assert_eq!(
        suggest("mt", &lex).first().map(String::as_str),
        Some("明天")
    );
    assert_eq!(
        suggest("zt", &lex).first().map(String::as_str),
        Some("昨天")
    );
}

#[test]
fn caret_inserts_and_deletes_in_the_middle() {
    let mut session = Session::new(Arc::new(Lexicon::load()));
    type_in(&mut session, "nihao");
    assert_eq!(session.view().caret, 5);
    session.handle(0xff51, 0);
    session.handle(0xff51, 0);
    assert_eq!(session.view().caret, 3);
    session.handle(u32::from(b'x'), 0);
    assert_eq!(session.view().preedit, "nihxao");
    assert_eq!(session.view().caret, 4);
    session.handle(0xff08, 0);
    assert_eq!(session.view().preedit, "nihao");
    assert_eq!(session.view().caret, 3);
    session.handle(0xff52, 0);
    assert_eq!(session.view().cursor, 0);
    session.handle(0xff54, 0);
    assert_eq!(session.view().cursor, 1);
}

#[test]
fn shift_preserves_pinyin_and_punctuation_widens() {
    let mut session = Session::new(Arc::new(Lexicon::load()));
    for shift in [0xffe1, 0xffe2] {
        assert!(matches!(session.handle(shift, 0), Effect::Ignore));
        assert!(matches!(session.handle(shift, 1 << 30), Effect::Ignore));
        type_in(&mut session, "ni");
        let before = session.view();
        assert!(matches!(session.handle(shift, 0), Effect::Ignore));
        assert!(matches!(session.handle(shift, 1 << 30), Effect::Ignore));
        let after = session.view();
        assert_eq!(after.preedit, before.preedit);
        assert_eq!(after.caret, before.caret);
        assert_eq!(after.cursor, before.cursor);
        assert_eq!(after.candidates, before.candidates);
        type_in(&mut session, "hao");
        assert_eq!(committed(session.handle(0x20, 0)), "你好");
        assert!(matches!(session.handle(shift, 0), Effect::Ignore));
        assert!(matches!(session.handle(u32::from(b'A'), 1), Effect::Ignore));
        assert!(matches!(session.handle(shift, 1 << 30), Effect::Ignore));
    }
    assert_eq!(committed(session.handle(u32::from(b'"'), 0)), "“");
    assert_eq!(committed(session.handle(u32::from(b'"'), 0)), "”");
    assert_eq!(committed(session.handle(u32::from(b'('), 0)), "（");
    assert_eq!(committed(session.handle(u32::from(b')'), 0)), "）");
    assert_eq!(committed(session.handle(u32::from(b'['), 0)), "【");
    assert!(matches!(session.handle(u32::from(b'<'), 0), Effect::Ignore));
    type_in(&mut session, "nihao");
    assert_eq!(committed(session.handle(u32::from(b'<'), 0)), "你好《");
}

#[test]
fn dense_syllable_keeps_a_wide_pool_and_loads_two_pages() {
    assert_eq!(PAGE_SIZE, 5);
    let lex = Arc::new(Lexicon::load());
    let full = suggest("yi", &lex);
    assert!(full.len() > 15, "pool {}", full.len());
    let ranked = choices("yi", &lex);
    let body: Vec<_> = ranked
        .iter()
        .filter(|choice| choice.span == 2)
        .skip(2)
        .collect();
    for pair in body.windows(2) {
        let ordered = pair[0].tier < pair[1].tier
            || (pair[0].tier == pair[1].tier && pair[0].score >= pair[1].score);
        assert!(
            ordered,
            "{} ({}) before {} ({})",
            pair[0].word, pair[0].score, pair[1].word, pair[1].score
        );
    }
    let mut session = Session::new(lex);
    type_in(&mut session, "yi");
    assert_eq!(session.loaded(), PAGE_SIZE * 2);
    let shown = session.view().candidates.clone();
    assert_eq!(shown, full[..shown.len()]);
    session.handle(0xff56, 0);
    let loaded = session.loaded();
    assert!(
        loaded > PAGE_SIZE * 2 && loaded <= PAGE_SIZE * 3,
        "{loaded}"
    );
    let shown = session.view().candidates.clone();
    assert_eq!(shown, full[..shown.len()]);
}

#[test]
fn sentence_end_closes_association_and_drops_the_link() {
    let lex = Arc::new(Lexicon::load());
    let mut session = Session::new(Arc::clone(&lex));
    type_in(&mut session, "nihao");
    assert_eq!(committed(session.handle(u32::from(b'.'), 0)), "你好。");
    assert!(!session.view().show_lookup);

    type_in(&mut session, "nihao");
    assert_eq!(committed(session.handle(u32::from(b','), 0)), "你好，");
    assert!(!session.view().show_lookup);

    for key in [0xff0eu32, u32::from(b'!'), u32::from(b'?')] {
        type_in(&mut session, "beijing");
        assert_eq!(pick_word(&mut session, "北京"), "北京");
        assert!(!session.view().show_lookup);
        let text = committed(session.handle(key, 0));
        assert!(text == "。" || text == "！" || text == "？", "{text}");
        assert!(!session.view().show_lookup);
        for _ in 0..2 {
            type_in(&mut session, "zaijian");
            assert_eq!(pick_word(&mut session, "再见"), "再见");
            assert_eq!(committed(session.handle(u32::from(b'.'), 0)), "。");
            assert!(!session.view().show_lookup);
        }
        type_in(&mut session, "beijing");
        assert_eq!(pick_word(&mut session, "北京"), "北京");
        assert!(
            !session.view().candidates.iter().any(|word| word == "再见"),
            "{:?}",
            session.view().candidates
        );
        assert_eq!(committed(session.handle(0xff0e, 0)), "。");
    }
}

#[test]
fn umlaut_spellings_reach_the_stored_syllable() {
    let lex = Lexicon::load();
    let on_first_page = |input: &str, word: &str| {
        let list = suggest(input, &lex);
        let at = list
            .iter()
            .position(|item| item == word)
            .unwrap_or_else(|| panic!("{word} missing from {}", list.len()));
        assert!(at < PAGE_SIZE, "{input} {word} at {at}");
    };
    on_first_page("celue", "策略");
    on_first_page("celve", "策略");
    on_first_page("nue", "虐待");
    on_first_page("nve", "虐待");
    on_first_page("jve", "觉得");
    on_first_page("jue", "觉得");
    on_first_page("xve", "学");
    on_first_page("qvan", "全");
    let yvan = suggest("yvan", &lex);
    let at = yvan.iter().position(|word| word == "元").expect("元");
    assert!(at < PAGE_SIZE * 2, "yvan 元 at {at}");
    assert_eq!(suggest("lu", &lex).first().map(String::as_str), Some("路"));
    assert_eq!(
        suggest("lv", &lex).first().map(String::as_str),
        Some("律师")
    );
}

#[test]
fn unlisted_phrase_is_built_from_its_characters() {
    let lex = Arc::new(Lexicon::load());
    let list = suggest("budanzhi", &lex);
    assert!(
        list.iter().any(|word| word == "不单止"),
        "不单止 missing, {} candidates",
        list.len()
    );
    let rows = choices("budanzhi", &lex);
    assert!(rows.iter().any(|row| row.word == "不单" && row.span == 5));
    let mut session = Session::new(lex);
    type_in(&mut session, "budanzhi");
    assert!(
        session.view().candidates.iter().any(|word| word == "不单"),
        "{:?}",
        session.view().candidates
    );
}

#[test]
fn personal_links_do_not_open_an_association_list() {
    let lex = Arc::new(Lexicon::load());
    let personal = ["甲", "乙", "丙", "丁", "戊"];
    let log: String = personal
        .iter()
        .map(|word| format!("B\t1\t北京\t{word}\t2\tbei\n"))
        .collect();
    let mut session = Session::with_user(lex, UserDict::from_log(&log));
    type_in(&mut session, "beijing");
    assert_eq!(pick_word(&mut session, "北京"), "北京");
    let view = session.view();
    assert!(!view.show_lookup);
    assert!(view.candidates.is_empty(), "{:?}", view.candidates);
}

#[test]
fn common_vocabulary_regression() {
    let lex = Lexicon::load();
    let mut first = 0;
    for line in include_str!("../tests/common.tsv").lines() {
        let (raw, expected) = line.split_once('\t').unwrap();
        let list = suggest(raw, &lex);
        first += usize::from(list.first().is_some_and(|w| w == expected));
        assert!(
            list.iter().take(PAGE_SIZE).any(|w| w == expected),
            "{raw}: {expected}"
        );
    }
    assert_eq!(first, 200);
}

#[test]
fn imported_shortcut_works_without_a_lexicon_match() {
    let mut session = Session::new(Arc::new(Lexicon::load()));
    type_in(&mut session, "z");
    session.set_imported(vec![crate::imports::Record {
        code: "zzzzzz".into(),
        word: "马上到！".into(),
        shortcut: true,
    }]);
    type_in(&mut session, "zzzzz");
    assert_eq!(session.view().candidates[0], "马上到！");
    assert_eq!(committed(session.handle(0x20, 0)), "马上到！");
}

#[test]
fn imported_phrases_require_all_syllables() {
    use crate::lexicon::match_imported_reading;
    assert!(
        match_imported_reading(Default::default(), "nihao", "ni'hao'a").is_none_or(|h| h.span < 5)
    );
    assert!(match_imported_reading(Default::default(), "ni'hao", "ni'hao'a").is_none());
    assert!(match_imported_reading(Default::default(), "nihao", "ni'hao").is_some());
    assert!(match_imported_reading(Default::default(), "nh", "ni'hao").is_some());
    assert!(match_imported_reading(Default::default(), "niha", "ni'hao").is_some());
}

#[test]
fn daily_sentence_composition_keeps_common_words_together() {
    let lex = Lexicon::load();
    for (raw, expected) in [
        ("womenxiacizailiao", "我们下次再聊"),
        ("shurufaciku", "输入法词库"),
        ("denghuizailiao", "等会再聊"),
        ("wozaikaihui", "我在开会"),
        ("mingtiantianqi", "明天天气"),
    ] {
        assert_eq!(
            suggest(raw, &lex).first().map(String::as_str),
            Some(expected),
            "{raw}"
        );
        let mut session = Session::new(Arc::new(Lexicon::load()));
        type_in(&mut session, raw);
        assert_eq!(session.view().candidates[0], expected, "{raw}");
    }
}

#[test]
fn exact_imported_phrase_beats_generated_forms_but_not_common_homophones() {
    let mut session = Session::new(Arc::new(Lexicon::load()));
    type_in(&mut session, "h");
    session.set_imported(vec![crate::imports::Record {
        code: "hao'jiu'bu'jian'le'o".into(),
        word: "好久不见了哦".into(),
        shortcut: false,
    }]);
    type_in(&mut session, "aojiubujianleo");
    assert_eq!(session.view().candidates[0], "好久不见了哦");
    session.clear();
    type_in(&mut session, "b");
    session.set_imported(vec![crate::imports::Record {
        code: "bao'xian".into(),
        word: "爆线".into(),
        shortcut: false,
    }]);
    type_in(&mut session, "aoxian");
    assert_eq!(session.view().candidates[0], "保险");
}

#[test]
fn sentence_inputs_cover_full_initial_and_mixed_spellings() {
    let lex = Arc::new(Lexicon::load());
    let mut first = [0usize; 3];
    let mut page = 0;
    let mut total = 0;
    for line in include_str!("../tests/sentences.tsv")
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
    {
        let cols: Vec<_> = line.split('\t').collect();
        for (mode, raw) in cols[1..].iter().enumerate() {
            let mut session = Session::new(Arc::clone(&lex));
            type_in(&mut session, raw);
            let view = session.view();
            total += 1;
            if view.candidates.first().is_some_and(|w| w == cols[0]) {
                first[mode] += 1;
                assert_eq!(view.spans[0], raw.len());
                assert_eq!(committed(session.handle(0x20, 0)), cols[0]);
                assert!(session.view().preedit.is_empty());
            }
            page += usize::from(view.candidates.iter().take(PAGE_SIZE).any(|w| w == cols[0]));
        }
    }
    assert_eq!(total, 75);
    assert_eq!(first[0], 25, "full spelling: {first:?}");
    assert!(
        first[1] >= 24 && first[2] == 25,
        "abbreviation/mixed: {first:?}"
    );
    assert!(page >= 74, "first page: {page}/75");
}

#[test]
fn initials_choose_complete_words_before_partial_prefixes() {
    let lex = Arc::new(Lexicon::load());
    for (raw, expected) in [
        ("wshm", "为什么"),
        ("zguoren", "中国人"),
        ("xqcf", "先去吃饭"),
        ("wmmtyqcf", "我们明天一起吃饭"),
        ("wxzzkh", "我现在在开会"),
        ("jtwsyqcf", "今天晚上一起吃饭"),
        ("wmxczl", "我们下次再聊"),
    ] {
        let mut session = Session::new(Arc::clone(&lex));
        type_in(&mut session, raw);
        assert_eq!(session.view().candidates[0], expected, "{raw}");
    }
}

#[test]
fn sentence_candidates_keep_readings_and_explicit_boundaries() {
    let lex = Arc::new(Lexicon::load());
    for raw in [
        "wo'ming'tian'shang'wu'yao'kai'hui",
        "w'm't'sh'w'y'k'h",
        "wo'mt'sw'y'kaihui",
    ] {
        let mut session = Session::new(Arc::clone(&lex));
        type_in(&mut session, raw);
        assert_eq!(session.view().candidates[0], "我明天上午要开会", "{raw}");
        assert_eq!(committed(session.handle(0x20, 0)), "我明天上午要开会");
    }
    for c in choices("wxzzkh", &lex).into_iter().take(3) {
        assert!(!c.reading.is_empty(), "{}", c.word);
        let hit = crate::lexicon::match_imported_reading(Default::default(), "wxzzkh", &c.reading)
            .unwrap();
        assert_eq!(hit.span, 6);
    }
}

#[test]
fn ambiguous_long_initial_input_has_a_bounded_fallback() {
    let lex = Lexicon::load();
    let started = std::time::Instant::now();
    let raw = "x".repeat(96);
    let list = suggest(&raw, &lex);
    assert!(!list.is_empty());
    assert!(started.elapsed().as_secs() < 5, "unbounded sentence search");
}

#[test]
fn session_preferences_control_user_fuzzy_heads() {
    let lex = Arc::new(Lexicon::load());
    let user = UserDict::from_log("U\t1\tla\t拿\t8\t0\n");
    let mut prefs = crate::InputPreferences::default();
    prefs.groups[6] = true;
    let mut on = Session::with_preferences(lex.clone(), user, prefs);
    type_in(&mut on, "na");
    assert_eq!(on.view().candidates[0], "拿");
    let user = UserDict::from_log("U\t1\tla\t拿\t8\t0\n");
    prefs.enabled = false;
    let mut off = Session::with_preferences(lex, user, prefs);
    type_in(&mut off, "na");
    assert_ne!(off.view().candidates[0], "拿");
}

#[test]
fn imported_preferences_preserve_shortcut_exactness() {
    use crate::imports::{Catalog, Record};
    let catalog = Catalog::from_records(vec![
        Record {
            code: "zan".into(),
            word: "测试音组".into(),
            shortcut: false,
        },
        Record {
            code: "zan".into(),
            word: "测试快捷".into(),
            shortcut: true,
        },
    ]);
    let prefs = crate::InputPreferences::default();
    let fuzzy = catalog.choices_with_preferences("zhang", prefs);
    assert!(fuzzy
        .iter()
        .any(|c| c.word == "测试音组" && c.reading == "zan" && c.span == 5));
    assert!(fuzzy.iter().all(|c| c.word != "测试快捷"));
    assert!(catalog
        .choices_with_preferences("zan", prefs)
        .iter()
        .any(|c| c.word == "测试快捷"));
    let off = crate::InputPreferences {
        enabled: false,
        ..prefs
    };
    assert!(catalog.choices_with_preferences("zhang", off).is_empty());
}
