//! Turn the text dictionaries into a lexicon image. This runs from `build.rs`.

use crate::image::{encode, BigramRec, EdgeRec, NodeRec, RangeRec, Tables, WordRec};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

pub fn compile() -> Vec<u8> {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let data = source_dir(&PathBuf::from(manifest));
    let read = |name: &str| -> String {
        std::fs::read_to_string(data.join(name)).unwrap_or_else(|err| {
            panic!(
                "read {}: {err}; provide a complete WAYPINYIN_SOURCE_DATA directory",
                data.join(name).display()
            )
        })
    };
    let mut extra = read("extra.txt");
    extra.push('\n');
    extra.push_str(&read("terms.txt"));
    let built = assemble(
        &read("dict.txt"),
        &extra,
        &read("frost.txt"),
        &read("cedict.txt"),
        &read("freq.txt"),
        &read("ts-characters.txt"),
        &read("pinyin.txt"),
        &read("gb2312.txt"),
    );
    freeze(built)
}

/// Preserve the local complete corpus; public exports fall back to original demo inputs.
pub fn source_dir(manifest: &std::path::Path) -> PathBuf {
    if let Some(path) = std::env::var_os("WAYPINYIN_SOURCE_DATA") {
        let path = PathBuf::from(path);
        assert!(
            path.is_absolute() && path.is_dir(),
            "WAYPINYIN_SOURCE_DATA must name an existing absolute directory"
        );
        return path;
    }
    if manifest.join("data/dict.txt").is_file() {
        manifest.join("data")
    } else {
        manifest.join("tests/fixtures/demo")
    }
}

struct Built {
    texts: Vec<String>,
    syls: Vec<String>,
    nodes: Vec<TrieNode>,
    edges: Vec<TrieEdge>,
    auto: Vec<[u32; 26]>,
    auto_term: Vec<u16>,
    exact: Vec<Bigram>,
    exact_at: Vec<(u32, u32)>,
    extend: Vec<Bigram>,
    extend_at: Vec<(u32, u32)>,
    ln_corpus: f64,
    word_at: Vec<u32>,
}

fn freeze(built: Built) -> Vec<u8> {
    let mut words = Vec::new();
    let nodes: Vec<NodeRec> = built
        .nodes
        .iter()
        .map(|node| {
            let word_lo = u32::try_from(words.len()).expect("word table");
            for word in &node.words {
                words.push(WordRec {
                    text: word.text,
                    seq: word.seq,
                    score: word.score,
                });
            }
            NodeRec {
                parent: node.parent,
                edge_lo: node.edge_lo,
                word_lo,
                word_n: u32::try_from(node.words.len()).expect("word list"),
                syl: node.syl,
                depth: node.depth,
                edge_n: node.edge_n,
                _pad: 0,
            }
        })
        .collect();
    let edges: Vec<EdgeRec> = built
        .edges
        .iter()
        .map(|edge| EdgeRec {
            node: edge.node,
            syl: edge.syl,
            letter: edge.letter,
            _pad: 0,
        })
        .collect();
    let exact: Vec<BigramRec> = built
        .exact
        .iter()
        .map(|item| BigramRec {
            text: item.text,
            _pad: 0,
            score: item.score,
        })
        .collect();
    let extend: Vec<BigramRec> = built
        .extend
        .iter()
        .map(|item| BigramRec {
            text: item.text,
            _pad: 0,
            score: item.score,
        })
        .collect();
    let exact_at: Vec<RangeRec> = built
        .exact_at
        .iter()
        .map(|(start, end)| RangeRec {
            start: *start,
            end: *end,
        })
        .collect();
    let extend_at: Vec<RangeRec> = built
        .extend_at
        .iter()
        .map(|(start, end)| RangeRec {
            start: *start,
            end: *end,
        })
        .collect();
    encode(&Tables {
        texts: &built.texts,
        syls: &built.syls,
        nodes: &nodes,
        words: &words,
        edges: &edges,
        auto: &built.auto,
        auto_term: &built.auto_term,
        exact: &exact,
        exact_at: &exact_at,
        extend: &extend,
        extend_at: &extend_at,
        word_at: &built.word_at,
        ln_corpus: built.ln_corpus,
    })
}

#[allow(clippy::too_many_arguments)] // Independent source dictionaries; preserve ranking inputs.
fn assemble(
    dict: &str,
    extra: &str,
    frost: &str,
    cedict: &str,
    freq: &str,
    ts_characters: &str,
    pinyin: &str,
    gb2312: &str,
) -> Built {
    let mut word_freq: HashMap<String, u32> = HashMap::with_capacity(50_000);
    let mut char_freq: HashMap<char, u32> = HashMap::with_capacity(8_000);
    for line in freq.lines() {
        let mut parts = line.split_whitespace();
        let Some(word) = parts.next() else { continue };
        let Ok(count) = parts.next().unwrap_or("0").parse::<u32>() else {
            continue;
        };
        if count == 0 {
            continue;
        }
        word_freq.insert(word.to_string(), count);
        if word.chars().count() == 1 {
            if let Some(ch) = word.chars().next() {
                char_freq.insert(ch, count);
            }
        }
    }

    let traditional = traditional_chars(ts_characters);
    let primary = primary_readings(pinyin);
    let simplified: HashSet<char> = gb2312.chars().collect();
    let mut trie = TrieBuilder::new();

    // Each source has its own frequency scale. Align positive multi-character
    // weights using the median shared-word count ratio. Tiny coverage-only
    // weights (1..=3) never establish a calibration scale.
    let sources = [dict, extra, frost, cedict];
    let scales: Vec<f64> = sources
        .iter()
        .map(|source| {
            let mut ratios: Vec<f64> = source
                .lines()
                .filter_map(|line| {
                    let mut cols = line.split('\t');
                    let word = cols.next()?;
                    let _reading = cols.next()?;
                    let listed = cols.next()?.parse::<f64>().ok()?;
                    let count = *word_freq.get(word)?;
                    (listed > 3.0 && word.chars().count() > 1).then_some(f64::from(count) / listed)
                })
                .collect();
            if ratios.len() < 100 {
                return 1.0;
            }
            ratios.sort_by(f64::total_cmp);
            ratios[ratios.len() / 2].clamp(0.01, 100.0)
        })
        .collect();
    for (source_index, source) in sources.iter().enumerate() {
        for line in source.lines() {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut parts = line.split('\t');
            let Some(word) = parts.next() else { continue };
            let Some(pinyin) = parts.next() else { continue };
            let listed = parts
                .next()
                .and_then(|field| field.parse::<i64>().ok())
                .filter(|score| *score > 0)
                .unwrap_or(0);
            if word.is_empty()
                || word.chars().any(|ch| traditional.contains(&ch))
                || !pinyin.bytes().all(|b| b.is_ascii_lowercase() || b == b'\'')
            {
                continue;
            }
            let syls: Vec<&str> = pinyin.split('\'').filter(|s| !s.is_empty()).collect();
            if syls.is_empty() || syls.iter().any(|syl| !has_vowel(syl)) {
                continue;
            }
            if word.chars().count() == 1 {
                let ch = word.chars().next().unwrap();
                if !simplified.contains(&ch) || syls.len() != 1 {
                    continue;
                }
                let count = char_freq.get(&ch).copied().unwrap_or(0) as i64;
                // A listed score can lift a character the subtitles barely
                // counted, such as 栈. It only applies to the primary reading.
                let score = match primary.get(&ch) {
                    Some(reading) if reading == syls[0] => count.max(listed),
                    Some(_) => count / 200,
                    None => count.max(listed),
                };
                if score <= 0 {
                    continue;
                }
                trie.insert(&syls, word, score, count);
            } else if word.chars().all(|ch| simplified.contains(&ch)) {
                let count = word_freq.get(word).copied().map(i64::from).unwrap_or(0);
                // libime lists words such as 合入 with no count. Keep them
                // typeable, behind every word that has a real frequency.
                let calibrated = if listed <= 3 {
                    listed
                } else {
                    // Historical mixed sources include authored priors as well as
                    // corpus counts. Shrink toward the aligned count instead of
                    // erasing that prior on the strength of a sparse reference.
                    (listed as f64 * (0.5 + 0.5 * scales[source_index]))
                        .round()
                        .max(1.0) as i64
                };
                // Preserve established corpus anchors while calibrating words
                // missing from that corpus. This avoids displacing ordinary
                // homophones merely because a source used a different unit.
                let mut chars = word.chars();
                let unit = chars
                    .next_back()
                    .is_some_and(|ch| "点时分秒年月日天周岁".contains(ch));
                let numeric_unit = unit
                    && chars.clone().next().is_some()
                    && chars.all(|ch| "零〇一二两三四五六七八九十百千万".contains(ch));
                let score = if count > 0 || numeric_unit {
                    count.max(listed)
                } else {
                    calibrated.max(1)
                };
                trie.insert(&syls, word, score, count);
            }
        }
    }

    // Subtitle counts still rank the common characters. Every other GB2312
    // character with a kMandarin reading stays typeable at the bottom.
    let mut rare: Vec<char> = simplified.iter().copied().collect();
    rare.sort_unstable();
    for ch in rare {
        if ch.is_whitespace() || traditional.contains(&ch) {
            continue;
        }
        let Some(reading) = primary.get(&ch) else {
            continue;
        };
        if !has_vowel(reading) {
            continue;
        }
        let count = char_freq.get(&ch).copied().unwrap_or(0) as i64;
        let score = if count > 0 { count } else { 1 };
        trie.insert(&[reading.as_str()], &ch.to_string(), score, count);
    }

    let corpus = word_freq
        .values()
        .fold(0u64, |sum, count| sum.saturating_add(u64::from(*count)));
    let (texts, syls, nodes, edges, auto, auto_term, notes) = trie.finish();
    let (exact, exact_at, extend, extend_at) =
        index_bigrams(&texts, &auto, &auto_term, syls.len(), notes);
    let word_at = locate_words(&nodes, texts.len());
    Built {
        texts,
        syls,
        nodes,
        edges,
        auto,
        auto_term,
        exact,
        exact_at,
        extend,
        extend_at,
        ln_corpus: (corpus.max(1) as f64).ln(),
        word_at,
    }
}

#[derive(Clone, Copy)]
struct Bigram {
    text: u32,
    score: i64,
}

struct WordAt {
    text: u32,
    score: i64,
    seq: u32,
}

struct TrieNode {
    syl: u16,
    parent: u32,
    depth: u16,
    edge_lo: u32,
    edge_n: u16,
    words: Vec<WordAt>,
}

struct TrieEdge {
    node: u32,
    syl: u16,
    letter: u8,
}

struct RawNode {
    syl: u16,
    parent: u32,
    depth: u16,
    words: Vec<WordAt>,
}

struct TrieBuilder {
    syls: Vec<String>,
    syl_map: HashMap<String, u16>,
    texts: Vec<String>,
    text_map: HashMap<String, u32>,
    nodes: Vec<RawNode>,
    edges: HashMap<(u32, u16), u32>,
    bigrams: HashMap<(u16, u32), i64>,
}

impl TrieBuilder {
    fn new() -> Self {
        let mut nodes = Vec::with_capacity(1 << 20);
        nodes.push(RawNode {
            syl: u16::MAX,
            parent: u32::MAX,
            depth: 0,
            words: Vec::new(),
        });
        Self {
            syls: Vec::with_capacity(512),
            syl_map: HashMap::with_capacity(512),
            texts: Vec::with_capacity(1 << 20),
            text_map: HashMap::with_capacity(1 << 20),
            nodes,
            edges: HashMap::with_capacity(1 << 20),
            bigrams: HashMap::with_capacity(1 << 19),
        }
    }

    fn intern_syl(&mut self, syl: &str) -> u16 {
        if let Some(&id) = self.syl_map.get(syl) {
            return id;
        }
        assert!(self.syls.len() < u16::MAX as usize, "too many syllables");
        let id = self.syls.len() as u16;
        self.syls.push(syl.to_string());
        self.syl_map.insert(self.syls[id as usize].clone(), id);
        id
    }

    fn intern_text(&mut self, word: &str) -> u32 {
        if let Some(&id) = self.text_map.get(word) {
            return id;
        }
        let id = self.texts.len() as u32;
        self.texts.push(word.to_string());
        self.text_map.insert(self.texts[id as usize].clone(), id);
        id
    }

    fn insert(&mut self, syls: &[&str], word: &str, score: i64, count: i64) {
        if syls.is_empty() || word.is_empty() || score <= 0 {
            return;
        }
        let text = self.intern_text(word);
        let seq = freq_seq(count);
        let mut node = 0u32;
        let mut first = 0u16;
        for (index, syl) in syls.iter().enumerate() {
            let id = self.intern_syl(syl);
            if index == 0 {
                first = id;
            }
            let child = if let Some(&child) = self.edges.get(&(node, id)) {
                child
            } else {
                let child = self.nodes.len() as u32;
                let depth = self.nodes[node as usize].depth + 1;
                self.nodes.push(RawNode {
                    syl: id,
                    parent: node,
                    depth,
                    words: Vec::new(),
                });
                self.edges.insert((node, id), child);
                child
            };
            node = child;
        }
        let words = &mut self.nodes[node as usize].words;
        if let Some(found) = words.iter_mut().find(|item| item.text == text) {
            if score > found.score {
                found.score = score;
            }
            if seq < found.seq {
                found.seq = seq;
            }
        } else {
            words.push(WordAt { text, score, seq });
        }
        if syls.len() == 2 {
            self.bigrams
                .entry((first, text))
                .and_modify(|old| {
                    if score > *old {
                        *old = score;
                    }
                })
                .or_insert(score);
        }
    }

    #[allow(clippy::type_complexity)] // Internal compiler tables, not a public API.
    fn finish(
        mut self,
    ) -> (
        Vec<String>,
        Vec<String>,
        Vec<TrieNode>,
        Vec<TrieEdge>,
        Vec<[u32; 26]>,
        Vec<u16>,
        HashMap<(u16, u32), i64>,
    ) {
        for raw in &mut self.nodes {
            // Same order as a candidate listing: score, subtitle count, length, code point.
            // A lazy merge of these lists then matches a full sort.
            raw.words.sort_by(|a, b| {
                let left = &self.texts[a.text as usize];
                let right = &self.texts[b.text as usize];
                b.score
                    .cmp(&a.score)
                    .then(a.seq.cmp(&b.seq))
                    .then(left.chars().count().cmp(&right.chars().count()))
                    .then(left.cmp(right))
            });
        }
        let mut grouped: Vec<Vec<(u8, u16, u32)>> = vec![Vec::new(); self.nodes.len()];
        for ((parent, syl), child) in self.edges {
            let letter = self.syls[syl as usize].as_bytes()[0];
            grouped[parent as usize].push((letter, syl, child));
        }
        let mut edges = Vec::new();
        let mut ranges = Vec::with_capacity(self.nodes.len());
        for kids in &mut grouped {
            kids.sort_unstable();
            let edge_lo = edges.len() as u32;
            let edge_n = u16::try_from(kids.len()).expect("too many syllable edges");
            for &(letter, syl, node) in kids.iter() {
                edges.push(TrieEdge { node, syl, letter });
            }
            ranges.push((edge_lo, edge_n));
        }
        let nodes = self
            .nodes
            .into_iter()
            .enumerate()
            .map(|(index, raw)| {
                let (edge_lo, edge_n) = ranges[index];
                TrieNode {
                    syl: raw.syl,
                    parent: raw.parent,
                    depth: raw.depth,
                    edge_lo,
                    edge_n,
                    words: raw.words,
                }
            })
            .collect();
        let mut auto = vec![[u32::MAX; 26]];
        let mut auto_term = vec![u16::MAX];
        for (id, syl) in self.syls.iter().enumerate() {
            let mut node = 0u32;
            for &byte in syl.as_bytes() {
                let letter = (byte - b'a') as usize;
                let next = auto[node as usize][letter];
                if next == u32::MAX {
                    let created = auto.len() as u32;
                    auto.push([u32::MAX; 26]);
                    auto_term.push(u16::MAX);
                    auto[node as usize][letter] = created;
                    node = created;
                } else {
                    node = next;
                }
            }
            auto_term[node as usize] = id as u16;
        }
        (
            self.texts,
            self.syls,
            nodes,
            edges,
            auto,
            auto_term,
            self.bigrams,
        )
    }
}

#[allow(clippy::type_complexity)] // Parallel image tables.
fn index_bigrams(
    texts: &[String],
    auto: &[[u32; 26]],
    auto_term: &[u16],
    syl_count: usize,
    notes: HashMap<(u16, u32), i64>,
) -> (Vec<Bigram>, Vec<(u32, u32)>, Vec<Bigram>, Vec<(u32, u32)>) {
    let mut by_syl = vec![Vec::new(); syl_count];
    for ((syl, text), score) in notes {
        by_syl[syl as usize].push(Bigram { text, score });
    }
    for list in &mut by_syl {
        list.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then_with(|| texts[a.text as usize].cmp(&texts[b.text as usize]))
        });
        list.truncate(8);
        for item in list.iter_mut() {
            item.score /= 8;
        }
        list.retain(|item| item.score > 0);
    }

    let mut memo = vec![None; auto.len()];
    fill_bigrams(0, auto, auto_term, &by_syl, texts, &mut memo);

    let mut exact = Vec::new();
    let mut exact_at = Vec::with_capacity(auto.len());
    let mut extend = Vec::new();
    let mut extend_at = Vec::with_capacity(auto.len());
    for node in 0..auto.len() {
        let start = exact.len() as u32;
        let term = auto_term[node];
        if term != u16::MAX {
            exact.extend_from_slice(&by_syl[term as usize]);
        }
        exact_at.push((start, exact.len() as u32));

        let start = extend.len() as u32;
        let mut acc = Vec::new();
        for &child in &auto[node] {
            if child == u32::MAX {
                continue;
            }
            if let Some(list) = &memo[child as usize] {
                acc.extend_from_slice(list);
            }
        }
        dedup_top(&mut acc, texts, 15);
        extend.extend_from_slice(&acc);
        extend_at.push((start, extend.len() as u32));
    }
    (exact, exact_at, extend, extend_at)
}

fn locate_words(nodes: &[TrieNode], texts: usize) -> Vec<u32> {
    let mut at = vec![u32::MAX; texts];
    let mut score = vec![0i64; texts];
    for (index, node) in nodes.iter().enumerate() {
        for word in &node.words {
            let id = word.text as usize;
            if at[id] == u32::MAX || word.score > score[id] {
                at[id] = index as u32;
                score[id] = word.score;
            }
        }
    }
    at
}

fn fill_bigrams(
    node: usize,
    auto: &[[u32; 26]],
    auto_term: &[u16],
    by_syl: &[Vec<Bigram>],
    texts: &[String],
    memo: &mut [Option<Vec<Bigram>>],
) {
    if memo[node].is_some() {
        return;
    }
    let mut acc = Vec::new();
    let term = auto_term[node];
    if term != u16::MAX {
        acc.extend_from_slice(&by_syl[term as usize]);
    }
    for letter in 0..26 {
        let child = auto[node][letter];
        if child == u32::MAX {
            continue;
        }
        let child = child as usize;
        fill_bigrams(child, auto, auto_term, by_syl, texts, memo);
        if let Some(list) = &memo[child] {
            acc.extend_from_slice(list);
        }
    }
    dedup_top(&mut acc, texts, 15);
    memo[node] = Some(acc);
}

fn dedup_top(items: &mut Vec<Bigram>, texts: &[String], cap: usize) {
    if items.len() <= 1 {
        return;
    }
    let mut best: HashMap<u32, i64> = HashMap::with_capacity(items.len());
    for item in items.iter() {
        best.entry(item.text)
            .and_modify(|score| {
                if item.score > *score {
                    *score = item.score;
                }
            })
            .or_insert(item.score);
    }
    items.clear();
    items.extend(best.into_iter().map(|(text, score)| Bigram { text, score }));
    items.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| texts[a.text as usize].cmp(&texts[b.text as usize]))
    });
    if items.len() > cap {
        let cutoff = items[cap - 1].score;
        let mut keep = cap;
        while keep < items.len() && items[keep].score == cutoff {
            keep += 1;
        }
        items.truncate(keep);
    }
}

fn traditional_chars(ts_characters: &str) -> HashSet<char> {
    let mut traditional = HashSet::new();
    for line in ts_characters.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split('\t');
        let Some(src) = parts.next() else { continue };
        let Some(values) = parts.next() else { continue };
        let mut src_chars = src.chars();
        let Some(ch) = src_chars.next() else { continue };
        if src_chars.next().is_some() {
            continue;
        }
        if !values.split_whitespace().any(|value| value == src) {
            traditional.insert(ch);
        }
    }
    traditional
}

fn primary_readings(pinyin: &str) -> HashMap<char, String> {
    let mut map = HashMap::new();
    for line in pinyin.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("U+") else {
            continue;
        };
        let Some((code, rest)) = rest.split_once(':') else {
            continue;
        };
        let Ok(cp) = u32::from_str_radix(code.trim(), 16) else {
            continue;
        };
        let Some(ch) = char::from_u32(cp) else {
            continue;
        };
        let reading = rest
            .split('#')
            .next()
            .unwrap_or("")
            .split(',')
            .next()
            .unwrap_or("")
            .trim();
        let plain = plain_syllable(reading);
        if !plain.is_empty() {
            map.insert(ch, plain);
        }
    }
    map
}

fn plain_syllable(toned: &str) -> String {
    toned
        .chars()
        .filter_map(|ch| {
            Some(match ch {
                'a' | 'ā' | 'á' | 'ǎ' | 'à' => 'a',
                'e' | 'ē' | 'é' | 'ě' | 'è' => 'e',
                'i' | 'ī' | 'í' | 'ǐ' | 'ì' => 'i',
                'o' | 'ō' | 'ó' | 'ǒ' | 'ò' => 'o',
                'u' | 'ū' | 'ú' | 'ǔ' | 'ù' => 'u',
                'ü' | 'ǖ' | 'ǘ' | 'ǚ' | 'ǜ' => 'v',
                'n' | 'ń' | 'ň' | 'ǹ' => 'n',
                'm' | 'ḿ' => 'm',
                'g' => 'g',
                other if other.is_ascii_lowercase() => other,
                _ => return None,
            })
        })
        .collect()
}

/// Lower sorts first. A word the subtitles never counted stays at `u32::MAX`.
fn freq_seq(count: i64) -> u32 {
    let count = u32::try_from(count.max(0)).unwrap_or(u32::MAX);
    u32::MAX - count
}

fn has_vowel(syl: &str) -> bool {
    syl.bytes()
        .any(|b| matches!(b, b'a' | b'e' | b'i' | b'o' | b'u' | b'v'))
}
