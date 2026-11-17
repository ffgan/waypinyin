//! Frozen pre-optimization beam search: a differential oracle, not runtime code.
use super::*;

#[derive(Clone)]
struct SentencePath {
    cost: f64,
    text: String,
    reading: Vec<String>,
    words: usize,
}

fn compose_sentences(input: &str, lex: &Lexicon) -> Vec<(String, Vec<String>)> {
    if input.len() < 4
        || input.len() > 128
        || !input.bytes().all(|c| c.is_ascii_lowercase() || c == b'\'')
    {
        return Vec::new();
    }
    let n = input.len();
    let fully_spelled = exact_syls(input, lex).is_some();
    let mut paths: Vec<Vec<SentencePath>> = vec![Vec::new(); n + 1];
    paths[0].push(SentencePath {
        cost: 0.0,
        text: String::new(),
        reading: Vec::new(),
        words: 0,
    });
    let mut budget = 60_000usize;
    for start in 0..n {
        if budget == 0 {
            break;
        }
        if paths[start].is_empty() {
            continue;
        }
        paths[start].sort_by(|a, b| a.cost.total_cmp(&b.cost).then(a.text.cmp(&b.text)));
        paths[start].truncate(8);
        let parents = paths[start].clone();
        let begin = skip_sep(input, start);
        if begin == n {
            continue;
        }
        let mut stack = vec![(0u32, begin, 0.0)];
        let mut seen: HashMap<(u32, usize), f64> = HashMap::new();
        let mut visited = 0;
        while let Some((node, pos, penalty)) = stack.pop() {
            if seen.get(&(node, pos)).is_some_and(|old| *old <= penalty) {
                continue;
            }
            seen.insert((node, pos), penalty);
            visited += 1;
            if budget == 0 {
                break;
            }
            budget -= 1;
            if visited > 20_000 {
                break;
            }
            if node != 0 {
                let end = skip_sep(input, pos);
                for word in lex.node_words(node).iter().take(4) {
                    let text = lex.text(word.text);
                    let base = (lex.ln_corpus - (word.score.max(1) as f64).ln()).max(0.0) + penalty;
                    for parent in &parents {
                        let cost = parent.cost + base
                            - original_sentence_bonus(parent.text.chars().last(), text);
                        // Keep multiple endings: the next word can change which
                        // homophone is best. This is a bounded beam, not greedy.
                        let mut next = parent.clone();
                        next.cost = cost;
                        next.text.push_str(text);
                        if let Some(index) = paths[end].iter().position(|p| p.text == next.text) {
                            if paths[end][index].cost <= cost {
                                continue;
                            }
                            paths[end].swap_remove(index);
                        }
                        if paths[end].len() >= 16 {
                            let worst = paths[end]
                                .iter()
                                .enumerate()
                                .max_by(|(_, a), (_, b)| a.cost.total_cmp(&b.cost))
                                .unwrap()
                                .0;
                            if cost >= paths[end][worst].cost {
                                continue;
                            }
                            paths[end].swap_remove(worst);
                        }
                        next.reading.extend(path_syls(lex, node));
                        next.words += 1;
                        if !paths[end]
                            .iter()
                            .any(|p| p.text == next.text && p.cost <= cost)
                        {
                            paths[end].push(next);
                        }
                    }
                }
            }
            if pos >= n {
                continue;
            }
            let rest = &input[pos..];
            for edge in lex.edges_for(node, input.as_bytes()[pos]) {
                let syl = lex.syl(edge.syl);
                let mut lengths = Vec::new();
                if rest.starts_with(syl) {
                    lengths.push((syl.len(), 0.0));
                }
                if !fully_spelled && !matches!(syl.as_bytes()[0], b'a' | b'e' | b'o') {
                    lengths.push((1, 0.25 * (syl.len().saturating_sub(1)) as f64));
                    if (syl.starts_with("zh") || syl.starts_with("ch") || syl.starts_with("sh"))
                        && rest.starts_with(&syl[..2])
                    {
                        lengths.push((2, 0.25 * (syl.len().saturating_sub(2)) as f64));
                    }
                }
                for (len, extra) in lengths {
                    if pos + len <= n {
                        stack.push((edge.node, skip_sep(input, pos + len), penalty + extra));
                    }
                }
            }
        }
    }
    paths[n].sort_by(|a, b| a.cost.total_cmp(&b.cost).then(a.text.cmp(&b.text)));
    let mut seen = HashSet::new();
    paths[n]
        .iter()
        .filter(|p| p.words >= 2 && seen.insert(p.text.clone()))
        .take(3)
        .map(|p| (p.text.clone(), p.reading.clone()))
        .collect()
}

#[test]
fn optimized_beam_matches_reference() {
    let lex = Lexicon::load();
    let mut inputs = std::collections::BTreeSet::new();
    for line in include_str!("../tests/sentences.tsv")
        .lines()
        .filter(|line| !line.starts_with('#') && !line.is_empty())
    {
        for raw in line.split('\t').skip(1) {
            for end in 1..=raw.len() {
                inputs.insert(raw[..end].to_string());
            }
        }
    }
    for raw in ["ni'hao", "xuexiwan", "sou", "lue", "zan", "x'x'x'x'x'x'x'x"] {
        inputs.insert(raw.into());
    }
    inputs.insert("x".repeat(96));
    inputs.insert("x".repeat(129));
    for raw in inputs {
        assert_eq!(
            super::compose_sentences(&raw, &lex, exact_syls(&raw, &lex).is_some()),
            compose_sentences(&raw, &lex),
            "{raw}"
        );
    }
}

struct PhraseState {
    score: u32,
    idx: Vec<u8>,
}

impl PartialEq for PhraseState {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for PhraseState {}

impl PartialOrd for PhraseState {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PhraseState {
    fn cmp(&self, other: &Self) -> Ordering {
        self.score
            .cmp(&other.score)
            .then_with(|| other.idx.cmp(&self.idx))
    }
}

/// Single characters for each exact syllable, best combinations first.
/// `budanzhi` has no stored 不单止; the characters still combine here.
fn assemble_phrases(syls: &[u16], lex: &Lexicon, limit: usize) -> Vec<(String, i64, String)> {
    if syls.len() < 2 || limit == 0 {
        return Vec::new();
    }
    let reading = joined_reading(syls, lex);
    let mut cols = Vec::new();
    for syl in syls {
        let Some(node) = lex.walk_id(0, *syl) else {
            return Vec::new();
        };
        let mut col = Vec::new();
        for word in lex.node_words(node) {
            let text = lex.text(word.text);
            if word.score <= 0 || text.chars().count() != 1 {
                continue;
            }
            col.push((text.to_string(), word.score, word.score.max(1).ilog2()));
            if col.len() == PHRASE_COLUMN {
                break;
            }
        }
        if col.is_empty() {
            return Vec::new();
        }
        cols.push(col);
    }
    let width = cols.len();
    let start = vec![0u8; width];
    let mut seen = HashSet::new();
    seen.insert(start.clone());
    let mut heap = BinaryHeap::new();
    heap.push(PhraseState {
        score: cols.iter().map(|col| col[0].2).sum(),
        idx: start,
    });
    let mut out = Vec::new();
    while let Some(state) = heap.pop() {
        if out.len() == limit {
            break;
        }
        let mut word = String::new();
        let mut score = i64::MAX;
        for (col, &index) in cols.iter().zip(&state.idx) {
            let (text, char_score, _) = &col[index as usize];
            word.push_str(text);
            score = score.min(*char_score);
        }
        out.push((word, score, reading.clone()));
        for dim in 0..width {
            let next = state.idx[dim] as usize + 1;
            if next >= cols[dim].len() {
                continue;
            }
            let mut idx = state.idx.clone();
            idx[dim] = next as u8;
            if !seen.insert(idx.clone()) {
                continue;
            }
            let total = cols
                .iter()
                .zip(&idx)
                .map(|(col, &index)| col[index as usize].2)
                .sum();
            heap.push(PhraseState { score: total, idx });
        }
    }
    out
}

#[test]
fn packed_phrase_enumeration_matches_reference() {
    let lex = Lexicon::load();
    for raw in ["nihao", "budanzhi", "xuexiwan", "zhongguo"] {
        if let Some(syls) = exact_syls(raw, &lex) {
            for limit in [0, 1, 7, 128, PHRASE_LIMIT] {
                assert_eq!(
                    super::assemble_phrases(&syls, &lex, limit),
                    assemble_phrases(&syls, &lex, limit),
                    "{raw}/{limit}"
                );
            }
        }
    }
    let ni = lex.syllable_id("ni").unwrap();
    for width in [25, 26] {
        let syls = vec![ni; width];
        assert_eq!(
            super::assemble_phrases(&syls, &lex, 64),
            assemble_phrases(&syls, &lex, 64)
        );
    }
}

fn original_piece_match(prefs: crate::InputPreferences, piece: &str, syl: &str) -> Option<Kind> {
    if let Some(kind) = piece_match_plain(prefs, piece, syl) {
        return Some(kind);
    }
    // ü is stored as v (lüe → lve) and, after j/q/x/y, as u (jue). The other
    // spelling is the same syllable: lue, nue, jve, qvan, xve, yvan.
    let alt = umlaut_alias(syl)?;
    let piece_b = piece.as_bytes();
    let alt_b = alt.as_bytes();
    if piece_b.len() < 2 || piece_b.get(1) != alt_b.get(1) {
        return None;
    }
    piece_match_plain(prefs, piece, &alt)
}

#[test]
fn umlaut_rejection_matches_reference() {
    let lex = Lexicon::load();
    let syllables: Vec<_> = (0..lex.syls.off.len() - 1)
        .map(|id| lex.syl(id as u16))
        .collect();
    let mut pieces = std::collections::BTreeSet::new();
    for syl in &syllables {
        for end in 1..=syl.len() {
            pieces.insert(syl[..end].to_string());
        }
        if let Some(alias) = umlaut_alias(syl) {
            for end in 1..=alias.len() {
                pieces.insert(alias[..end].to_string());
            }
        }
    }
    for prefs in [
        crate::InputPreferences::default(),
        crate::InputPreferences {
            enabled: false,
            ..Default::default()
        },
    ] {
        for syl in &syllables {
            for piece in &pieces {
                assert_eq!(
                    super::piece_match(prefs, piece, syl).map(|k| k as u8),
                    original_piece_match(prefs, piece, syl).map(|k| k as u8),
                    "{piece}/{syl}"
                );
            }
        }
    }
}

#[test]
fn deferred_learning_floor_matches_reference() {
    let lex = Lexicon::load();
    let plan = Arc::new(plan("ni", &lex));
    let expected = peek_ranked(&plan, &lex).map_or(100, |r| r.score / 2);
    let mut learned = plain_choice("个人词".into(), 2);
    learned.user_weight = 1;
    let pool = Pool::open(plan, &lex, vec![learned], Vec::new());
    assert_eq!(pool.users[0].score, expected);
}

#[test]
fn phrase_cursor_stops_after_the_requested_first_page() {
    let lex = Lexicon::load();
    let syls = exact_syls("budanzhi", &lex).unwrap();
    let mut cursor = super::phrase_cursor(&syls, &lex, PHRASE_LIMIT);
    let head = cursor.by_ref().take(5).collect::<Vec<_>>();
    assert_eq!(head, assemble_phrases(&syls, &lex, 5));
    match &cursor {
        super::PhraseCursor::Packed(c) => {
            assert_eq!(c.remaining, PHRASE_LIMIT - 5);
            assert!(c.seen.len() < PHRASE_LIMIT / 2);
        }
        _ => panic!("expected packed phrase cursor"),
    }
    let mut complete = head;
    complete.extend(cursor);
    assert_eq!(complete, assemble_phrases(&syls, &lex, PHRASE_LIMIT));
}

fn original_sentence_bonus(previous: Option<char>, word: &str) -> f64 {
    static PAIRS: OnceLock<HashMap<(char, char), u32>> = OnceLock::new();
    let pairs = PAIRS.get_or_init(|| {
        let mut pairs = HashMap::new();
        for line in include_str!("../data/sentences.txt")
            .lines()
            .filter(|l| !l.starts_with('#'))
        {
            let chars: Vec<_> = line.chars().collect();
            for pair in chars.windows(2) {
                *pairs.entry((pair[0], pair[1])).or_insert(0) += 1;
            }
        }
        pairs
    });
    let mut prev = previous;
    let mut bonus = 0.0;
    for ch in word.chars() {
        if let Some(p) = prev {
            if let Some(count) = pairs.get(&(p, ch)) {
                bonus += 2.0 * (f64::from(*count)).ln_1p().min(3.0);
            } else {
                bonus -= 1.5;
            }
        }
        prev = Some(ch);
    }
    bonus
}

#[test]
fn precomputed_pair_weights_match_original_float_bits() {
    let lex = Lexicon::load();
    for previous in [
        None,
        Some('我'),
        Some('的'),
        Some('。'),
        Some('罕'),
        Some('😀'),
    ] {
        for id in (0..lex.texts.off.len() - 1).step_by(97).take(2000) {
            let word = lex.text(id as u32);
            assert_eq!(
                sentence_bonus(previous, word).to_bits(),
                original_sentence_bonus(previous, word).to_bits(),
                "{previous:?}/{word}",
            );
        }
    }
}
