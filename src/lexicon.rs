//! Syllable trie and candidate ranking. This module does not see key events.
//!
//! The text dictionaries are compiled into one image by the build script.
//! The running process maps and validates that file before querying it. A node's
//! word list is that reading, best score first.
//! Two-syllable predictions are folded onto the spelling automaton when the
//! image is built, so a short prefix does not search or allocate them again.

use crate::image::{open, BigramRec, EdgeRec, NodeRec, RangeRec, WordRec};
use rustc_hash::FxHashMap;
use std::borrow::Cow;
use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet};
use std::fs::File;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

const BAKED_LEXICON: &str = env!("WAYPINYIN_LEXICON");
/// Characters kept from each syllable when a spelling is built out of
/// single characters. 止 is past the first dozen of `zhi`.
const PHRASE_COLUMN: usize = 24;
const _: () = assert!(PHRASE_COLUMN <= 32);
const PHRASE_LIMIT: usize = 4096;

#[derive(Clone, Copy)]
struct StrTable {
    off: &'static [u32],
    bytes: &'static [u8],
}

impl StrTable {
    #[inline]
    fn get(&self, index: u32) -> &'static str {
        let index = index as usize;
        let start = self.off[index] as usize;
        let end = self.off[index + 1] as usize;
        // Offsets address UTF-8 written by the build script.
        unsafe { std::str::from_utf8_unchecked(&self.bytes[start..end]) }
    }
}

/// Syllable trie. A typed spelling walks this instead of every lexicon entry
/// that shares its first letter.
#[derive(Clone)]
pub struct Lexicon {
    pub(crate) preferences: crate::preferences::InputPreferences,
    model: Option<Arc<crate::language_model::LanguageModel>>,
    ranking_context: Arc<str>,
    context_history: ([u64; 2], [u64; 2]),
    texts: StrTable,
    syls: StrTable,
    nodes: &'static [NodeRec],
    words: &'static [WordRec],
    edges: &'static [EdgeRec],
    /// Syllable spelling automaton. `auto[node][letter]` is the next node, or `u32::MAX`.
    auto: &'static [[u32; 26]],
    auto_term: &'static [u16],
    /// Top bigrams of the syllable this automaton node spells.
    exact: &'static [BigramRec],
    exact_at: &'static [RangeRec],
    /// Top bigrams of syllables that continue past this automaton node.
    extend: &'static [BigramRec],
    extend_at: &'static [RangeRec],
    /// ln(sum of lexicon counts). Partial candidates are ranked by
    /// -log(freq / corpus) per character, the same unified score Google
    /// Pinyin uses once the full-length matches are set aside.
    ln_corpus: f64,
    /// Trie node of each lexicon word's highest-scoring reading.
    /// `u32::MAX` when a text has no node. Bigram hits do not carry the
    /// walk node, and learning still needs that word's syllables.
    word_at: &'static [u32],
}

impl std::fmt::Debug for Lexicon {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Lexicon")
            .field("texts", &(self.texts.off.len().saturating_sub(1)))
            .field("syllables", &(self.syls.off.len().saturating_sub(1)))
            .field("nodes", &self.nodes.len())
            .finish()
    }
}

impl Lexicon {
    #[cfg(test)]
    pub(crate) fn with_test_model(&self) -> Self {
        use crate::language_model::{fingerprint, LanguageModel};
        let body = format!(
            "W\t0\t{}\t{}\t8\nW\t0\t{}\t{}\t8\n",
            LanguageModel::token("去"),
            LanguageModel::token("北京"),
            LanguageModel::token("看"),
            LanguageModel::token("背景")
        );
        let bytes = format!(
            "WAYPINYIN-LM\t1\t42\t{}\n{body}",
            fingerprint(body.as_bytes())
        );
        let mut lex = self.clone();
        lex.model = Some(Arc::new(
            LanguageModel::parse(bytes.as_bytes(), 42).unwrap(),
        ));
        lex
    }
    pub(crate) fn with_context(&self, context: &str) -> Self {
        let mut lex = self.clone();
        lex.ranking_context = Arc::from(context);
        lex.context_history = lex
            .model
            .as_ref()
            .map_or(([0; 2], [0; 2]), |model| model.history(context));
        lex
    }

    pub(crate) fn ranking_context(&self) -> &str {
        &self.ranking_context
    }

    pub(crate) fn context_enabled(&self) -> bool {
        self.preferences.contextual && self.model.is_some()
    }

    fn contextual_score(&self, score: i64, word: &str) -> i64 {
        if !self.context_enabled() || self.ranking_context.is_empty() {
            return score;
        }
        let model = self.model.as_ref().unwrap();
        let (chars, tokens) = self.context_history;
        if self.ranking_context.ends_with(word) {
            return score;
        }
        let extra = (model.bonus(chars, tokens, word) - model.bonus([0; 2], [0; 2], word)).max(0.0);
        let multiplier = (model.weight * extra).exp().min(2.0);
        ((score.max(1) as f64 * multiplier).min(i64::MAX as f64 / 2.0)) as i64
    }

    pub fn model_available(&self) -> bool {
        self.model.is_some()
    }

    /// Shares the mapped dictionary data while selecting independent matching preferences.
    pub fn with_preferences(&self, preferences: crate::preferences::InputPreferences) -> Self {
        let mut lex = self.clone();
        lex.preferences = preferences;
        lex
    }
    pub fn load() -> Self {
        static MAPPED: OnceLock<crate::image::Mapped<'static>> = OnceLock::new();
        static MODEL: OnceLock<Option<Arc<crate::language_model::LanguageModel>>> = OnceLock::new();
        let mapped = MAPPED.get_or_init(|| {
            let bytes = image_bytes();
            let mapped = open(bytes);
            // Validation streams over every table. Only candidate lookup is
            // random; defer disabling readahead until validation has finished.
            advise(bytes, libc::MADV_RANDOM);
            mapped
        });
        Self {
            preferences: Default::default(),
            model: MODEL.get_or_init(|| {
                let path = std::env::var_os("WAYPINYIN_MODEL").map(PathBuf::from)
                    .unwrap_or_else(|| lexicon_path().with_extension("lm"));
                match crate::language_model::LanguageModel::load(&path, crate::language_model::fingerprint(image_bytes())) {
                    Ok(model) => Some(Arc::new(model)),
                    Err(err) => {
                        eprintln!("waypinyin: context model unavailable ({}): {err}; using dictionary ranking", path.display());
                        None
                    }
                }
            }).clone(),
            ranking_context: Arc::from(""),
            context_history: ([0;2],[0;2]),
            texts: StrTable {
                off: mapped.texts_off,
                bytes: mapped.text_bytes,
            },
            syls: StrTable {
                off: mapped.syls_off,
                bytes: mapped.syl_bytes,
            },
            nodes: mapped.nodes,
            words: mapped.words,
            edges: mapped.edges,
            auto: mapped.auto,
            auto_term: mapped.auto_term,
            exact: mapped.exact,
            exact_at: mapped.exact_at,
            extend: mapped.extend,
            extend_at: mapped.extend_at,
            ln_corpus: mapped.ln_corpus,
            word_at: mapped.word_at,
        }
    }

    /// Likely next letters for idle speculation; all letters remain eligible.
    pub(crate) fn preload_letters(&self, raw: &str) -> Vec<u8> {
        let mut letters = Vec::with_capacity(26);
        for start in 0..raw.len() {
            if let Some(node) = self.auto_end(&raw[start..]) {
                for (index, next) in self.auto[node as usize].iter().enumerate() {
                    if *next != u32::MAX {
                        letters.push(b'a' + index as u8);
                    }
                }
                break;
            }
        }
        for c in b"aeioundghszcbpmflrktjqxwyv" {
            if !letters.contains(c) {
                letters.push(*c);
            }
        }
        debug_assert_eq!(letters.len(), 26);
        letters
    }

    #[inline]
    fn text(&self, id: u32) -> &'static str {
        self.texts.get(id)
    }

    #[inline]
    fn syl(&self, id: u16) -> &str {
        self.syls.get(u32::from(id))
    }

    fn node_words(&self, node: u32) -> &[WordRec] {
        let rec = &self.nodes[node as usize];
        let start = rec.word_lo as usize;
        &self.words[start..start + rec.word_n as usize]
    }

    fn edges_for(&self, node: u32, letter: u8) -> &[EdgeRec] {
        let item = &self.nodes[node as usize];
        let start = item.edge_lo as usize;
        let edges = &self.edges[start..start + item.edge_n as usize];
        let lower = edges.partition_point(|edge| edge.letter < letter);
        let upper = lower + edges[lower..].partition_point(|edge| edge.letter == letter);
        &edges[lower..upper]
    }

    fn edges_all(&self, node: u32) -> &[EdgeRec] {
        let item = &self.nodes[node as usize];
        let start = item.edge_lo as usize;
        &self.edges[start..start + item.edge_n as usize]
    }

    pub(crate) fn walk_public(&self, node: u32, syl: &str) -> Option<u32> {
        self.walk(node, syl)
    }

    pub(crate) fn score_of(&self, node: u32, word: &str) -> i64 {
        self.node_words(node)
            .iter()
            .find(|rec| self.text(rec.text) == word)
            .map(|rec| rec.score)
            .unwrap_or(0)
    }

    fn walk(&self, node: u32, syl: &str) -> Option<u32> {
        let letter = *syl.as_bytes().first()?;
        if !letter.is_ascii_lowercase() {
            return None;
        }
        self.edges_for(node, letter)
            .iter()
            .find(|edge| self.syl(edge.syl) == syl)
            .map(|edge| edge.node)
    }

    fn syllable_id(&self, raw: &str) -> Option<u16> {
        let node = self.auto_end(raw)?;
        let term = self.auto_term[node as usize];
        (term != u16::MAX).then_some(term)
    }

    fn walk_id(&self, node: u32, syl: u16) -> Option<u32> {
        let letter = *self.syl(syl).as_bytes().first()?;
        if !letter.is_ascii_lowercase() {
            return None;
        }
        self.edges_for(node, letter)
            .iter()
            .find(|edge| edge.syl == syl)
            .map(|edge| edge.node)
    }

    /// Suffixes of longer words that start with `word`. `北京` yields `大学`
    /// from `北京大学`. The suffix keeps only its own syllables, so learning
    /// it later does not store the whole compound's reading.
    pub(crate) fn extensions(&self, word: &str, reading: &str) -> Vec<(String, String, i64)> {
        if word.is_empty() || !valid_syllables(reading) {
            return Vec::new();
        }
        let mut node = 0u32;
        let mut prefix_len = 0usize;
        for syl in reading.split('\'') {
            node = match self.walk(node, syl) {
                Some(next) => next,
                None => return Vec::new(),
            };
            prefix_len += 1;
        }
        let max_extra = 2;
        let mut best: HashMap<String, (String, i64)> = HashMap::new();
        let mut level: Vec<u32> = self.edges_all(node).iter().map(|edge| edge.node).collect();
        for depth in 1..=max_extra {
            let mut next = Vec::new();
            for current in level {
                self.remember_extension(&mut best, word, prefix_len, current);
                if depth == max_extra {
                    continue;
                }
                let mut kids: Vec<(i64, u32)> = self
                    .edges_all(current)
                    .iter()
                    .map(|edge| {
                        let score = self
                            .node_words(edge.node)
                            .first()
                            .map(|rec| rec.score)
                            .unwrap_or(0);
                        (score, edge.node)
                    })
                    .collect();
                kids.sort_by(|left, right| right.0.cmp(&left.0).then(left.1.cmp(&right.1)));
                kids.truncate(6);
                next.extend(kids.into_iter().map(|(_, child)| child));
            }
            level = next;
        }
        let mut rows: Vec<(String, String, i64, i64)> = Vec::new();
        for (suffix, (reading, compound)) in best {
            let own = self.own_score(&reading, &suffix);
            // Dictionary padding (CEDICT at 2, frost tails at 3) is not a
            // continuation worth offering.
            if own <= 3 {
                continue;
            }
            rows.push((suffix, reading, own, compound));
        }
        // A frequent word hiding in a rare compound is not a continuation.
        // Keep suffixes from compounds in the same order of magnitude as the
        // strongest one, then rank those by the suffix's own frequency.
        if let Some(best_compound) = rows.iter().map(|row| row.3).max() {
            let floor = best_compound / 10;
            if floor > 0 {
                rows.retain(|row| row.3 >= floor);
            }
        }
        rows.sort_by(|left, right| {
            let left_single = left.0.chars().count() == 1;
            let right_single = right.0.chars().count() == 1;
            left_single
                .cmp(&right_single)
                .then(right.2.cmp(&left.2))
                .then(left.0.cmp(&right.0))
        });
        if let Some(best_score) = rows.first().map(|row| row.2) {
            let floor = best_score / 10;
            if floor > 3 {
                rows.retain(|row| row.2 >= floor);
            }
        }
        rows.truncate(8);
        rows.into_iter()
            .map(|(suffix, reading, own, _)| (suffix, reading, own))
            .collect()
    }

    fn own_score(&self, reading: &str, word: &str) -> i64 {
        let mut node = 0u32;
        for syl in reading.split('\'') {
            match self.walk(node, syl) {
                Some(next) => node = next,
                None => return 0,
            }
        }
        self.node_words(node)
            .iter()
            .find(|rec| self.text(rec.text) == word)
            .map(|rec| rec.score)
            .unwrap_or(0)
    }

    fn remember_extension(
        &self,
        best: &mut HashMap<String, (String, i64)>,
        word: &str,
        prefix_len: usize,
        current: u32,
    ) {
        for rec in self.node_words(current).iter().take(8) {
            let text = self.text(rec.text);
            let Some(suffix) = text.strip_prefix(word) else {
                continue;
            };
            if suffix.is_empty() || suffix.chars().count() > 4 {
                continue;
            }
            let syls = path_syls(self, current);
            if syls.len() <= prefix_len {
                continue;
            }
            let suffix_reading = syls[prefix_len..].join("'");
            let replace = best.get(suffix).is_none_or(|(_, score)| rec.score > *score);
            if replace {
                best.insert(suffix.to_string(), (suffix_reading, rec.score));
            }
        }
    }

    /// Node reached by walking `raw`, when every syllable spelling that starts
    /// with it passes through that node. `u` and `v` may stand in for each
    /// other, so `nue` lands on `nve` when `nue` itself is not a syllable.
    fn auto_end(&self, raw: &str) -> Option<u32> {
        self.auto_walk(raw.as_bytes(), 0)
    }

    fn auto_walk(&self, raw: &[u8], node: u32) -> Option<u32> {
        if raw.is_empty() {
            return Some(node);
        }
        let byte = *raw.first()?;
        if !byte.is_ascii_lowercase() {
            return None;
        }
        let mut letters = [byte, 0];
        let mut count = 1usize;
        if matches!(byte, b'u' | b'v') {
            letters[1] = if byte == b'u' { b'v' } else { b'u' };
            count = 2;
        }
        for &letter in &letters[..count] {
            let next = self.auto[node as usize][(letter - b'a') as usize];
            if next == u32::MAX {
                continue;
            }
            if let Some(end) = self.auto_walk(&raw[1..], next) {
                return Some(end);
            }
        }
        None
    }
}

fn image_bytes() -> &'static [u8] {
    static IMAGE: OnceLock<&'static [u8]> = OnceLock::new();
    IMAGE.get_or_init(|| map_file(&lexicon_path()))
}

fn lexicon_path() -> PathBuf {
    if let Ok(path) = std::env::var("WAYPINYIN_LEXICON") {
        let path = PathBuf::from(path);
        return path;
    }
    let installed = PathBuf::from(env!("WAYPINYIN_DATADIR")).join("lexicon.bin");
    if installed.is_file() || !cfg!(debug_assertions) {
        return installed;
    }
    PathBuf::from(BAKED_LEXICON)
}

fn map_file(path: &Path) -> &'static [u8] {
    let file = File::open(path).unwrap_or_else(|err| {
        panic!("open lexicon image {}: {err}", path.display());
    });
    let len = file
        .metadata()
        .ok()
        .and_then(|meta| usize::try_from(meta.len()).ok())
        .unwrap_or(0);
    assert!(
        (128..=isize::MAX as usize).contains(&len),
        "lexicon image {} has an invalid size",
        path.display()
    );
    let ptr = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            len,
            libc::PROT_READ,
            libc::MAP_PRIVATE,
            file.as_raw_fd(),
            0,
        )
    };
    assert!(
        ptr != libc::MAP_FAILED && !ptr.is_null(),
        "map lexicon image {}",
        path.display()
    );
    // The image is fully checked before use; permit kernel readahead during
    // that sequential pass. Advice failure affects performance, not validity.
    let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, len) };
    advise(bytes, libc::MADV_SEQUENTIAL);
    bytes
}

fn advise(bytes: &[u8], advice: i32) {
    // SAFETY: bytes spans the live, page-aligned mmap. madvise does not write
    // mapped bytes for either of the access-pattern hints used here.
    unsafe { libc::madvise(bytes.as_ptr().cast_mut().cast(), bytes.len(), advice) };
}

fn is_ng_extension(typed: &str, syl: &str) -> bool {
    syl.len() == typed.len() + 1 && syl.starts_with(typed) && syl.ends_with('g')
}

/// One way to read the typed letters. The scale divides the word count when
/// a finished syllable was stretched (`ni` → `nian` is 100, `ni` → `nin` is
/// 10). An unfinished tail (`sh` → `shi`) stays at 1.
fn segmentations(input: &str, lex: &Lexicon, allow_split: bool) -> Vec<(Vec<u16>, i64)> {
    fn walk(
        input: &str,
        lex: &Lexicon,
        allow_split: bool,
        path: &mut Vec<u16>,
        out: &mut Vec<(Vec<u16>, i64)>,
        budget: &mut usize,
    ) {
        if out.len() >= 48 || *budget == 0 {
            return;
        }
        *budget -= 1;
        if input.is_empty() {
            out.push((path.clone(), 1));
            return;
        }
        let (mut cont, mut closed, mut longer) = syllable_splits(lex, input);
        cont.sort_by(|&a, &b| {
            let left = lex.syl(a);
            let right = lex.syl(b);
            right.len().cmp(&left.len()).then(left.cmp(right))
        });
        closed.sort_by(|&a, &b| lex.syl(a).cmp(lex.syl(b)));
        longer.sort_by(|&a, &b| {
            let left = lex.syl(a);
            let right = lex.syl(b);
            left.len().cmp(&right.len()).then(left.cmp(right))
        });
        longer.truncate(20);
        let split = allow_split || closed.is_empty();
        if split {
            for syl in &cont {
                let n = lex.syl(*syl).len();
                path.push(*syl);
                walk(&input[n..], lex, allow_split, path, out, budget);
                path.pop();
            }
        }
        for syl in &closed {
            path.push(*syl);
            out.push((path.clone(), 1));
            path.pop();
        }
        if cont.is_empty() || !split {
            let finished = !closed.is_empty();
            for syl in &longer {
                let text = lex.syl(*syl);
                let extra = text.len().saturating_sub(input.len()) as u32;
                let scale = if finished {
                    10i64.saturating_pow(extra.max(1))
                } else {
                    1
                };
                path.push(*syl);
                out.push((path.clone(), scale));
                path.pop();
            }
        }
    }

    let mut out = Vec::new();
    walk(
        input,
        lex,
        allow_split,
        &mut Vec::new(),
        &mut out,
        &mut 8192,
    );
    out
}

fn syl_text(lex: &Lexicon, syls: &[u16]) -> Vec<String> {
    syls.iter().map(|id| lex.syl(*id).to_owned()).collect()
}

/// Syllables that continue `input`, equal it, or extend past it.
fn syllable_splits(lex: &Lexicon, input: &str) -> (Vec<u16>, Vec<u16>, Vec<u16>) {
    let mut cont = Vec::new();
    let mut closed = Vec::new();
    let mut node = 0u32;
    let mut consumed_all = true;
    for (index, &byte) in input.as_bytes().iter().enumerate() {
        if !byte.is_ascii_lowercase() {
            consumed_all = false;
            break;
        }
        let next = lex.auto[node as usize][(byte - b'a') as usize];
        if next == u32::MAX {
            consumed_all = false;
            break;
        }
        node = next;
        let term = lex.auto_term[node as usize];
        if term != u16::MAX {
            if index + 1 < input.len() {
                cont.push(term);
            } else {
                closed.push(term);
            }
        }
    }
    let mut longer = Vec::new();
    if consumed_all {
        let mut stack = vec![(node, 0u16)];
        while let Some((current, extra)) = stack.pop() {
            if extra > 0 {
                let term = lex.auto_term[current as usize];
                if term != u16::MAX {
                    let syl = lex.syl(term);
                    if !is_ng_extension(input, syl) {
                        longer.push(term);
                    }
                }
            }
            for letter in 0..26 {
                let next = lex.auto[current as usize][letter];
                if next != u32::MAX {
                    stack.push((next, extra + 1));
                }
            }
        }
    }
    (cont, closed, longer)
}

fn covers(raw: &str, lex: &Lexicon) -> Vec<(Vec<u16>, i64)> {
    if !raw.contains('\'') {
        return segmentations(raw, lex, true);
    }
    let parts: Vec<&str> = raw.split('\'').filter(|part| !part.is_empty()).collect();
    let mut fixed = Vec::new();
    for (index, part) in parts.iter().enumerate() {
        if index + 1 == parts.len() {
            return segmentations(part, lex, false)
                .into_iter()
                .map(|(mut tail, scale)| {
                    let mut syls = fixed.clone();
                    syls.append(&mut tail);
                    (syls, scale)
                })
                .collect();
        }
        let Some(id) = lex.syllable_id(part) else {
            return Vec::new();
        };
        fixed.push(id);
    }
    Vec::new()
}

/// Resultative complements. The character is the one appended to the verb.
fn complement_char(syl: &str) -> Option<char> {
    Some(match syl {
        "wan" => '完',
        "hao" => '好',
        "diao" => '掉',
        "shang" => '上',
        "xia" => '下',
        "qi" => '起',
        "dao" => '到',
        "cheng" => '成',
        "zhu" => '住',
        "guo" => '过',
        _ => return None,
    })
}

/// `verb + complement` rows for a spelling the lexicon does not store as a
/// word. A one-syllable verb keeps its four most common characters, so
/// `pawan` offers 怕完 and 爬完. A longer verb keeps the word that already
/// covers it, so `xuexiwan` offers 学习完.
fn complement_forms(syls: Option<&[u16]>, lex: &Lexicon) -> Vec<(String, i64, String)> {
    let Some(syls) = syls else {
        return Vec::new();
    };
    if syls.len() < 2 {
        return Vec::new();
    }
    let Some(mark) = complement_char(lex.syl(*syls.last().expect("syllable"))) else {
        return Vec::new();
    };
    let prefix = &syls[..syls.len() - 1];
    // `yi xia` is 一下, not a resultative 下.
    if mark == '下' && prefix.last().is_some_and(|id| lex.syl(*id) == "yi") {
        return Vec::new();
    }
    let reading = syls
        .iter()
        .map(|id| lex.syl(*id))
        .collect::<Vec<_>>()
        .join("'");
    if prefix
        .last()
        .is_some_and(|id| matches!(lex.syl(*id), "de" | "bu"))
    {
        return Vec::new();
    }
    let mut out = Vec::new();
    if prefix.len() == 1 {
        let Some(node) = lex.walk_id(0, prefix[0]) else {
            return Vec::new();
        };
        for word in lex.node_words(node) {
            let text = lex.text(word.text);
            if text.chars().count() != 1 {
                continue;
            }
            let mut compound = text.to_owned();
            compound.push(mark);
            out.push((compound, word.score, reading.clone()));
            if out.len() == 4 {
                break;
            }
        }
    } else if let Some((word, score)) = prefix_head(prefix, lex) {
        let mut compound = word;
        compound.push(mark);
        out.push((compound, score, reading));
    }
    out
}

fn exact_syls(raw: &str, lex: &Lexicon) -> Option<Vec<u16>> {
    covers(raw, lex)
        .into_iter()
        .filter(|(_, scale)| *scale == 1)
        .min_by_key(|(syls, _)| syls.len())
        .map(|(syls, _)| syls)
}

fn joined_reading(syls: &[u16], lex: &Lexicon) -> String {
    syls.iter()
        .map(|id| lex.syl(*id))
        .collect::<Vec<_>>()
        .join("'")
}

fn degree_word(syls: &[u16], lex: &Lexicon) -> Option<(usize, String)> {
    if syls.len() >= 2 {
        let pair = match (lex.syl(syls[0]), lex.syl(syls[1])) {
            ("fei", "chang") => Some("非常"),
            ("te", "bie") => Some("特别"),
            ("bi", "jiao") => Some("比较"),
            ("you", "dian") => Some("有点"),
            ("zhe", "me") => Some("这么"),
            ("na", "me") => Some("那么"),
            ("duo", "me") => Some("多么"),
            ("shi", "fen") => Some("十分"),
            _ => None,
        };
        if let Some(word) = pair {
            return Some((2, word.to_string()));
        }
    }
    let mark = match lex.syl(syls[0]) {
        "hao" => "好",
        "hen" => "很",
        "tai" => "太",
        "zhen" => "真",
        "ting" => "挺",
        "geng" => "更",
        "zui" => "最",
        "chao" => "超",
        "zei" => "贼",
        "ke" => "可",
        "te" => "特",
        "man" => "蛮",
        "lao" => "老",
        "guai" => "怪",
        _ => return None,
    };
    Some((1, mark.to_string()))
}

/// Degree word plus a word the lexicon already has. `haojianlou` is 好简陋,
/// `henjianlou` is 很简陋, `feichangjianlou` is 非常简陋. A stored phrase
/// such as 好看 is left alone by the caller.
fn degree_forms(syls: Option<&[u16]>, lex: &Lexicon) -> Vec<(String, i64, String)> {
    let Some(syls) = syls else {
        return Vec::new();
    };
    let Some((taken, mark)) = degree_word(syls, lex) else {
        return Vec::new();
    };
    if syls.len() <= taken {
        return Vec::new();
    }
    let tail = &syls[taken..];
    let reading = joined_reading(syls, lex);
    if tail.len() == 1 {
        return single_heads(lex, tail[0])
            .into_iter()
            .take(2)
            .map(|(word, score)| {
                let mut compound = mark.clone();
                compound.push_str(&word);
                (compound, score, reading.clone())
            })
            .collect();
    }
    let Some((word, score)) = prefix_head(tail, lex) else {
        return Vec::new();
    };
    if word.chars().count() < 2 {
        return Vec::new();
    }
    let mut compound = mark;
    compound.push_str(&word);
    vec![(compound, score, reading)]
}

fn directional_word(syls: &[u16], lex: &Lexicon) -> Option<(usize, &'static str)> {
    if syls.len() >= 2 {
        let n = syls.len();
        let pair = match (lex.syl(syls[n - 2]), lex.syl(syls[n - 1])) {
            ("shang", "qu") => Some((2, "上去")),
            ("xia", "qu") => Some((2, "下去")),
            ("shang", "lai") => Some((2, "上来")),
            ("xia", "lai") => Some((2, "下来")),
            ("chu", "lai") => Some((2, "出来")),
            ("chu", "qu") => Some((2, "出去")),
            ("jin", "qu") => Some((2, "进去")),
            ("jin", "lai") => Some((2, "进来")),
            ("qi", "lai") => Some((2, "起来")),
            ("guo", "lai") => Some((2, "过来")),
            ("guo", "qu") => Some((2, "过去")),
            ("hui", "lai") => Some((2, "回来")),
            ("hui", "qu") => Some((2, "回去")),
            _ => None,
        };
        if pair.is_some() {
            return pair;
        }
    }
    Some(match lex.syl(*syls.last()?) {
        "shang" => (1, "上"),
        "xia" => (1, "下"),
        "lai" => (1, "来"),
        "qu" => (1, "去"),
        _ => return None,
    })
}

/// `爬上去` / `爬来` from a verb plus a directional the lexicon never stored.
fn directional_forms(syls: Option<&[u16]>, lex: &Lexicon) -> Vec<(String, i64, String)> {
    let Some(syls) = syls else {
        return Vec::new();
    };
    let Some((taken, mark)) = directional_word(syls, lex) else {
        return Vec::new();
    };
    if syls.len() <= taken {
        return Vec::new();
    }
    let verb = &syls[..syls.len() - taken];
    if verb
        .last()
        .is_some_and(|id| matches!(lex.syl(*id), "de" | "bu"))
    {
        return Vec::new();
    }
    if mark == "下" && verb.last().is_some_and(|id| lex.syl(*id) == "yi") {
        return Vec::new();
    }
    glue_head(lex, verb, mark, &joined_reading(syls, lex))
}

/// `爬得上` / `爬不上`. The syllable before the complement is 得 or 不.
fn potential_forms(syls: Option<&[u16]>, lex: &Lexicon) -> Vec<(String, i64, String)> {
    let Some(syls) = syls else {
        return Vec::new();
    };
    if syls.len() < 3 {
        return Vec::new();
    }
    let mut mid = None;
    for (index, id) in syls.iter().enumerate() {
        if index == 0 || index + 1 == syls.len() {
            continue;
        }
        if matches!(lex.syl(*id), "de" | "bu") {
            if mid.is_some() {
                return Vec::new();
            }
            mid = Some(index);
        }
    }
    let Some(mid) = mid else {
        return Vec::new();
    };
    let particle = if lex.syl(syls[mid]) == "de" {
        "得"
    } else {
        "不"
    };
    let tail = &syls[mid + 1..];
    let Some(tail_mark) = tail_text(tail, lex) else {
        return Vec::new();
    };
    let mut mark = String::from(particle);
    mark.push_str(&tail_mark);
    glue_head(lex, &syls[..mid], &mark, &joined_reading(syls, lex))
}

/// `简陋了` / `简陋着` / `简陋过` when that spelling is not already a word.
fn aspect_forms(syls: Option<&[u16]>, lex: &Lexicon) -> Vec<(String, i64, String)> {
    let Some(syls) = syls else {
        return Vec::new();
    };
    if syls.len() < 2 {
        return Vec::new();
    }
    let mark = match lex.syl(*syls.last().expect("syllable")) {
        "le" => "了",
        "zhe" => "着",
        "guo" => "过",
        _ => return Vec::new(),
    };
    let verb = &syls[..syls.len() - 1];
    if verb
        .last()
        .is_some_and(|id| matches!(lex.syl(*id), "de" | "bu"))
    {
        return Vec::new();
    }
    glue_head(lex, verb, mark, &joined_reading(syls, lex))
}

/// `简陋的` / `慢慢地`. A medial 得 stays with the potential complement.
fn particle_forms(syls: Option<&[u16]>, lex: &Lexicon) -> Vec<(String, i64, String)> {
    let Some(syls) = syls else {
        return Vec::new();
    };
    if syls.len() < 2 {
        return Vec::new();
    }
    let mark = match lex.syl(*syls.last().expect("syllable")) {
        "de" => "的",
        "di" => "地",
        _ => return Vec::new(),
    };
    glue_head(
        lex,
        &syls[..syls.len() - 1],
        mark,
        &joined_reading(syls, lex),
    )
}

/// `爬一下`. The two syllables are the word 一下, not a separate 一 and 下.
fn yixia_forms(syls: Option<&[u16]>, lex: &Lexicon) -> Vec<(String, i64, String)> {
    let Some(syls) = syls else {
        return Vec::new();
    };
    if syls.len() < 3 {
        return Vec::new();
    }
    let n = syls.len();
    if lex.syl(syls[n - 2]) != "yi" || lex.syl(syls[n - 1]) != "xia" {
        return Vec::new();
    }
    glue_head(lex, &syls[..n - 2], "一下", &joined_reading(syls, lex))
}

/// `爬爬` and `爬一爬` when that spelling is not already a lexicon word.
fn redup_forms(syls: Option<&[u16]>, lex: &Lexicon) -> Vec<(String, i64, String)> {
    let Some(syls) = syls else {
        return Vec::new();
    };
    if syls.len() == 2 && syls[0] == syls[1] {
        let reading = joined_reading(syls, lex);
        return single_heads(lex, syls[0])
            .into_iter()
            .take(2)
            .map(|(word, score)| {
                let mut compound = word.clone();
                compound.push_str(&word);
                (compound, score, reading.clone())
            })
            .collect();
    }
    if syls.len() == 3 && syls[0] == syls[2] && lex.syl(syls[1]) == "yi" {
        let reading = joined_reading(syls, lex);
        return single_heads(lex, syls[0])
            .into_iter()
            .take(2)
            .map(|(word, score)| (format!("{word}一{word}"), score, reading.clone()))
            .collect();
    }
    Vec::new()
}

/// `没爬` / `不` plus a word already in the lexicon. A stored word such as
/// 不是 is left alone by the caller.
fn negation_forms(syls: Option<&[u16]>, lex: &Lexicon) -> Vec<(String, i64, String)> {
    let Some(syls) = syls else {
        return Vec::new();
    };
    if syls.len() < 2 {
        return Vec::new();
    }
    let mark = match lex.syl(syls[0]) {
        "bu" => "不",
        "mei" => "没",
        _ => return Vec::new(),
    };
    let reading = joined_reading(syls, lex);
    let tail = &syls[1..];
    if tail.len() == 1 {
        return single_heads(lex, tail[0])
            .into_iter()
            .take(2)
            .map(|(word, score)| (format!("{mark}{word}"), score, reading.clone()))
            .collect();
    }
    let Some((word, score)) = prefix_head(tail, lex) else {
        return Vec::new();
    };
    vec![(format!("{mark}{word}"), score, reading)]
}

/// `简陋吗`. The particle is only the last syllable.
fn mood_forms(syls: Option<&[u16]>, lex: &Lexicon) -> Vec<(String, i64, String)> {
    let Some(syls) = syls else {
        return Vec::new();
    };
    if syls.len() < 2 {
        return Vec::new();
    }
    let mark = match lex.syl(*syls.last().expect("syllable")) {
        "ma" => "吗",
        "ne" => "呢",
        "ba" => "吧",
        "a" => "啊",
        _ => return Vec::new(),
    };
    glue_head(
        lex,
        &syls[..syls.len() - 1],
        mark,
        &joined_reading(syls, lex),
    )
}

fn glue_head(lex: &Lexicon, head: &[u16], mark: &str, reading: &str) -> Vec<(String, i64, String)> {
    if head.is_empty() {
        return Vec::new();
    }
    if head.len() == 1 {
        return single_heads(lex, head[0])
            .into_iter()
            .take(2)
            .map(|(word, score)| {
                let mut compound = word;
                compound.push_str(mark);
                (compound, score, reading.to_string())
            })
            .collect();
    }
    let Some((word, score)) = prefix_head(head, lex) else {
        return Vec::new();
    };
    if word.chars().count() < 2 {
        return Vec::new();
    }
    let mut compound = word;
    compound.push_str(mark);
    vec![(compound, score, reading.to_string())]
}

fn potential_mark(syl: &str) -> Option<&'static str> {
    Some(match syl {
        "shang" => "上",
        "xia" => "下",
        "lai" => "来",
        "qu" => "去",
        "dao" => "到",
        "liao" => "了",
        "wan" => "完",
        "dong" => "动",
        "qi" => "起",
        "guo" => "过",
        "zhu" => "住",
        "jian" => "见",
        "chu" => "出",
        "jin" => "进",
        "kai" => "开",
        "cheng" => "成",
        "diao" => "掉",
        "zhao" => "着",
        "hui" => "回",
        _ => return None,
    })
}

fn tail_text(tail: &[u16], lex: &Lexicon) -> Option<String> {
    if let Some((taken, mark)) = directional_word(tail, lex) {
        if taken == tail.len() && taken == 2 {
            return Some(mark.to_string());
        }
    }
    if tail.len() != 1 {
        return None;
    }
    potential_mark(lex.syl(tail[0])).map(str::to_string)
}

fn single_heads(lex: &Lexicon, syl: u16) -> Vec<(String, i64)> {
    let Some(node) = lex.walk_id(0, syl) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for word in lex.node_words(node) {
        let text = lex.text(word.text);
        if text.chars().count() != 1 {
            continue;
        }
        out.push((text.to_owned(), word.score));
        if out.len() == 4 {
            break;
        }
    }
    out
}

/// Put at most two complement rows on the first page, keeping the best
/// lexicon hit and the next one that the prefix insertion would otherwise
/// crowd off that page.
fn place_complements(
    ranked: &mut Vec<Ranked>,
    forms: &[(String, i64, String)],
    readings: &mut HashMap<String, String>,
) {
    let mut added = Vec::new();
    for (word, score, reading) in forms {
        if ranked.iter().any(|item| item.word == *word) {
            continue;
        }
        added.push(Ranked {
            word: word.clone(),
            tier: 0,
            score: (*score).max(1),
            node: None,
            read_at: None,
        });
        readings.insert(word.clone(), reading.clone());
        if added.len() == 2 {
            break;
        }
    }
    if added.is_empty() {
        return;
    }
    if ranked.is_empty() {
        ranked.append(&mut added);
        return;
    }
    let mut tail = ranked.split_off(1);
    let runner = if tail.len() > 1 {
        Some(tail.remove(1))
    } else {
        None
    };
    ranked.append(&mut added);
    if let Some(runner) = runner {
        ranked.push(runner);
    }
    ranked.append(&mut tail);
}

/// Best word covering every syllable of `syls`, or a longest-match glue.
fn prefix_head(syls: &[u16], lex: &Lexicon) -> Option<(String, i64)> {
    if syls.is_empty() {
        return None;
    }
    let mut node = 0u32;
    let mut walked = true;
    for syl in syls {
        match lex.walk_id(node, *syl) {
            Some(next) => node = next,
            None => {
                walked = false;
                break;
            }
        }
    }
    if walked {
        if let Some(best) = lex.node_words(node).first() {
            return Some((lex.text(best.text).to_owned(), best.score));
        }
    }
    // Grammar synthesis operates on a short head, not an entire clause.
    // Cropping the last syllable of a sentence before greedy composition
    // turns e.g. 可以完成 into 可以玩 + 成 and suppresses sentence decoding.
    // Long heads remain eligible when they are actual dictionary words above.
    if syls.len() > 4 {
        return None;
    }
    let sentence = compose(syls, lex)?;
    let head = lex.walk_id(0, syls[0])?;
    let score = lex
        .node_words(head)
        .first()
        .map(|word| word.score)
        .unwrap_or(1);
    Some((sentence, score))
}

/// Frequency composition. Each span keeps its highest-scoring word, and the
/// segmentation with the larger sum of `ilog2(score)` wins. An equal sum
/// keeps fewer pieces, so one stored word still beats two pieces that add
/// up to the same weight. Abbreviation stays on [`compose_mixed`].
fn compose(syls: &[u16], lex: &Lexicon) -> Option<String> {
    if syls.len() < 2 {
        return None;
    }
    let n = syls.len();
    let mut costs = vec![f64::INFINITY; n + 1];
    costs[n] = 0.0;
    let mut pieces = vec![0u16; n + 1];
    let mut jump = vec![0u16; n + 1];
    let mut reach = vec![false; n + 1];
    reach[n] = true;
    for index in (0..n).rev() {
        let mut node = 0u32;
        for len in 1..=n - index {
            let Some(next) = lex.walk_id(node, syls[index + len - 1]) else {
                break;
            };
            node = next;
            if !reach[index + len] {
                continue;
            }
            let Some(word) = lex.node_words(node).first() else {
                continue;
            };
            // Each token must pay its corpus normalization cost. Summing
            // positive log counts rewarded splitting common phrases into chars.
            let total =
                (lex.ln_corpus - (word.score.max(1) as f64).ln()).max(0.0) + costs[index + len];
            let count = pieces[index + len] + 1;
            let better = !reach[index]
                || total < costs[index]
                || (total == costs[index] && count < pieces[index]);
            if better {
                reach[index] = true;
                costs[index] = total;
                pieces[index] = count;
                jump[index] = len as u16;
            }
        }
    }
    if !reach[0] || pieces[0] < 2 {
        return None;
    }
    let mut parts = Vec::new();
    let mut index = 0;
    while index < n {
        let len = jump[index] as usize;
        if len == 0 || index + len > n {
            return None;
        }
        let mut node = 0u32;
        for syl in &syls[index..index + len] {
            node = lex.walk_id(node, *syl)?;
        }
        let word = lex.node_words(node).first()?;
        parts.push(lex.text(word.text));
        index += len;
    }
    Some(parts.concat())
}

/// A small conversational character-pair prior, applied only during sentence
/// composition; stored exact words retain their corpus ranking.
fn sentence_pairs() -> &'static FxHashMap<u64, f64> {
    static PAIRS: OnceLock<FxHashMap<u64, f64>> = OnceLock::new();
    PAIRS.get_or_init(|| {
        let mut pairs = FxHashMap::default();
        for line in include_str!("../data/sentences.txt")
            .lines()
            .filter(|l| !l.starts_with('#'))
        {
            let chars: Vec<_> = line.chars().collect();
            for pair in chars.windows(2) {
                *pairs.entry(pair_key(pair[0], pair[1])).or_insert(0u32) += 1;
            }
        }
        pairs
            .into_iter()
            .map(|(key, count)| (key, 2.0 * f64::from(count).ln_1p().min(3.0)))
            .collect()
    })
}

fn sentence_bonus(previous: Option<char>, word: &str) -> f64 {
    let pairs = sentence_pairs();
    let mut prev = previous;
    let mut bonus = 0.0;
    for ch in word.chars() {
        if let Some(p) = prev {
            if let Some(weight) = pairs.get(&pair_key(p, ch)) {
                bonus += weight;
            } else {
                bonus -= 1.5;
            }
        }
        prev = Some(ch);
    }
    bonus
}

fn pair_key(previous: char, next: char) -> u64 {
    (u64::from(previous) << 32) | u64::from(next)
}

#[derive(Clone)]
struct SentencePath {
    cost: f64,
    text: String,
    reading: Vec<u16>,
    words: usize,
    last: Option<char>,
    chars: [u64; 2],
    tokens: [u64; 2],
    edited: bool,
}

fn compose_sentences(
    input: &str,
    lex: &Lexicon,
    fully_spelled: bool,
) -> Vec<(String, Vec<String>, f64)> {
    // Short word prefixes already have dictionary correction and completion.
    // Do not spend a sentence-model search budget on an unfinished word.
    if lex.context_enabled() && lex.ranking_context.is_empty() && input.len() < 8 {
        let mut fallback = lex.clone();
        fallback.preferences.contextual = false;
        return compose_sentence_search(input, &fallback, fully_spelled, false);
    }
    if lex.context_enabled() && lex.ranking_context.is_empty() {
        let mut fallback = lex.clone();
        fallback.preferences.contextual = false;
        let anchored = compose_sentence_search(input, &fallback, fully_spelled, false);
        if let Some((text, _, _)) = anchored.first() {
            let chars: Vec<_> = text.chars().collect();
            let known = chars
                .windows(2)
                .filter(|p| sentence_pairs().contains_key(&pair_key(p[0], p[1])))
                .count();
            // Keep a well-supported existing reading stable. Sparse-model
            // extrapolation is useful where the conversational prior is weak,
            // not where almost every boundary is already supported.
            if chars.len() >= 3 && known * 4 >= (chars.len() - 1) * 3 {
                return anchored;
            }
        }
    }
    let ordinary = compose_sentence_search(input, lex, fully_spelled, false);
    if ordinary.is_empty() && lex.context_enabled() && input.len() >= 16 {
        compose_sentence_search(input, lex, fully_spelled, true)
    } else {
        ordinary
    }
}

fn compose_sentence_search(
    input: &str,
    lex: &Lexicon,
    fully_spelled: bool,
    repair: bool,
) -> Vec<(String, Vec<String>, f64)> {
    if input.len() < 4
        || input.len() > 128
        || !input.bytes().all(|c| c.is_ascii_lowercase() || c == b'\'')
    {
        return Vec::new();
    }
    let n = input.len();
    let model = lex.model.as_deref().filter(|_| lex.preferences.contextual);
    let beam_width = if model.is_some() { 4 } else { 8 };
    let target_limit = beam_width * 2;
    let (context_chars, context_tokens) = if model.is_some() {
        lex.context_history
    } else {
        ([0; 2], [0; 2])
    };
    let mut paths: Vec<Vec<SentencePath>> = vec![Vec::new(); n + 1];
    // For a full destination beam, keep the original max_by tie winner. Most
    // proposed paths can then be rejected before comparing any strings.
    let mut worst = vec![0usize; n + 1];
    paths[0].push(SentencePath {
        cost: 0.0,
        text: String::new(),
        reading: Vec::new(),
        words: 0,
        last: (context_chars[1] != 0)
            .then(|| char::from_u32(context_chars[1] as u32))
            .flatten(),
        chars: context_chars,
        tokens: context_tokens,
        edited: false,
    });
    // Bound memoization independently of the search budget. A full cache only
    // stops inserting: it never prunes a path or changes score arithmetic.
    // Only packed dictionary IDs/characters enter these numeric maps. The
    // fixed traversal/cache caps remain in force; user-text maps keep SipHash.
    let mut bonuses: FxHashMap<u64, f64> = FxHashMap::default();
    let mut model_bonuses: FxHashMap<[u64; 5], f64> = FxHashMap::default();
    let mut features: FxHashMap<u32, crate::language_model::WordFeatures> = FxHashMap::default();
    // Reuse traversal storage across word boundaries. Clearing retains capacity
    // without changing the stack order or any visited-state decisions.
    let mut stack = Vec::new();
    let mut seen: FxHashMap<u64, f64> = FxHashMap::default();
    let mut budget = 60_000usize;
    for start in 0..n {
        if budget == 0 {
            break;
        }
        if paths[start].is_empty() {
            continue;
        }
        paths[start].sort_by(|a, b| a.cost.total_cmp(&b.cost).then(a.text.cmp(&b.text)));
        paths[start].truncate(beam_width);
        let parents = std::mem::take(&mut paths[start]);
        let begin = skip_sep(input, start);
        if begin == n {
            continue;
        }
        stack.clear();
        stack.push((0u32, begin, 0.0, false));
        seen.clear();
        let mut visited = 0;
        while let Some((node, pos, penalty, edited)) = stack.pop() {
            let state_key = (u64::from(node) << 32) | ((pos as u64) << 1) | u64::from(edited);
            match seen.entry(state_key) {
                std::collections::hash_map::Entry::Occupied(mut entry) => {
                    if *entry.get() <= penalty {
                        continue;
                    }
                    *entry.get_mut() = penalty;
                }
                std::collections::hash_map::Entry::Vacant(entry) => {
                    entry.insert(penalty);
                }
            }
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
                let mut node_reading = None;
                let words = lex.node_words(node);
                let mut model_bonus = |parent: &SentencePath, index: usize| {
                    let Some(model) = model else {
                        return 0.0;
                    };
                    let word = &words[index];
                    let key = [
                        parent.chars[0],
                        parent.chars[1],
                        parent.tokens[0],
                        parent.tokens[1],
                        u64::from(word.text),
                    ];
                    if let Some(value) = model_bonuses.get(&key) {
                        return *value;
                    }
                    let feature = if let Some(feature) = features.get(&word.text) {
                        *feature
                    } else {
                        let feature = model.features(lex.text(word.text));
                        if features.len() < 4096 {
                            features.insert(word.text, feature);
                        }
                        feature
                    };
                    let value =
                        model.weight * model.bonus_features(parent.chars, parent.tokens, feature);
                    if model_bonuses.len() < 4096 {
                        model_bonuses.insert(key, value);
                    }
                    value
                };
                // Compare 32 homophones against the leading context, retaining
                // four frequency leaders plus two context leaders. All parent
                // histories still score these surviving words independently.
                let mut selected: Vec<usize> = (0..words.len().min(4)).collect();
                if let Some(selected_model) = model.filter(|_| words.len() > 4) {
                    for parent in parents.iter().take(1) {
                        let mut ranked: Vec<_> = words
                            .iter()
                            .take(32)
                            .enumerate()
                            .map(|(i, w)| {
                                let original =
                                    (lex.ln_corpus - (w.score.max(1) as f64).ln()).max(0.0);
                                let cost = selected_model.word_cost(original, lex.text(w.text))
                                    - model_bonus(parent, i);
                                (i, cost)
                            })
                            .collect();
                        ranked.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
                        for (index, _) in ranked.into_iter().take(2) {
                            if !selected.contains(&index) {
                                selected.push(index);
                            }
                        }
                    }
                }
                for index in selected {
                    let word = &words[index];
                    let text = lex.text(word.text);
                    let dictionary_cost =
                        (lex.ln_corpus - (word.score.max(1) as f64).ln()).max(0.0);
                    let base = if let Some(model) = model {
                        model.word_cost(dictionary_cost, text)
                    } else {
                        dictionary_cost
                    } + penalty;
                    let last = text.chars().last();
                    let token = crate::language_model::LanguageModel::token(text);
                    for parent in &parents {
                        if parent.edited && edited {
                            continue;
                        }
                        let key = (u64::from(parent.last.map_or(u32::MAX, u32::from)) << 32)
                            | u64::from(word.text);
                        let bonus = if let Some(bonus) = bonuses.get(&key) {
                            *bonus
                        } else {
                            let bonus = sentence_bonus(parent.last, text);
                            if bonuses.len() < 4096 {
                                bonuses.insert(key, bonus);
                            }
                            bonus
                        };
                        let contextual = model_bonus(parent, index);
                        let cost = parent.cost + base
                            - bonus * model.map_or(1.0, |m| m.prior_weight)
                            - contextual;
                        let target = &mut paths[end];
                        // A duplicate in a full beam cannot cost more than its
                        // worst path. This rejection is also safe before dedup.
                        if target.len() == target_limit && cost >= target[worst[end]].cost {
                            continue;
                        }
                        let mut next_chars = parent.chars;
                        for ch in text.chars() {
                            next_chars = [next_chars[1], ch as u64];
                        }
                        let next_tokens = [parent.tokens[1], token];
                        // Before the final boundary, paths with the same LM
                        // history have identical future costs. Keep only the
                        // cheapest state instead of filling the beam with
                        // alternate prefixes that can never overtake it.
                        // Final alternatives retain their distinct visible text.
                        if model.is_some() && end < n {
                            let same_state = |p: &SentencePath| {
                                p.chars == next_chars
                                    && p.tokens == next_tokens
                                    && p.edited == (parent.edited || edited)
                                    && p.words.min(2) == (parent.words + 1).min(2)
                            };
                            let mut count = 0;
                            let mut worst_state: Option<(usize, f64)> = None;
                            for (i, p) in target.iter().enumerate().filter(|(_, p)| same_state(p)) {
                                count += 1;
                                if worst_state.is_none_or(|(_, cost)| p.cost >= cost) {
                                    worst_state = Some((i, p.cost));
                                }
                            }
                            if count >= 2 {
                                let (i, worst_cost) = worst_state.unwrap();
                                if worst_cost <= cost {
                                    continue;
                                }
                                target.swap_remove(i);
                            }
                        }
                        // Compare concatenations before allocating the new path.
                        let same_text = |p: &SentencePath| {
                            (model.is_none() || end == n)
                                && p.edited == (parent.edited || edited)
                                && p.text.len() == parent.text.len() + text.len()
                                && p.text.starts_with(&parent.text)
                                && &p.text[parent.text.len()..] == text
                        };
                        if let Some(index) = target.iter().position(same_text) {
                            if target[index].cost <= cost {
                                continue;
                            }
                            target.swap_remove(index);
                        }
                        if target.len() == target_limit {
                            target.swap_remove(worst[end]);
                        }
                        // Keep the original replacement and tie-breaking order,
                        // but copy text/readings only for paths that survive it.
                        let mut next = parent.clone();
                        next.cost = cost;
                        next.text.push_str(text);
                        next.last = last.or(parent.last);
                        next.chars = next_chars;
                        next.tokens = next_tokens;
                        next.reading.extend_from_slice(
                            node_reading.get_or_insert_with(|| path_ids(lex, node)),
                        );
                        next.words += 1;
                        next.edited |= edited;
                        target.push(next);
                        if target.len() == target_limit {
                            worst[end] = target
                                .iter()
                                .enumerate()
                                .max_by(|(_, a), (_, b)| a.cost.total_cmp(&b.cost))
                                .unwrap()
                                .0;
                        }
                    }
                }
            }
            if pos >= n {
                continue;
            }
            let rest = &input[pos..];
            let edges = if repair {
                lex.edges_all(node)
            } else {
                lex.edges_for(node, input.as_bytes()[pos])
            };
            for edge in edges {
                let syl = lex.syl(edge.syl);
                if repair {
                    for len in 1..=rest.len().min(syl.len() + 1) {
                        let typed = &rest[..len];
                        if !typed.bytes().all(|b| b.is_ascii_lowercase()) {
                            break;
                        }
                        if matches!(piece_match(lex.preferences, typed, syl), Some(Kind::Fuzzy)) {
                            stack.push((
                                edge.node,
                                skip_sep(input, pos + len),
                                penalty + 4.0,
                                edited,
                            ));
                        }
                    }
                    if !edited {
                        if let Some(len) = one_edit_full(rest, syl) {
                            if len > 0 && rest[..len].bytes().all(|b| b.is_ascii_lowercase()) {
                                stack.push((
                                    edge.node,
                                    skip_sep(input, pos + len),
                                    penalty + 8.0,
                                    true,
                                ));
                            }
                        }
                    }
                }
                let mut lengths = [(0usize, 0.0); 5];
                let mut count = 0;
                if syl.as_bytes().get(1) != rest.as_bytes().get(1)
                    && rest
                        .as_bytes()
                        .get(1)
                        .is_some_and(|b| matches!(b, b'u' | b'v'))
                {
                    if let Some(alias) = umlaut_alias(syl) {
                        if rest.starts_with(&alias) {
                            lengths[count] = (alias.len(), 0.0);
                            count += 1;
                        }
                    }
                }
                if rest.starts_with(syl) {
                    lengths[count] = (syl.len(), 0.0);
                    count += 1;
                }
                if syl.len() > rest.len() && syl.starts_with(rest) {
                    // Complete only the final, open syllable. Stretching an
                    // already complete spelling costs tenfold frequency per
                    // added letter, as in dictionary prefix matching.
                    let tail_scale = if lex.syllable_id(rest).is_some() {
                        std::f64::consts::LN_10
                    } else {
                        0.25
                    };
                    lengths[count] = (rest.len(), tail_scale * (syl.len() - rest.len()) as f64);
                    count += 1;
                }
                if !fully_spelled && !matches!(syl.as_bytes()[0], b'a' | b'e' | b'o') {
                    lengths[count] = (1, 0.25 * (syl.len().saturating_sub(1)) as f64);
                    count += 1;
                    if (syl.starts_with("zh") || syl.starts_with("ch") || syl.starts_with("sh"))
                        && rest.starts_with(&syl[..2])
                    {
                        lengths[count] = (2, 0.25 * (syl.len().saturating_sub(2)) as f64);
                        count += 1;
                    }
                }
                for &(len, extra) in &lengths[..count] {
                    if pos + len <= n {
                        stack.push((
                            edge.node,
                            skip_sep(input, pos + len),
                            penalty + extra,
                            edited,
                        ));
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
        .take(if model.is_some() { 5 } else { 3 })
        .map(|p| (p.text.clone(), syl_text(lex, &p.reading), p.cost))
        .collect()
}

fn path_ids(lex: &Lexicon, node: u32) -> Vec<u16> {
    let mut ids = Vec::new();
    let mut current = node;
    while current != 0 {
        ids.push(lex.nodes[current as usize].syl);
        current = lex.nodes[current as usize].parent;
    }
    ids.reverse();
    ids
}

fn path_syls(lex: &Lexicon, node: u32) -> Vec<String> {
    syl_text(lex, &path_ids(lex, node))
}

fn reading_path(lex: &Lexicon, node: u32) -> String {
    let mut ids = Vec::new();
    let mut current = node;
    while current != 0 {
        ids.push(lex.nodes[current as usize].syl);
        current = lex.nodes[current as usize].parent;
    }
    let mut out = String::new();
    for (index, id) in ids.iter().rev().enumerate() {
        if index > 0 {
            out.push('\'');
        }
        out.push_str(lex.syl(*id));
    }
    out
}

struct Ranked {
    word: String,
    tier: u8,
    score: i64,
    node: Option<u32>,
    read_at: Option<u32>,
}

#[derive(Clone, Debug)]
pub(crate) struct Choice {
    pub(crate) word: String,
    /// Bytes of the typed spelling this word uses. A shorter span leaves the
    /// tail in the preedit, which is how Gboard commits a partial candidate.
    pub(crate) span: usize,
    /// Syllables joined by `'`, such as `ni'hao`. Empty for dates and raw letters.
    pub(crate) reading: String,
    pub(crate) tier: u8,
    pub(crate) score: i64,
    /// Decayed user weight. Zero leaves this row in corpus order.
    pub(crate) user_weight: u32,
}

/// How a stored reading covers the typed spelling.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ReadingMatch {
    pub(crate) span: usize,
    pub(crate) tier: u8,
}

#[derive(Clone, Debug)]
pub(crate) struct Listing {
    pub(crate) full: Vec<Choice>,
    pub(crate) partials: Vec<Choice>,
}

pub fn suggest(raw: &str, lex: &Lexicon) -> Vec<String> {
    choices(raw, lex)
        .into_iter()
        .map(|choice| choice.word)
        .collect()
}

/// Full-length readings first, then the words that cover only a prefix of
/// that reading. Prefixes follow exact matches and precede fuzzy matches.
pub(crate) fn choices(raw: &str, lex: &Lexicon) -> Vec<Choice> {
    let planned = plan(raw, lex);
    let mut listed = planned.realize(lex);
    attach_partials(&mut listed.full, listed.partials);
    let seen: HashSet<String> = listed
        .full
        .iter()
        .map(|choice| choice.word.clone())
        .collect();
    for (word, score, reading) in assemble_phrases(&planned.phrase, lex, PHRASE_LIMIT) {
        if seen.contains(&word) {
            continue;
        }
        listed.full.push(Choice {
            word,
            span: raw.len(),
            reading,
            tier: 0,
            score,
            user_weight: 0,
        });
    }
    listed.full
}

/// Sources for one spelling. Words are merged from the nodes on demand.
pub(crate) struct Plan {
    raw: String,
    nodes: Vec<NodeSrc>,
    extras: Vec<Extra>,
    partials: Vec<PartialNode>,
    ambiguous: Option<String>,
    fixed: Option<String>,
    has_char: bool,
    lexicon_bare: bool,
    forms: Vec<(String, i64, String)>,
    sentence: Option<(String, Vec<String>)>,
    plain: Option<Choice>,
    /// Exact syllables of the whole spelling, when it divides cleanly.
    phrase: Vec<u16>,
}

impl std::fmt::Debug for Plan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Plan")
            .field("raw", &self.raw)
            .field("nodes", &self.nodes.len())
            .finish()
    }
}

#[derive(Clone)]
struct NodeSrc {
    node: u32,
    tier: u8,
    divisor: i64,
}

#[derive(Clone)]
struct Extra {
    word: String,
    reading: String,
    tier: u8,
    score: i64,
    seq: u32,
    node: Option<u32>,
    read_at: Option<u32>,
}

#[derive(Clone)]
struct PartialNode {
    node: u32,
    span: usize,
    syls: u16,
}

pub(crate) fn plan(raw: &str, lex: &Lexicon) -> Plan {
    let empty = Plan {
        raw: raw.to_string(),
        nodes: Vec::new(),
        extras: Vec::new(),
        partials: Vec::new(),
        ambiguous: None,
        fixed: None,
        has_char: false,
        lexicon_bare: true,
        forms: Vec::new(),
        sentence: None,
        plain: None,
        phrase: Vec::new(),
    };
    if raw.is_empty()
        || !raw
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte == b'\'')
    {
        return empty;
    }
    let found = gather(raw, lex);
    // All morphology and sentence checks use the same spelling segmentation.
    let syllables = exact_syls(raw, lex);
    let mut lexicon_full = found.bigrams.iter().any(|hit| hit.tier == 0);
    let mut lexicon_bare = found.bigrams.is_empty();
    let mut strong_direct = false;
    let mut has_direct = found.bigrams.iter().any(|hit| hit.tier <= 1);
    let mut nodes = Vec::new();
    // Classify each source once. In particular, tier-two sources used to build
    // and match the same reading in both direct-match predicates.
    for (&node, &(tier, divisor)) in &found.nodes {
        let has_word = node_has_word(lex, node, divisor);
        lexicon_full |= tier == 0 && has_word;
        lexicon_bare &= !has_word;
        has_direct |= tier <= 1 && has_word;
        if tier <= 2
            && (!strong_direct || (tier == 2 && !has_direct))
            && lex.node_words(node).first().is_some_and(|w| w.score >= 100)
        {
            let matched = match_imported_reading(lex.preferences, raw, &reading_path(lex, node))
                .is_some_and(|h| h.span == raw.len() && h.tier <= 2);
            strong_direct |= matched;
            has_direct |= tier == 2 && matched;
        }
        if has_word {
            nodes.push(NodeSrc {
                node,
                tier,
                divisor,
            });
        }
    }

    // Hash-map discovery order must not decide which equally ranked reading
    // survives deduplication or supplies a partial candidate.
    nodes.sort_by_key(|src| (src.tier, src.node, src.divisor));
    let mut forms = if lexicon_full {
        Vec::new()
    } else {
        complement_forms(syllables.as_deref(), lex)
    };
    if !lexicon_full {
        forms.extend(degree_forms(syllables.as_deref(), lex));
        forms.extend(directional_forms(syllables.as_deref(), lex));
        forms.extend(potential_forms(syllables.as_deref(), lex));
        forms.extend(aspect_forms(syllables.as_deref(), lex));
    }
    forms.extend(particle_forms(syllables.as_deref(), lex));
    forms.extend(yixia_forms(syllables.as_deref(), lex));
    forms.extend(redup_forms(syllables.as_deref(), lex));
    forms.extend(negation_forms(syllables.as_deref(), lex));
    forms.extend(mood_forms(syllables.as_deref(), lex));

    let mut extras = Vec::new();
    let mut sentence = None;
    if (!has_direct && forms.is_empty())
        || (raw.len() >= 4 && !lexicon_full && !strong_direct && syllables.is_none())
    {
        let composed = compose_sentences(raw, lex, syllables.is_some());
        if !composed.is_empty() {
            let length = composed[0].1.len().max(1) as f64;
            for (index, (text, syls, cost)) in composed.into_iter().enumerate() {
                let score = (lex.ln_corpus - cost / length)
                    .clamp(0.0, lex.ln_corpus)
                    .exp() as i64;
                extras.push(extra_word(
                    text.clone(),
                    syls.join("'"),
                    score.saturating_sub(index as i64).max(1),
                ));
                if index == 0 {
                    sentence = Some((text, syls));
                }
            }
        } else if let Some(syls) = syllables.as_deref() {
            if let Some(text) = compose(syls, lex) {
                extras.push(extra_word(
                    text.clone(),
                    String::new(),
                    lex.ln_corpus.exp() as i64,
                ));
                sentence = Some((text, syl_text(lex, syls)));
            }
        }
    }

    if lexicon_bare {
        for (word, score, reading) in &forms {
            extras.push(Extra {
                word: word.clone(),
                reading: reading.clone(),
                tier: 0,
                score: (*score).max(1),
                seq: u32::MAX,
                node: None,
                read_at: None,
            });
        }
    }
    for (offset, text) in specials(raw).into_iter().enumerate() {
        extras.push(extra_word(
            text,
            String::new(),
            i64::MAX / 4 - offset as i64,
        ));
    }
    for hit in &found.bigrams {
        if hit.score <= 0 {
            continue;
        }
        extras.push(Extra {
            word: lex.text(hit.text).to_string(),
            reading: hit
                .read_at
                .map(|node| reading_path(lex, node))
                .unwrap_or_default(),
            tier: hit.tier,
            score: hit.score,
            seq: u32::MAX,
            node: None,
            read_at: hit.read_at,
        });
    }

    let has_anything = !found.nodes.is_empty() || !extras.is_empty();
    if !has_anything {
        let letters: String = raw.chars().filter(|ch| *ch != '\'').collect();
        return if letters.is_empty() {
            empty
        } else {
            Plan {
                plain: Some(plain_choice(letters, raw.len())),
                ..empty
            }
        };
    }

    let has_char = nodes.iter().filter(|src| src.tier == 0).any(|src| {
        lex.node_words(src.node)
            .iter()
            .any(|word| lex.text(word.text).chars().count() == 1)
    });
    let ambiguous = found
        .ambiguous
        .map(|(text, _, _)| lex.text(text).to_string());
    let fixed = found.fixed.map(|(text, _)| lex.text(text).to_string());
    let mut planned = Plan {
        raw: raw.to_string(),
        nodes,
        extras,
        partials: Vec::new(),
        ambiguous,
        fixed,
        has_char,
        lexicon_bare,
        forms,
        sentence,
        plain: None,
        phrase: syllables.filter(|syls| syls.len() >= 2).unwrap_or_default(),
    };
    planned.partials = partial_nodes(&planned, lex);
    planned
}

fn node_has_word(lex: &Lexicon, node: u32, divisor: i64) -> bool {
    lex.node_words(node)
        .iter()
        .any(|word| word.score / divisor > 0)
}

fn extra_word(word: String, reading: String, score: i64) -> Extra {
    Extra {
        word,
        reading,
        tier: 0,
        score,
        seq: u32::MAX,
        node: None,
        read_at: None,
    }
}

#[derive(Clone)]
struct PhraseState<I> {
    score: u32,
    idx: I,
}

impl<I: Ord> PartialEq for PhraseState<I> {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl<I: Ord> Eq for PhraseState<I> {}

impl<I: Ord> PartialOrd for PhraseState<I> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<I: Ord> Ord for PhraseState<I> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.score
            .cmp(&other.score)
            .then_with(|| other.idx.cmp(&self.idx))
    }
}

/// Single characters for each exact syllable, best combinations first.
/// `budanzhi` has no stored 不单止; the characters still combine here.
fn assemble_phrases(syls: &[u16], lex: &Lexicon, limit: usize) -> Vec<(String, i64, String)> {
    phrase_cursor(syls, lex, limit).collect()
}

#[derive(Clone)]
enum PhraseCursor {
    Empty,
    Packed(PhraseCombinations<u128>),
    Wide(PhraseCombinations<Vec<u8>>),
}

impl Iterator for PhraseCursor {
    type Item = (String, i64, String);
    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Empty => None,
            Self::Packed(cursor) => cursor.next(),
            Self::Wide(cursor) => cursor.next(),
        }
    }
}

fn phrase_cursor(syls: &[u16], lex: &Lexicon, limit: usize) -> PhraseCursor {
    if syls.len() < 2 || limit == 0 {
        return PhraseCursor::Empty;
    }
    let reading = joined_reading(syls, lex);
    let mut cols = Vec::new();
    for syl in syls {
        let Some(node) = lex.walk_id(0, *syl) else {
            return PhraseCursor::Empty;
        };
        let mut col = Vec::new();
        for word in lex.node_words(node) {
            let text = lex.text(word.text);
            if word.score <= 0 || text.chars().count() != 1 {
                continue;
            }
            col.push((text, word.score, word.score.max(1).ilog2()));
            if col.len() == PHRASE_COLUMN {
                break;
            }
        }
        if col.is_empty() {
            return PhraseCursor::Empty;
        }
        cols.push(col);
    }
    if cols.len() <= 25 {
        PhraseCursor::Packed(PhraseCombinations::new(cols, reading, limit))
    } else {
        PhraseCursor::Wide(PhraseCombinations::new(cols, reading, limit))
    }
}

// Five bits address each of the 24 characters. Fixed-width numeric order is
// exactly the original lexicographic index order; longer phrases keep Vecs.
trait PhraseIndex: Clone + Eq + Ord + std::hash::Hash {
    fn allocated_bytes(&self) -> usize;
    fn zero(width: usize) -> Self;
    fn at(&self, dim: usize, width: usize) -> usize;
    fn increment(&self, dim: usize, width: usize) -> Self;
}

impl PhraseIndex for u128 {
    fn allocated_bytes(&self) -> usize {
        0
    }
    fn zero(_: usize) -> Self {
        0
    }
    fn at(&self, dim: usize, width: usize) -> usize {
        ((self >> ((width - 1 - dim) * 5)) & 31) as usize
    }
    fn increment(&self, dim: usize, width: usize) -> Self {
        self + (1u128 << ((width - 1 - dim) * 5))
    }
}

impl PhraseIndex for Vec<u8> {
    fn allocated_bytes(&self) -> usize {
        self.capacity()
    }
    fn zero(width: usize) -> Self {
        vec![0; width]
    }
    fn at(&self, dim: usize, _: usize) -> usize {
        self[dim] as usize
    }
    fn increment(&self, dim: usize, _: usize) -> Self {
        let mut next = self.clone();
        next[dim] += 1;
        next
    }
}

type PhraseColumn = Vec<(&'static str, i64, u32)>;

#[derive(Clone)]
struct PhraseCombinations<I> {
    cols: Vec<PhraseColumn>,
    reading: String,
    seen: HashSet<I>,
    heap: BinaryHeap<PhraseState<I>>,
    remaining: usize,
}

impl<I: PhraseIndex> PhraseCombinations<I> {
    fn new(cols: Vec<PhraseColumn>, reading: String, limit: usize) -> Self {
        let start = I::zero(cols.len());
        let mut seen = HashSet::new();
        seen.insert(start.clone());
        let mut heap = BinaryHeap::new();
        heap.push(PhraseState {
            score: cols.iter().map(|col| col[0].2).sum(),
            idx: start,
        });
        Self {
            cols,
            reading,
            seen,
            heap,
            remaining: limit,
        }
    }
}

impl<I: PhraseIndex> Iterator for PhraseCombinations<I> {
    type Item = (String, i64, String);
    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        let state = self.heap.pop()?;
        self.remaining -= 1;
        let width = self.cols.len();
        let mut word = String::with_capacity(width * 4);
        let mut score = i64::MAX;
        for (dim, col) in self.cols.iter().enumerate() {
            let (text, char_score, _) = &col[state.idx.at(dim, width)];
            word.push_str(text);
            score = score.min(*char_score);
        }
        for dim in 0..width {
            let next = state.idx.at(dim, width) + 1;
            if next >= self.cols[dim].len() {
                continue;
            }
            let idx = state.idx.increment(dim, width);
            if !self.seen.insert(idx.clone()) {
                continue;
            }
            let total = self
                .cols
                .iter()
                .enumerate()
                .map(|(dim, col)| col[idx.at(dim, width)].2)
                .sum();
            self.heap.push(PhraseState { score: total, idx });
        }
        Some((word, score, self.reading.clone()))
    }
}

fn partial_nodes(plan: &Plan, lex: &Lexicon) -> Vec<PartialNode> {
    // No positive-length prefix can be shorter than a one-byte input.
    if plan.raw.len() < 2 {
        return Vec::new();
    }
    let mut batches = Vec::new();
    if let Some(first) = peek_ranked(plan, lex) {
        if let Some(syls) = first.node.map(|node| path_syls(lex, node)).or_else(|| {
            plan.sentence
                .as_ref()
                .filter(|(word, _)| word == &first.word)
                .map(|(_, syls)| syls.clone())
        }) {
            if syls.len() >= 2 {
                batches.push(syls);
            }
        }
    }
    if plan.phrase.len() >= 2 {
        let phrase = syl_text(lex, &plan.phrase);
        if batches.first() != Some(&phrase) {
            batches.push(phrase);
        }
    }
    let mut found = Vec::new();
    let mut seen = HashSet::new();
    for syls in batches {
        for partial in prefix_nodes(plan, lex, &syls) {
            if seen.insert(partial.node) {
                found.push(partial);
            }
        }
    }
    found
}

fn prefix_nodes(plan: &Plan, lex: &Lexicon, syls: &[String]) -> Vec<PartialNode> {
    if syls.len() < 2 {
        return Vec::new();
    }
    let Some(ends) = align_ends(lex.preferences, &plan.raw, syls) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    let mut node = 0u32;
    for len in 1..syls.len() {
        let Some(next) = lex.walk(node, &syls[len - 1]) else {
            break;
        };
        node = next;
        let span = skip_sep(&plan.raw, ends[len - 1]);
        if span == 0 || span >= plan.raw.len() {
            continue;
        }
        found.push(PartialNode {
            node,
            span,
            syls: len as u16,
        });
    }
    found
}

fn peek_ranked(plan: &Plan, lex: &Lexicon) -> Option<Ranked> {
    RawMerge::start(plan, lex).pop(lex)
}

impl Plan {
    fn realize(&self, lex: &Lexicon) -> Listing {
        if let Some(plain) = &self.plain {
            return Listing {
                full: vec![plain.clone()],
                partials: Vec::new(),
            };
        }
        let mut ranked = Vec::new();
        let mut merge = RawMerge::start(self, lex);
        while let Some(item) = merge.pop(lex) {
            ranked.push(item);
        }
        self.pin(&mut ranked);
        let full = ranked
            .into_iter()
            .map(|item| self.choice(lex, &item, 0))
            .collect();
        let partials = expand_partials(self, lex);
        Listing { full, partials }
    }

    fn pin(&self, ranked: &mut Vec<Ranked>) {
        if let Some(word) = &self.ambiguous {
            if self.has_char {
                if let Some(pos) = ranked.iter().position(|item| &item.word == word) {
                    if pos > 1 {
                        let item = ranked.remove(pos);
                        ranked.insert(1, item);
                    }
                }
            }
        }
        if !self.lexicon_bare {
            let mut readings = HashMap::new();
            place_complements(ranked, &self.forms, &mut readings);
        }
        if let Some(word) = &self.fixed {
            if let Some(pos) = ranked.iter().position(|item| &item.word == word) {
                if pos > 1 {
                    let item = ranked.remove(pos);
                    ranked.insert(1, item);
                }
            }
        }
    }

    fn choice(&self, lex: &Lexicon, item: &Ranked, weight: u32) -> Choice {
        let mut reading = reading_of(lex, item.read_at, &item.word, self.sentence.as_ref());
        if reading.is_empty() {
            if let Some(extra) = self.extras.iter().find(|extra| extra.word == item.word) {
                if !extra.reading.is_empty() {
                    reading = extra.reading.clone();
                }
            }
        }
        if reading.is_empty() {
            if let Some((_, _, form)) = self.forms.iter().find(|(word, _, _)| word == &item.word) {
                reading = form.clone();
            }
        }
        Choice {
            word: item.word.clone(),
            span: self.raw.len(),
            reading,
            tier: item.tier,
            score: item.score,
            user_weight: weight,
        }
    }
}

struct RawMerge<'a> {
    plan: &'a Plan,
    at: Vec<u32>,
    heap: BinaryHeap<MergeItem>,
    seen: HashSet<Cow<'static, str>>,
}

#[derive(Clone)]
struct MergeItem {
    tier: u8,
    score: i64,
    seq: u32,
    chars: u32,
    word: Cow<'static, str>,
    node: Option<u32>,
    read_at: Option<u32>,
    src: usize,
    kind: u8,
}

impl PartialEq for MergeItem {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for MergeItem {}

impl PartialOrd for MergeItem {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for MergeItem {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .tier
            .cmp(&self.tier)
            .then(self.score.cmp(&other.score))
            .then(other.seq.cmp(&self.seq))
            .then(other.chars.cmp(&self.chars))
            .then(other.word.cmp(&self.word))
    }
}

impl<'a> RawMerge<'a> {
    fn start(plan: &'a Plan, lex: &Lexicon) -> Self {
        let mut merge = Self {
            plan,
            at: vec![0; plan.nodes.len()],
            heap: BinaryHeap::new(),
            seen: HashSet::new(),
        };
        for index in 0..plan.nodes.len() {
            merge.push_node(lex, index);
        }
        for (index, extra) in plan.extras.iter().enumerate() {
            merge.heap.push(MergeItem {
                tier: extra.tier,
                score: extra.score,
                seq: extra.seq,
                chars: extra.word.chars().count() as u32,
                word: Cow::Owned(extra.word.clone()),
                node: extra.node,
                read_at: extra.read_at,
                src: index,
                kind: 1,
            });
        }
        merge
    }

    fn push_node(&mut self, lex: &Lexicon, index: usize) {
        let src = &self.plan.nodes[index];
        let words = lex.node_words(src.node);
        while (self.at[index] as usize) < words.len() {
            let word = words[self.at[index] as usize];
            self.at[index] += 1;
            let score = word.score / src.divisor;
            if score <= 0 {
                continue;
            }
            let text = lex.text(word.text);
            self.heap.push(MergeItem {
                tier: src.tier,
                score,
                seq: word.seq,
                chars: text.chars().count() as u32,
                word: Cow::Borrowed(text),
                node: Some(src.node),
                read_at: Some(src.node),
                src: index,
                kind: 0,
            });
            break;
        }
    }

    fn pop(&mut self, lex: &Lexicon) -> Option<Ranked> {
        while let Some(item) = self.heap.pop() {
            if item.kind == 0 {
                self.push_node(lex, item.src);
            }
            if !self.seen.insert(item.word.clone()) {
                continue;
            }
            return Some(Ranked {
                word: item.word.into_owned(),
                tier: item.tier,
                score: item.score,
                node: item.node,
                read_at: item.read_at,
            });
        }
        None
    }

    fn pop_tier(&mut self, lex: &Lexicon, tier: u8) -> Option<Ranked> {
        loop {
            let top = self.heap.peek()?;
            if top.tier != tier {
                return None;
            }
            let item = self.heap.pop().unwrap();
            if item.kind == 0 {
                self.push_node(lex, item.src);
            }
            if !self.seen.insert(item.word.clone()) {
                continue;
            }
            return Some(Ranked {
                word: item.word.into_owned(),
                tier: item.tier,
                score: item.score,
                node: item.node,
                read_at: item.read_at,
            });
        }
    }
}

/// Session-order cursor. Learned scores are bounded; prefixes follow exact matches.
#[derive(Clone)]
pub(crate) struct Pool {
    plan: Arc<Plan>,
    merge: Option<MergeState>,
    head: Vec<Choice>,
    head_at: usize,
    users: Vec<Choice>,
    user_at: usize,
    shown: HashSet<String>,
    tier: u8,
    prefix: Option<Choice>,
    emitted_full: bool,
    soon: Vec<Choice>,
    user_partials: Vec<Choice>,
    partials: Vec<Choice>,
    partial_at: usize,
    partials_ready: bool,
    phrases: Option<PhraseCursor>,
    phase: u8,
}

/// Dictionary-only first page and its independent continuation cursor.
/// Personal/imported overlays are always applied on the foreground path.
pub(crate) struct Prepared {
    pub(crate) pool: Pool,
    pub(crate) out: Arc<Vec<Choice>>,
    pub(crate) done: bool,
}

impl Prepared {
    pub(crate) fn new(plan: Arc<Plan>, lex: &Lexicon, page_size: usize) -> Self {
        let mut pool = Pool::open(plan, lex, Vec::new(), Vec::new());
        let mut out = Vec::with_capacity(page_size);
        let mut done = false;
        while out.len() < page_size {
            if let Some(choice) = pool.next(lex) {
                out.push(choice);
            } else {
                done = true;
                break;
            }
        }
        Self {
            pool,
            out: Arc::new(out),
            done,
        }
    }

    pub(crate) fn plan(&self) -> Arc<Plan> {
        self.pool.plan.clone()
    }

    /// Conservative accounting of owned payloads, not mapped dictionary pages
    /// or allocator/RSS accounting. Shared plans are counted for each entry.
    pub(crate) fn bytes(&self) -> usize {
        use std::mem::{size_of, size_of_val};
        let choice_strings = |c: &Choice| c.word.capacity() + c.reading.capacity();
        let choices = |v: &Vec<Choice>| {
            v.capacity() * size_of::<Choice>() + v.iter().map(choice_strings).sum::<usize>()
        };
        let p = &self.pool;
        let plan = &p.plan;
        let mut bytes = size_of_val(self)
            + size_of::<Plan>()
            + choices(&self.out)
            + plan.raw.capacity()
            + plan.nodes.capacity() * size_of::<NodeSrc>()
            + plan.extras.capacity() * size_of::<Extra>()
            + plan.partials.capacity() * size_of::<PartialNode>()
            + plan.phrase.capacity() * size_of::<u16>()
            + plan.forms.capacity() * size_of::<(String, i64, String)>();
        bytes += plan
            .extras
            .iter()
            .map(|e| e.word.capacity() + e.reading.capacity())
            .sum::<usize>();
        bytes += plan
            .forms
            .iter()
            .map(|(w, _, r)| w.capacity() + r.capacity())
            .sum::<usize>();
        bytes += plan.ambiguous.as_ref().map_or(0, String::capacity)
            + plan.fixed.as_ref().map_or(0, String::capacity)
            + plan.plain.as_ref().map_or(0, choice_strings);
        if let Some((text, reading)) = &plan.sentence {
            bytes += text.capacity()
                + reading.capacity() * size_of::<String>()
                + reading.iter().map(String::capacity).sum::<usize>();
        }
        for v in [&p.head, &p.users, &p.soon, &p.user_partials, &p.partials] {
            bytes += choices(v);
        }
        bytes += p.prefix.as_ref().map_or(0, choice_strings)
            + p.shown.capacity() * (size_of::<String>() + 16)
            + p.shown.iter().map(String::capacity).sum::<usize>();
        let cow_bytes = |s: &Cow<'static, str>| match s {
            Cow::Owned(s) => s.capacity(),
            _ => 0,
        };
        if let Some(merge) = &p.merge {
            bytes += merge.at.capacity() * size_of::<u32>()
                + merge.heap.capacity() * size_of::<MergeItem>()
                + merge.heap.iter().map(|i| cow_bytes(&i.word)).sum::<usize>()
                + merge.seen.capacity() * (size_of::<Cow<'static, str>>() + 16)
                + merge.seen.iter().map(cow_bytes).sum::<usize>();
        }
        fn cursor_bytes<I: PhraseIndex>(c: &PhraseCombinations<I>) -> usize {
            c.cols.capacity() * std::mem::size_of::<PhraseColumn>()
                + c.cols
                    .iter()
                    .map(|v| v.capacity() * std::mem::size_of::<(&str, i64, u32)>())
                    .sum::<usize>()
                + c.reading.capacity()
                + c.seen.capacity() * (std::mem::size_of::<I>() + 16)
                + c.seen
                    .iter()
                    .map(PhraseIndex::allocated_bytes)
                    .sum::<usize>()
                + c.heap.capacity() * std::mem::size_of::<PhraseState<I>>()
                + c.heap
                    .iter()
                    .map(|s| s.idx.allocated_bytes())
                    .sum::<usize>()
        }
        bytes += match p.phrases.as_ref() {
            Some(PhraseCursor::Packed(c)) => cursor_bytes(c),
            Some(PhraseCursor::Wide(c)) => cursor_bytes(c),
            _ => 0,
        };
        bytes
    }
}

#[derive(Clone)]
struct MergeState {
    at: Vec<u32>,
    heap: BinaryHeap<MergeItem>,
    seen: HashSet<Cow<'static, str>>,
}

fn stored_tier(plan: &Plan, lex: &Lexicon, word: &str) -> Option<u8> {
    let mut best: Option<u8> = None;
    let keep = |best: &mut Option<u8>, tier: u8| {
        *best = Some(best.map_or(tier, |old| old.min(tier)));
    };
    for src in &plan.nodes {
        if lex
            .node_words(src.node)
            .iter()
            .any(|item| lex.text(item.text) == word)
        {
            keep(&mut best, src.tier);
        }
    }
    for extra in &plan.extras {
        if extra.word == word {
            keep(&mut best, extra.tier);
        }
    }
    best
}

impl Pool {
    pub(crate) fn prefer(&mut self, choice: Choice) {
        self.soon.retain(|other| other.word != choice.word);
        self.remember(&choice.word);
        self.emitted_full = true;
        self.soon.push(choice);
    }

    pub(crate) fn open(
        plan: Arc<Plan>,
        lex: &Lexicon,
        mut users: Vec<Choice>,
        partials: Vec<Choice>,
    ) -> Self {
        let native_exact = plan
            .nodes
            .iter()
            .any(|src| src.tier == 0 && node_has_word(lex, src.node, src.divisor))
            || plan
                .extras
                .iter()
                .any(|e| e.tier == 0 && e.read_at.is_some());
        if !native_exact {
            for choice in &mut users {
                if choice.user_weight == 0
                    && !choice.reading.is_empty()
                    && choice.reading.replace('\'', "") == plan.raw.replace('\'', "")
                {
                    // A fully spelled stored phrase precedes generated grammar
                    // forms and guessed sentences, never a native exact word.
                    choice.score = i64::MAX / 16;
                }
            }
        }
        // The corpus floor only affects learned entries without a corpus score.
        // Avoid constructing a complete merge heap for ordinary empty overlays.
        let mut floor = None;
        for choice in users.iter_mut() {
            if choice.user_weight > 0 && choice.score == 0 {
                choice.score = *floor
                    .get_or_insert_with(|| peek_ranked(&plan, lex).map_or(100, |r| r.score / 2));
            }
            if let Some(tier) = stored_tier(&plan, lex, &choice.word) {
                choice.tier = tier;
            }
            choice.score = lex.contextual_score(choice.score, &choice.word);
        }
        users.sort_by(|left, right| {
            left.tier
                .cmp(&right.tier)
                .then(
                    (right
                        .score
                        .max(1)
                        .saturating_mul(1 + i64::from(right.user_weight.min(8))))
                    .cmp(
                        &left
                            .score
                            .max(1)
                            .saturating_mul(1 + i64::from(left.user_weight.min(8))),
                    ),
                )
                .then(left.word.cmp(&right.word))
        });
        let prefix = best_prefix(&plan, lex, &partials);
        let mut pool = Self {
            plan,
            merge: None,
            head: Vec::new(),
            head_at: 0,
            users,
            user_at: 0,
            shown: HashSet::new(),
            tier: 0,
            prefix,
            emitted_full: false,
            soon: Vec::new(),
            user_partials: partials,
            partials: Vec::new(),
            partial_at: 0,
            partials_ready: false,
            phrases: None,
            phase: 0,
        };
        if pool.plan.plain.is_some() && pool.users.is_empty() {
            pool.soon = vec![pool.plan.plain.clone().unwrap()];
            pool.phase = 2;
            pool.partials_ready = true;
        }
        pool
    }

    pub(crate) fn next(&mut self, lex: &Lexicon) -> Option<Choice> {
        if let Some(choice) = self.soon.pop() {
            return Some(choice);
        }
        if self.phase == 0 {
            self.boot(lex);
            self.phase = 1;
        }
        if self.phase == 1 {
            if let Some(choice) = self.pull_tier(lex) {
                return Some(self.note_first(choice));
            }
            if self.tier == 0 && self.emitted_full {
                if let Some(prefix) = self.prefix.take() {
                    if !self.remembered(&prefix.word) {
                        self.remember(&prefix.word);
                        return Some(prefix);
                    }
                }
            }
            if self.tier < 3 {
                self.tier += 1;
                return self.next(lex);
            }
            self.phase = 2;
        }
        if !self.partials_ready {
            let users = std::mem::take(&mut self.user_partials);
            self.partials = expand_partials_tail(&self.plan, lex, users);
            self.partials_ready = true;
        }
        while self.partial_at < self.partials.len() {
            let choice = self.partials[self.partial_at].clone();
            self.partial_at += 1;
            if self.remembered(&choice.word) {
                continue;
            }
            self.remember(&choice.word);
            return Some(choice);
        }
        if self.phrases.is_none() {
            self.phrases = Some(phrase_cursor(&self.plan.phrase, lex, PHRASE_LIMIT));
        }
        while let Some((word, score, reading)) = self.phrases.as_mut().unwrap().next() {
            if self.remembered(&word) {
                continue;
            }
            self.remember(&word);
            return Some(Choice {
                word,
                score,
                reading,
                span: self.plan.raw.len(),
                tier: 0,
                user_weight: 0,
            });
        }
        None
    }

    fn note_first(&mut self, choice: Choice) -> Choice {
        if !self.emitted_full && self.tier > 0 {
            if let Some(prefix) = self.prefix.take() {
                if prefix.word != choice.word && !self.remembered(&prefix.word) {
                    self.remember(&prefix.word);
                    self.soon.push(prefix);
                }
            }
        }
        self.emitted_full = true;
        self.remember(&choice.word);
        choice
    }

    fn remembered(&self, word: &str) -> bool {
        self.merge
            .as_ref()
            .is_some_and(|merge| merge.seen.contains(word))
    }

    fn remember(&mut self, word: &str) {
        self.shown.insert(word.to_string());
        if let Some(merge) = self.merge.as_mut() {
            merge.seen.insert(Cow::Owned(word.to_string()));
        }
    }

    fn boot(&mut self, lex: &Lexicon) {
        let mut merge = RawMerge::start(&self.plan, lex);
        let mut first = Vec::new();
        let mut held = None;
        let contextual =
            lex.context_enabled() && !lex.ranking_context.is_empty() && !self.plan.has_char;
        while first.len() < if contextual { 32 } else { 3 } {
            let Some(item) = merge.pop_tier(lex, 0) else {
                break;
            };
            if self.plan.has_char
                && self.plan.ambiguous.as_ref() == Some(&item.word)
                && first.len() > 1
            {
                held = Some(item);
                continue;
            }
            first.push(item);
        }
        if contextual {
            for item in &mut first {
                item.score = lex.contextual_score(item.score, &item.word);
            }
            // Stable ties retain subtitle counts and the dictionary's existing
            // homophone order, including after personal weights decay.
            first.sort_by_key(|a| std::cmp::Reverse(a.score));
        }
        if held.is_none() {
            if let Some(word) = &self.plan.ambiguous {
                if self.plan.has_char && !first.iter().any(|item| &item.word == word) {
                    held = Some(synthetic(word, 0));
                    merge.seen.insert(Cow::Owned(word.clone()));
                }
            }
        }
        if let Some(item) = held {
            let at = if first.len() > 1 { 1 } else { first.len() };
            first.insert(at, item);
        }
        if !self.plan.lexicon_bare {
            let mut readings = HashMap::new();
            place_complements(&mut first, &self.plan.forms, &mut readings);
        }
        self.head = first
            .iter()
            .map(|item| self.plan.choice(lex, item, 0))
            .collect();
        merge
            .seen
            .extend(self.shown.iter().cloned().map(Cow::Owned));
        self.merge = Some(MergeState {
            at: merge.at,
            heap: merge.heap,
            seen: merge.seen,
        });
    }

    fn pull_tier(&mut self, lex: &Lexicon) -> Option<Choice> {
        let tier = self.tier;
        while self.user_at < self.users.len() && self.users[self.user_at].tier < tier {
            self.user_at += 1;
        }
        let base_score = if tier == 0 && self.head_at < self.head.len() {
            self.head[self.head_at].score
        } else {
            self.merge
                .as_ref()
                .and_then(|m| m.heap.peek())
                .filter(|m| m.tier == tier)
                .map_or(0, |m| m.score)
        };
        if self.user_at < self.users.len()
            && self.users[self.user_at].tier == tier
            && self.users[self.user_at]
                .score
                .max(if self.users[self.user_at].user_weight > 0 {
                    100
                } else {
                    1
                })
                .saturating_mul(1 + i64::from(self.users[self.user_at].user_weight.min(8)))
                >= base_score
        {
            let choice = self.users[self.user_at].clone();
            self.user_at += 1;
            if self.shown.insert(choice.word.clone()) {
                self.merge
                    .as_mut()
                    .unwrap()
                    .seen
                    .insert(Cow::Owned(choice.word.clone()));
                return Some(choice);
            }
            return self.pull_tier(lex);
        }
        if tier == 0 && self.head_at < self.head.len() {
            let choice = self.head[self.head_at].clone();
            self.head_at += 1;
            if !self.shown.insert(choice.word.clone()) {
                return self.pull_tier(lex);
            }
            return Some(choice);
        }
        let merge = self.merge.as_mut()?;
        loop {
            let top = merge.heap.peek()?;
            if top.tier != tier {
                return None;
            }
            let item = merge.heap.pop().unwrap();
            if item.kind == 0 {
                let src = &self.plan.nodes[item.src];
                let words = lex.node_words(src.node);
                while (merge.at[item.src] as usize) < words.len() {
                    let word = words[merge.at[item.src] as usize];
                    merge.at[item.src] += 1;
                    let score = word.score / src.divisor;
                    if score <= 0 {
                        continue;
                    }
                    let text = lex.text(word.text);
                    merge.heap.push(MergeItem {
                        tier: src.tier,
                        score,
                        seq: word.seq,
                        chars: text.chars().count() as u32,
                        word: Cow::Borrowed(text),
                        node: Some(src.node),
                        read_at: Some(src.node),
                        src: item.src,
                        kind: 0,
                    });
                    break;
                }
            }
            if !merge.seen.insert(item.word.clone()) {
                continue;
            }
            let ranked = Ranked {
                word: item.word.into_owned(),
                tier: item.tier,
                score: item.score,
                node: item.node,
                read_at: item.read_at,
            };
            return Some(self.plan.choice(lex, &ranked, 0));
        }
    }
}

fn synthetic(word: &str, score: i64) -> Ranked {
    Ranked {
        word: word.to_string(),
        tier: 0,
        score,
        node: None,
        read_at: None,
    }
}

fn best_prefix(plan: &Plan, lex: &Lexicon, users: &[Choice]) -> Option<Choice> {
    if let Some(user) = users.first() {
        return Some(user.clone());
    }
    let mut picked: Option<(Choice, u16)> = None;
    for src in &plan.partials {
        let Some(word) = lex.node_words(src.node).first() else {
            continue;
        };
        let choice = Choice {
            word: lex.text(word.text).to_string(),
            span: src.span,
            reading: reading_path(lex, src.node),
            tier: 0,
            score: word.score,
            user_weight: 0,
        };
        let replace = match &picked {
            None => true,
            Some((old, old_syls)) => {
                let cost = partial_cost(choice.score, src.syls as usize, lex.ln_corpus);
                let old_cost = partial_cost(old.score, *old_syls as usize, lex.ln_corpus);
                cost < old_cost
                    || ((cost - old_cost).abs() < f64::EPSILON
                        && (choice.score > old.score
                            || (choice.score == old.score && choice.word < old.word)))
            }
        };
        if replace {
            picked = Some((choice, src.syls));
        }
    }
    picked.map(|(choice, _)| choice)
}

fn expand_partials(plan: &Plan, lex: &Lexicon) -> Vec<Choice> {
    expand_partials_tail(plan, lex, Vec::new())
}

fn expand_partials_tail(plan: &Plan, lex: &Lexicon, mut users: Vec<Choice>) -> Vec<Choice> {
    let mut found = Vec::new();
    for src in &plan.partials {
        for word in lex.node_words(src.node) {
            found.push((
                lex.text(word.text).to_string(),
                word.score,
                partial_cost(word.score, src.syls as usize, lex.ln_corpus),
                src.span,
                src.node,
            ));
        }
    }
    found.sort_by(|left, right| {
        left.2
            .partial_cmp(&right.2)
            .unwrap_or(Ordering::Equal)
            .then(right.1.cmp(&left.1))
            .then(left.0.cmp(&right.0))
    });
    let mut lexicon = Vec::new();
    for (word, score, _, span, node) in found {
        lexicon.push(Choice {
            reading: reading_path(lex, node),
            word,
            span,
            tier: 0,
            score,
            user_weight: 0,
        });
    }
    users.append(&mut lexicon);
    users
}

fn plain_choice(word: String, span: usize) -> Choice {
    Choice {
        word,
        span,
        reading: String::new(),
        tier: 0,
        score: 0,
        user_weight: 0,
    }
}

fn reading_of(
    lex: &Lexicon,
    read_at: Option<u32>,
    word: &str,
    sentence: Option<&(String, Vec<String>)>,
) -> String {
    if let Some(node) = read_at {
        let reading = reading_path(lex, node);
        if !reading.is_empty() {
            return reading;
        }
    }
    if let Some((text, syls)) = sentence {
        if text == word {
            return syls.join("'");
        }
    }
    String::new()
}

fn partial_cost(score: i64, len: usize, ln_corpus: f64) -> f64 {
    (ln_corpus - (score.max(1) as f64).ln()) / len.max(1) as f64
}

pub(crate) fn attach_partials(choices: &mut Vec<Choice>, partials: Vec<Choice>) {
    let mut seen: HashSet<String> = choices.iter().map(|choice| choice.word.clone()).collect();
    let fresh: Vec<Choice> = partials
        .into_iter()
        .filter(|choice| seen.insert(choice.word.clone()))
        .collect();
    if fresh.is_empty() || choices.is_empty() {
        return;
    }
    // Exact whole-input matches retain their order before prefixes.
    let at = choices
        .iter()
        .position(|c| c.tier > 0)
        .unwrap_or(choices.len());
    choices.insert(at.max(1), fresh[0].clone());
    choices.extend(fresh.into_iter().skip(1));
}

fn skip_sep(raw: &str, mut end: usize) -> usize {
    while raw.as_bytes().get(end) == Some(&b'\'') {
        end += 1;
    }
    end
}

fn align_ends(prefs: crate::InputPreferences, raw: &str, syls: &[String]) -> Option<Vec<usize>> {
    if syls.is_empty() || raw.is_empty() {
        return None;
    }
    if raw.contains('\'') {
        return align_quoted(prefs, raw, syls);
    }
    let mut ends = Vec::new();
    if align_walk(prefs, raw, syls, 0, 0, &mut ends) {
        Some(ends)
    } else {
        None
    }
}

fn align_walk(
    prefs: crate::InputPreferences,
    raw: &str,
    syls: &[String],
    index: usize,
    syl_index: usize,
    ends: &mut Vec<usize>,
) -> bool {
    if syl_index == syls.len() {
        return index == raw.len();
    }
    let syl = syls[syl_index].as_str();
    let rest = &raw[index..];
    if !head_compatible(prefs, rest, syl) {
        return false;
    }
    let max_len = rest.len().min(syl.len() + 2);
    for len in (1..=max_len).rev() {
        if piece_match(prefs, &rest[..len], syl).is_none() {
            continue;
        }
        ends.push(index + len);
        if align_walk(prefs, raw, syls, index + len, syl_index + 1, ends) {
            return true;
        }
        ends.pop();
    }
    false
}

fn align_quoted(prefs: crate::InputPreferences, raw: &str, syls: &[String]) -> Option<Vec<usize>> {
    let parts: Vec<&str> = raw.split('\'').filter(|part| !part.is_empty()).collect();
    if parts.len() != syls.len() {
        return None;
    }
    let bytes = raw.as_bytes();
    let mut pos = 0;
    let mut ends = Vec::with_capacity(syls.len());
    for (part, syl) in parts.iter().zip(syls.iter()) {
        if pos < bytes.len() && bytes[pos] == b'\'' {
            pos += 1;
        }
        if !raw[pos..].starts_with(part) || piece_match(prefs, part, syl).is_none() {
            return None;
        }
        pos += part.len();
        ends.push(pos);
    }
    if pos == raw.len() {
        Some(ends)
    } else {
        None
    }
}

fn partner_letters(prefs: crate::InputPreferences, letter: u8) -> [u8; 3] {
    prefs.heads(letter)
}

struct BigramHit {
    text: u32,
    tier: u8,
    score: i64,
    read_at: Option<u32>,
}

fn collect_bigrams(raw: &str, lex: &Lexicon) -> Vec<BigramHit> {
    if raw.contains('\'') {
        return Vec::new();
    }
    let Some(node) = lex.auto_end(raw) else {
        return Vec::new();
    };
    let node = node as usize;
    let (start, end, tier) = if lex.auto_term[node] != u16::MAX {
        let range = lex.exact_at[node];
        (range.start, range.end, 0)
    } else {
        let range = lex.extend_at[node];
        (range.start, range.end, 1)
    };
    let pool = if tier == 0 { &lex.exact } else { &lex.extend };
    pool[start as usize..end as usize]
        .iter()
        .filter(|item| item.score > 0)
        .map(|item| BigramHit {
            text: item.text,
            tier,
            score: item.score,
            read_at: lex
                .word_at
                .get(item.text as usize)
                .copied()
                .filter(|node| *node != u32::MAX),
        })
        .collect()
}

struct Step {
    node: u32,
    pos: u32,
    tier: u8,
}

/// Walk the syllable trie and keep the nodes that matched. Words stay on the
/// node until a page is merged out of those lists.
struct Gather {
    nodes: HashMap<u32, (u8, i64)>,
    bigrams: Vec<BigramHit>,
    ambiguous: Option<(u32, u32, i64)>,
    fixed: Option<(u32, u32)>,
}

fn gather(raw: &str, lex: &Lexicon) -> Gather {
    let mut nodes = HashMap::new();
    let mut ambiguous = None;
    match_lexicon(raw, lex, &mut nodes, &mut ambiguous);
    let bigrams = collect_bigrams(raw, lex);
    let mut fixed = None;
    if correction_applies(raw, &nodes, &bigrams) {
        let before: HashSet<u32> = nodes.keys().copied().collect();
        correct_lexicon(raw, lex, &mut nodes);
        fixed = best_fresh_word(lex, &nodes, &before);
    }
    Gather {
        nodes,
        bigrams,
        ambiguous,
        fixed,
    }
}

fn best_fresh_word(
    lex: &Lexicon,
    nodes: &HashMap<u32, (u8, i64)>,
    before: &HashSet<u32>,
) -> Option<(u32, u32)> {
    let mut best: Option<(i64, u32, u32)> = None;
    for (&node, &(_tier, divisor)) in nodes {
        if before.contains(&node) {
            continue;
        }
        for word in lex.node_words(node) {
            if lex.text(word.text).chars().count() < 2 {
                continue;
            }
            let score = word.score / divisor;
            if score <= 0 {
                continue;
            }
            let replace = best
                .map(|(old, old_text, old_node)| {
                    score > old
                        || (score == old
                            && (lex.text(word.text), node) < (lex.text(old_text), old_node))
                })
                .unwrap_or(true);
            if replace {
                best = Some((score, word.text, node));
            }
            break;
        }
    }
    best.map(|(_, text, node)| (text, node))
}

fn valid_syllables(reading: &str) -> bool {
    !reading.is_empty()
        && reading
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte == b'\'')
        && !reading.starts_with('\'')
        && !reading.ends_with('\'')
        && !reading.contains("''")
}

/// One keyboard edit, and only when the typed spelling has no exact word and
/// no prefix completion. Exact spellings such as `nihao` never reach this.
fn correction_applies(raw: &str, nodes: &HashMap<u32, (u8, i64)>, bigrams: &[BigramHit]) -> bool {
    (4..=16).contains(&raw.len())
        && !raw.contains('\'')
        && raw.bytes().all(|byte| byte.is_ascii_lowercase())
        && nodes.values().all(|(tier, _)| *tier >= 2)
        && bigrams.iter().all(|hit| hit.tier >= 2)
}

struct CorrStep {
    node: u32,
    pos: u32,
    tier: u8,
    edited: bool,
}

fn correct_lexicon(raw: &str, lex: &Lexicon, nodes: &mut HashMap<u32, (u8, i64)>) {
    thread_local! {
        static SEEN: RefCell<HashMap<(u32, u32, u8), u8>> = RefCell::new(HashMap::new());
    }
    SEEN.with(|seen| {
        let mut seen = seen.borrow_mut();
        seen.clear();
        let mut bins: [Vec<CorrStep>; 4] = [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
        bins[0].push(CorrStep {
            node: 0,
            pos: 0,
            tier: 0,
            edited: false,
        });
        let mut steps = 0u32;
        loop {
            steps += 1;
            if steps > 5000 {
                break;
            }
            let Some(bucket) = (0..4).find(|&tier| !bins[tier].is_empty()) else {
                break;
            };
            let step = bins[bucket].pop().unwrap();
            let key = (step.node, step.pos, u8::from(step.edited));
            if let Some(&old) = seen.get(&key) {
                if old <= step.tier {
                    continue;
                }
            }
            seen.insert(key, step.tier);
            if step.pos as usize == raw.len() {
                if step.edited && step.node != 0 {
                    note_node(nodes, step.node, 3, 1);
                }
                continue;
            }
            let rest = &raw[step.pos as usize..];
            push_plain_steps(
                lex,
                step.node,
                rest,
                step.pos,
                step.tier,
                step.edited,
                raw.len(),
                &mut bins,
            );
            if !step.edited {
                push_edit_steps(lex, step.node, rest, step.pos, &mut bins);
            }
        }
    });
}

#[allow(clippy::too_many_arguments)] // Bounded search state is kept explicit.
fn push_plain_steps(
    lex: &Lexicon,
    node: u32,
    rest: &str,
    pos: u32,
    tier: u8,
    edited: bool,
    raw_len: usize,
    bins: &mut [Vec<CorrStep>; 4],
) {
    let Some(typed) = rest.as_bytes().first().copied() else {
        return;
    };
    if !typed.is_ascii_lowercase() {
        return;
    }
    for letter in partner_letters(lex.preferences, typed) {
        if letter == 0 {
            continue;
        }
        for edge in lex.edges_for(node, letter) {
            let syl = lex.syl(edge.syl);
            let max_len = rest.len().min(syl.len() + 2);
            for len in 1..=max_len {
                let Some(kind) = piece_match(lex.preferences, &rest[..len], syl) else {
                    continue;
                };
                let next_pos = pos + len as u32;
                let mut next_tier = match kind {
                    Kind::Exact => tier,
                    Kind::Fuzzy => tier.max(3),
                    Kind::Prefix => {
                        if next_pos as usize == raw_len {
                            tier.max(1)
                        } else {
                            tier.max(2)
                        }
                    }
                };
                if edited {
                    next_tier = next_tier.max(3);
                }
                bins[usize::from(next_tier)].push(CorrStep {
                    node: edge.node,
                    pos: next_pos,
                    tier: next_tier,
                    edited,
                });
            }
        }
    }
}

fn push_edit_steps(lex: &Lexicon, node: u32, rest: &str, pos: u32, bins: &mut [Vec<CorrStep>; 4]) {
    let bytes = rest.as_bytes();
    let Some(&first) = bytes.first() else {
        return;
    };
    let mut letters = [0u8; 8];
    let mut count = 0usize;
    let add = |letters: &mut [u8; 8], count: &mut usize, letter: u8| {
        if letter == 0 || *count == letters.len() || letters[..*count].contains(&letter) {
            return;
        }
        letters[*count] = letter;
        *count += 1;
    };
    add(&mut letters, &mut count, first);
    let (row, col) = match key_axis(first) {
        Some(pos) => pos,
        None => return,
    };
    const ROWS: [&[u8]; 3] = [b"qwertyuiop", b"asdfghjkl", b"zxcvbnm"];
    for (dr, dc) in [(0, 1), (0, -1), (1, 0), (-1, 0)] {
        let next_row = row + dr;
        let next_col = col + dc;
        if next_row < 0 || next_col < 0 {
            continue;
        }
        let Some(keys) = ROWS.get(next_row as usize) else {
            continue;
        };
        let Some(&letter) = keys.get(next_col as usize) else {
            continue;
        };
        add(&mut letters, &mut count, letter);
    }
    if let Some(&second) = bytes.get(1) {
        add(&mut letters, &mut count, second);
    }
    let mut seen = HashSet::new();
    for letter in letters[..count].iter().copied() {
        for edge in lex.edges_for(node, letter) {
            if seen.insert(edge.node) {
                consider_edit(lex, edge, rest, pos, bins);
            }
        }
    }
    for edge in lex.edges_all(node) {
        let syl = lex.syl(edge.syl).as_bytes();
        if syl.get(1) == Some(&first) && syl.first() != Some(&first) && seen.insert(edge.node) {
            consider_edit(lex, edge, rest, pos, bins);
        }
    }
}

fn consider_edit(
    lex: &Lexicon,
    edge: &EdgeRec,
    rest: &str,
    pos: u32,
    bins: &mut [Vec<CorrStep>; 4],
) {
    let syl = lex.syl(edge.syl);
    let consumed = if let Some(consumed) = one_edit_full(rest, syl) {
        consumed
    } else if one_edit_open(rest, syl) {
        rest.len()
    } else {
        return;
    };
    bins[3].push(CorrStep {
        node: edge.node,
        pos: pos + consumed as u32,
        tier: 3,
        edited: true,
    });
}

/// `syl` against a prefix of `typed`, using exactly one edit, and consuming
/// the whole syllable. The edit is an adjacent-key substitution, one extra
/// typed letter, or one missing syllable letter.
fn one_edit_full(typed: &str, syl: &str) -> Option<usize> {
    let typed = typed.as_bytes();
    let syl = syl.as_bytes();
    if syl.is_empty() || syl.len() > typed.len() + 1 {
        return None;
    }
    if typed.len() >= syl.len() {
        let window = &typed[..syl.len()];
        let mut diffs = 0usize;
        for index in 0..syl.len() {
            if window[index] != syl[index] {
                diffs += 1;
                if diffs > 1 || !adjacent_key(window[index], syl[index]) {
                    diffs = 2;
                    break;
                }
            }
        }
        if diffs == 1 {
            return Some(syl.len());
        }
    }
    if typed.len() > syl.len() {
        let window = &typed[..syl.len() + 1];
        for skip in 0..window.len() {
            let mut index = 0usize;
            let mut matched = true;
            for (at, &byte) in window.iter().enumerate() {
                if at == skip {
                    continue;
                }
                if index >= syl.len() || byte != syl[index] {
                    matched = false;
                    break;
                }
                index += 1;
            }
            if matched && index == syl.len() {
                return Some(window.len());
            }
        }
    }
    if syl.len() >= 2 && typed.len() >= syl.len() - 1 {
        let need = syl.len() - 1;
        let window = &typed[..need];
        for skip in 0..syl.len() {
            let mut index = 0usize;
            let mut matched = true;
            for (at, &byte) in syl.iter().enumerate() {
                if at == skip {
                    continue;
                }
                if index >= window.len() || window[index] != byte {
                    matched = false;
                    break;
                }
                index += 1;
            }
            if matched && index == need {
                return Some(need);
            }
        }
    }
    None
}

/// The typed letters are a near-miss prefix of `syl` and stop one letter
/// short, so `nihs` can still offer 你好 before the last `o`.
fn one_edit_open(typed: &str, syl: &str) -> bool {
    let typed = typed.as_bytes();
    let syl = syl.as_bytes();
    if typed.len() < 2 || syl.len() != typed.len() + 1 {
        return false;
    }
    let head = &syl[..typed.len()];
    let mut diffs = 0usize;
    for index in 0..typed.len() {
        if typed[index] != head[index] {
            diffs += 1;
            if diffs > 1 || !adjacent_key(typed[index], head[index]) {
                return false;
            }
        }
    }
    diffs == 1
}

fn adjacent_key(left: u8, right: u8) -> bool {
    if left == right {
        return false;
    }
    let (Some((row_a, col_a)), Some((row_b, col_b))) = (key_axis(left), key_axis(right)) else {
        return false;
    };
    let row = (row_a - row_b).abs();
    let col = (col_a - col_b).abs();
    (row == 0 && col == 1) || (row == 1 && col == 0)
}

fn key_axis(letter: u8) -> Option<(i8, i8)> {
    const ROWS: [&[u8]; 3] = [b"qwertyuiop", b"asdfghjkl", b"zxcvbnm"];
    for (row, keys) in ROWS.iter().enumerate() {
        if let Some(col) = keys.iter().position(|&key| key == letter) {
            return Some((row as i8, col as i8));
        }
    }
    None
}

fn note_node(nodes: &mut HashMap<u32, (u8, i64)>, node: u32, tier: u8, divisor: i64) {
    if node == 0 {
        return;
    }
    nodes
        .entry(node)
        .and_modify(|old| {
            if tier < old.0 || (tier == old.0 && divisor < old.1) {
                *old = (tier, divisor);
            }
        })
        .or_insert((tier, divisor));
}

fn concat_eq(lex: &Lexicon, node: u32, raw: &str) -> bool {
    let mut ids = Vec::new();
    let mut current = node;
    while current != 0 {
        ids.push(lex.nodes[current as usize].syl);
        current = lex.nodes[current as usize].parent;
    }
    let mut at = raw.len();
    for id in &ids {
        let syl = lex.syl(*id);
        if syl.len() > at || &raw[at - syl.len()..at] != syl {
            return false;
        }
        at -= syl.len();
    }
    at == 0
}

fn maybe_ambiguous(
    lex: &Lexicon,
    node: u32,
    tier: u8,
    divisor: i64,
    raw: &str,
    full: bool,
    ambiguous: &mut Option<(u32, u32, i64)>,
) {
    if !full || tier != 0 || divisor != 1 || node == 0 {
        return;
    }
    if lex.nodes[node as usize].depth <= 1 || !concat_eq(lex, node, raw) {
        return;
    }
    let mut best: Option<(i64, u32)> = None;
    for word in lex.node_words(node) {
        let replace = best.map(|(score, _)| word.score > score).unwrap_or(true);
        if replace {
            best = Some((word.score, word.text));
        }
    }
    let Some((score, text)) = best else {
        return;
    };
    let replace = ambiguous
        .as_ref()
        .map(|(_, _, old)| score > *old)
        .unwrap_or(true);
    if replace {
        *ambiguous = Some((text, node, score));
    }
}

fn match_lexicon(
    raw: &str,
    lex: &Lexicon,
    nodes: &mut HashMap<u32, (u8, i64)>,
    ambiguous: &mut Option<(u32, u32, i64)>,
) {
    thread_local! {
        static SEEN: RefCell<HashMap<u64, u8>> = RefCell::new(HashMap::new());
    }
    let quoted = raw.contains('\'');
    let parts: Vec<&str> = if quoted {
        raw.split('\'').filter(|part| !part.is_empty()).collect()
    } else {
        Vec::new()
    };
    if quoted && parts.is_empty() {
        return;
    }
    SEEN.with(|seen| {
        let mut seen = seen.borrow_mut();
        seen.clear();
        let mut bins: [Vec<Step>; 4] = [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
        bins[0].push(Step {
            node: 0,
            pos: 0,
            tier: 0,
        });
        while let Some(bucket) = (0..4).find(|&tier| !bins[tier].is_empty()) {
            let step = bins[bucket].pop().unwrap();
            let key = (u64::from(step.node) << 32) | u64::from(step.pos);
            if let Some(&old) = seen.get(&key) {
                if old <= step.tier {
                    continue;
                }
            }
            seen.insert(key, step.tier);
            let done = if quoted {
                step.pos as usize == parts.len()
            } else {
                step.pos as usize == raw.len()
            };
            if done {
                if step.node != 0 {
                    note_node(nodes, step.node, step.tier, 1);
                    maybe_ambiguous(lex, step.node, step.tier, 1, raw, true, ambiguous);
                    if step.tier >= 2 && lex.nodes[step.node as usize].depth >= 2 {
                        for edge in lex.edges_all(step.node) {
                            note_node(nodes, edge.node, step.tier, 4);
                        }
                    }
                }
                continue;
            }
            if quoted {
                let part = parts[step.pos as usize];
                let Some(typed) = part.as_bytes().first().copied() else {
                    continue;
                };
                if !typed.is_ascii_lowercase() {
                    continue;
                }
                let last = step.pos as usize + 1 == parts.len();
                for letter in partner_letters(lex.preferences, typed) {
                    if letter == 0 {
                        continue;
                    }
                    for edge in lex.edges_for(step.node, letter) {
                        let syl = lex.syl(edge.syl);
                        let Some(kind) = piece_match(lex.preferences, part, syl) else {
                            continue;
                        };
                        let next_tier = match kind {
                            Kind::Exact => step.tier,
                            Kind::Prefix => {
                                if last {
                                    step.tier.max(1)
                                } else {
                                    step.tier.max(2)
                                }
                            }
                            Kind::Fuzzy => step.tier.max(3),
                        };
                        bins[next_tier as usize].push(Step {
                            node: edge.node,
                            pos: step.pos + 1,
                            tier: next_tier,
                        });
                    }
                }
            } else {
                let rest = &raw[step.pos as usize..];
                let Some(typed) = rest.as_bytes().first().copied() else {
                    continue;
                };
                if !typed.is_ascii_lowercase() {
                    continue;
                }
                for letter in partner_letters(lex.preferences, typed) {
                    if letter == 0 {
                        continue;
                    }
                    for edge in lex.edges_for(step.node, letter) {
                        let syl = lex.syl(edge.syl);
                        let max_len = rest.len().min(syl.len() + 2);
                        for len in 1..=max_len {
                            let Some(kind) = piece_match(lex.preferences, &rest[..len], syl) else {
                                continue;
                            };
                            let next_pos = step.pos + len as u32;
                            let next_tier = match kind {
                                Kind::Exact => step.tier,
                                Kind::Fuzzy => step.tier.max(3),
                                Kind::Prefix => {
                                    if next_pos as usize == raw.len() {
                                        step.tier.max(1)
                                    } else {
                                        step.tier.max(2)
                                    }
                                }
                            };
                            bins[next_tier as usize].push(Step {
                                node: edge.node,
                                pos: next_pos,
                                tier: next_tier,
                            });
                        }
                    }
                }
            }
        }
    });
}

#[derive(Clone, Copy)]
enum Kind {
    Exact,
    Prefix,
    Fuzzy,
}

fn head_compatible(prefs: crate::InputPreferences, input: &str, syl: &str) -> bool {
    input
        .bytes()
        .next()
        .is_some_and(|b| prefs.heads(b).contains(&syl.as_bytes()[0]))
}

fn piece_match(prefs: crate::InputPreferences, piece: &str, syl: &str) -> Option<Kind> {
    if let Some(kind) = piece_match_plain(prefs, piece, syl) {
        return Some(kind);
    }
    // ü is stored as v (lüe → lve) and, after j/q/x/y, as u (jue). The other
    // spelling is the same syllable: lue, nue, jve, qvan, xve, yvan.
    // Most failed matches cannot be umlaut aliases. Reject before allocating.
    let second = match syl.as_bytes().get(1)? {
        b'u' => b'v',
        b'v' => b'u',
        _ => return None,
    };
    if piece.as_bytes().get(1) != Some(&second) {
        return None;
    }
    let alt = umlaut_alias(syl)?;
    piece_match_plain(prefs, piece, &alt)
}

fn piece_match_plain(prefs: crate::InputPreferences, piece: &str, syl: &str) -> Option<Kind> {
    if piece.is_empty() || syl.is_empty() {
        return None;
    }
    if syl.starts_with(piece) {
        return Some(if piece == syl {
            Kind::Exact
        } else {
            Kind::Prefix
        });
    }
    let initial = |s: &str| {
        if s.starts_with("zh") || s.starts_with("ch") || s.starts_with("sh") {
            2
        } else if s
            .as_bytes()
            .first()
            .is_some_and(|b| b"bpmfdtnlgkhjqxrzcswy".contains(b))
        {
            1
        } else {
            0
        }
    };
    let (pi, si) = (initial(piece), initial(syl));
    let (ph, sh) = (&piece[..pi], &syl[..si]);
    let pair = |a: &str, b: &str, x: &str, y: &str| (a == x && b == y) || (a == y && b == x);
    let head = ph == sh
        || [
            (0, "z", "zh"),
            (1, "c", "ch"),
            (2, "s", "sh"),
            (6, "n", "l"),
            (7, "f", "h"),
            (8, "r", "l"),
        ]
        .iter()
        .any(|&(i, a, b)| prefs.group(i) && pair(ph, sh, a, b));
    if !head {
        return None;
    }
    let (pr, sr) = (&piece[pi..], &syl[si..]);
    if pr == sr {
        return Some(Kind::Fuzzy);
    }
    if prefs.group(9) && pair(pr, sr, "ou", "uo") {
        return Some(Kind::Fuzzy);
    }
    if [(3, "an", "ang"), (4, "en", "eng"), (5, "in", "ing")]
        .iter()
        .any(|&(i, a, b)| {
            prefs.group(i)
                && ((pr
                    .strip_suffix(a)
                    .zip(sr.strip_suffix(b))
                    .is_some_and(|(x, y)| x == y))
                    || (pr
                        .strip_suffix(b)
                        .zip(sr.strip_suffix(a))
                        .is_some_and(|(x, y)| x == y)))
        })
    {
        return Some(Kind::Fuzzy);
    }
    if ph != sh && sr.starts_with(pr) {
        return Some(if pi < si { Kind::Prefix } else { Kind::Fuzzy });
    }
    None
}

/// The other ASCII spelling of a ü syllable. `lu`/`lv` and `nu`/`nv` stay
/// distinct, because both are real syllables.
fn umlaut_alias(syl: &str) -> Option<String> {
    let bytes = syl.as_bytes();
    let swap = |from: u8| -> Option<String> {
        let mut out = bytes.to_vec();
        out[1] = if from == b'u' { b'v' } else { b'u' };
        Some(String::from_utf8(out).unwrap())
    };
    if bytes.len() >= 2
        && matches!(bytes[0], b'j' | b'q' | b'x' | b'y')
        && matches!(bytes[1], b'u' | b'v')
    {
        return swap(bytes[1]);
    }
    if bytes.len() == 3
        && matches!(bytes[0], b'n' | b'l')
        && bytes[2] == b'e'
        && matches!(bytes[1], b'u' | b'v')
    {
        return swap(bytes[1]);
    }
    None
}

/// Match a stored reading against typed letters. A leftover tail is a prefix
/// candidate; an unfinished reading is a prediction and covers the whole input.
pub(crate) fn match_reading(
    prefs: crate::InputPreferences,
    raw: &str,
    reading: &str,
) -> Option<ReadingMatch> {
    match_reading_inner(prefs, raw, reading, true)
}

pub(crate) fn match_imported_reading(
    prefs: crate::InputPreferences,
    raw: &str,
    reading: &str,
) -> Option<ReadingMatch> {
    match_reading_inner(prefs, raw, reading, false)
}

fn match_reading_inner(
    prefs: crate::InputPreferences,
    raw: &str,
    reading: &str,
    allow_prediction: bool,
) -> Option<ReadingMatch> {
    if raw.is_empty() || reading.is_empty() {
        return None;
    }
    if !raw
        .bytes()
        .all(|byte| byte.is_ascii_lowercase() || byte == b'\'')
    {
        return None;
    }
    let syls: Vec<&str> = reading.split('\'').filter(|syl| !syl.is_empty()).collect();
    let head = syls.first()?;
    if !head_compatible(prefs, raw, head) {
        return None;
    }
    if raw.contains('\'') {
        if !allow_prediction && raw.split('\'').filter(|s| !s.is_empty()).count() != syls.len() {
            return None;
        }
        return match_reading_quoted(prefs, raw, &syls);
    }
    let mut best: Option<(usize, u8)> = None;
    walk_reading(prefs, raw, &syls, 0, 0, 0, allow_prediction, &mut best);
    best.filter(|(span, _)| *span > 0)
        .map(|(span, tier)| ReadingMatch { span, tier })
}

#[allow(clippy::too_many_arguments)] // Recursive matching state is kept explicit.
fn walk_reading(
    prefs: crate::InputPreferences,
    raw: &str,
    syls: &[&str],
    syl_index: usize,
    pos: usize,
    tier: u8,
    allow_prediction: bool,
    best: &mut Option<(usize, u8)>,
) {
    if syl_index == syls.len() {
        consider_reading(best, pos, tier, raw.len());
        return;
    }
    if pos == raw.len() {
        if syl_index > 0 && allow_prediction {
            consider_reading(best, raw.len(), tier, raw.len());
        }
        return;
    }
    let rest = &raw[pos..];
    let syl = syls[syl_index];
    if !head_compatible(prefs, rest, syl) {
        return;
    }
    let max_len = rest.len().min(syl.len() + 2);
    for len in 1..=max_len {
        let Some(kind) = piece_match(prefs, &rest[..len], syl) else {
            continue;
        };
        let next_pos = pos + len;
        let next_tier = match kind {
            Kind::Exact => tier,
            Kind::Fuzzy => tier.max(3),
            Kind::Prefix => {
                if next_pos == raw.len() {
                    tier.max(1)
                } else {
                    tier.max(2)
                }
            }
        };
        walk_reading(
            prefs,
            raw,
            syls,
            syl_index + 1,
            next_pos,
            next_tier,
            allow_prediction,
            best,
        );
    }
}

fn consider_reading(best: &mut Option<(usize, u8)>, span: usize, tier: u8, total: usize) {
    let replace = match *best {
        None => true,
        Some((old_span, old_tier)) => {
            if (span == total) != (old_span == total) {
                span == total
            } else {
                tier < old_tier || (tier == old_tier && span > old_span)
            }
        }
    };
    if replace {
        *best = Some((span, tier));
    }
}

fn match_reading_quoted(
    prefs: crate::InputPreferences,
    raw: &str,
    syls: &[&str],
) -> Option<ReadingMatch> {
    let parts: Vec<&str> = raw.split('\'').filter(|part| !part.is_empty()).collect();
    if parts.is_empty() || parts.len() > syls.len() {
        return None;
    }
    let mut tier = 0u8;
    for (index, part) in parts.iter().enumerate() {
        let kind = piece_match(prefs, part, syls[index])?;
        let last = index + 1 == parts.len();
        tier = match kind {
            Kind::Exact => tier,
            Kind::Fuzzy => tier.max(3),
            Kind::Prefix => {
                if last {
                    tier.max(1)
                } else {
                    tier.max(2)
                }
            }
        };
    }
    Some(ReadingMatch {
        span: raw.len(),
        tier,
    })
}

fn specials(raw: &str) -> Vec<String> {
    if !matches!(raw, "rq" | "sj" | "xq" | "jt" | "mt" | "zt") {
        return Vec::new();
    }
    let Some(now) = local_clock() else {
        return Vec::new();
    };
    match raw {
        "rq" => vec![
            format!("{}年{}月{}日", now.year, now.month, now.day),
            format!("{:04}-{:02}-{:02}", now.year, now.month, now.day),
            format!("{}/{}/{}", now.year, now.month, now.day),
        ],
        "sj" => vec![
            format!("{:02}:{:02}", now.hour, now.minute),
            format!("{}时{}分", now.hour, now.minute),
        ],
        "xq" => match now.wday {
            1 => vec!["星期一".to_string()],
            2 => vec!["星期二".to_string()],
            3 => vec!["星期三".to_string()],
            4 => vec!["星期四".to_string()],
            5 => vec!["星期五".to_string()],
            6 => vec!["星期六".to_string()],
            0 => vec!["星期日".to_string()],
            _ => Vec::new(),
        },
        "jt" => day_words(0, "今天"),
        "mt" => day_words(1, "明天"),
        "zt" => day_words(-1, "昨天"),
        _ => Vec::new(),
    }
}

fn day_words(offset: i64, label: &str) -> Vec<String> {
    let Some(now) = local_clock_offset(offset) else {
        return vec![label.to_string()];
    };
    vec![
        label.to_string(),
        format!("{}年{}月{}日", now.year, now.month, now.day),
    ]
}

struct Clock {
    year: i32,
    month: i32,
    day: i32,
    hour: i32,
    minute: i32,
    wday: i32,
}

#[repr(C)]
struct LibcTm {
    sec: i32,
    min: i32,
    hour: i32,
    mday: i32,
    mon: i32,
    year: i32,
    wday: i32,
    yday: i32,
    isdst: i32,
    gmtoff: i64,
    zone: *mut std::ffi::c_void,
}

extern "C" {
    fn time(tloc: *mut i64) -> i64;
    fn localtime_r(timep: *const i64, result: *mut LibcTm) -> *mut LibcTm;
}

fn local_clock() -> Option<Clock> {
    local_clock_offset(0)
}

fn local_clock_offset(days: i64) -> Option<Clock> {
    let mut seconds = 0i64;
    if unsafe { time(&mut seconds) } < 0 {
        return None;
    }
    seconds = seconds.saturating_add(days.saturating_mul(86_400));
    let mut tm = LibcTm {
        sec: 0,
        min: 0,
        hour: 0,
        mday: 0,
        mon: 0,
        year: 0,
        wday: 0,
        yday: 0,
        isdst: 0,
        gmtoff: 0,
        zone: std::ptr::null_mut(),
    };
    if unsafe { localtime_r(&seconds, &mut tm) }.is_null() {
        return None;
    }
    Some(Clock {
        year: tm.year + 1900,
        month: tm.mon + 1,
        day: tm.mday,
        hour: tm.hour,
        minute: tm.min,
        wday: tm.wday,
    })
}

#[cfg(test)]
mod sentence_completion_tests {
    use super::*;

    #[test]
    #[cfg(waypinyin_full_lexicon)]
    fn long_clauses_do_not_become_a_single_grammar_head() {
        let lex = Lexicon::load().with_preferences(crate::InputPreferences {
            contextual: false,
            ..Default::default()
        });
        assert_eq!(
            suggest("woxiangzhidaozhegewentishenmeshihoukeyiwancheng", &lex)
                .first()
                .map(String::as_str),
            Some("我想知道这个问题什么时候可以完成")
        );
    }

    #[test]
    fn equal_text_keeps_token_histories_with_different_future_scores() {
        use crate::language_model::{fingerprint, LanguageModel};
        let token = LanguageModel::token;
        let base = format!(
            "U\t0\t0\t{}\t2\nU\t0\t0\t{}\t2\nU\t0\t0\t{}\t0.1\n",
            token("你"),
            token("好"),
            token("你好")
        );
        let score = |body: &str| {
            let data = format!(
                "WAYPINYIN-LM\t1\t42\t{}\n{body}",
                fingerprint(body.as_bytes())
            );
            let mut model = LanguageModel::parse(data.as_bytes(), 42).unwrap();
            model.weight = 0.9;
            model.prior_weight = 1.0;
            let mut lex = Lexicon::load();
            lex.model = Some(Arc::new(model));
            let rows = compose_sentence_search("nihaozhongguo", &lex, true, false);
            assert_eq!(rows[0].0, "你好中国");
            rows[0].2
        };
        let without = score(&base);
        let with = score(&format!(
            "{base}W\t{}\t{}\t{}\t8\n",
            token("你"),
            token("好"),
            token("中国")
        ));
        assert!(
            with < without - 1.0,
            "different token histories were merged: {with}/{without}"
        );
    }

    #[test]
    #[cfg(waypinyin_full_lexicon)]
    fn sentence_repair_keeps_one_edit_across_word_boundaries() {
        let lex = Lexicon::load().with_test_model();
        let repaired = compose_sentence_search("nihaozhngguoren", &lex, true, true);
        assert!(
            repaired
                .iter()
                .any(|(text, reading, _)| text == "你好中国人"
                    && reading.join("'") == "ni'hao'zhong'guo'ren"),
            "{repaired:?}"
        );
        let twice = compose_sentence_search("nihaozhngguren", &lex, true, true);
        assert!(twice
            .iter()
            .all(|(_, reading, _)| reading.join("'") != "ni'hao'zhong'guo'ren"));
    }

    #[test]
    fn only_the_open_tail_can_complete_a_syllable() {
        // Exercise spelling/boundary mechanics independently of optional
        // language-model preferences for this artificial greeting.
        let lex = Lexicon::load().with_preferences(crate::InputPreferences {
            contextual: false,
            ..Default::default()
        });
        for raw in ["nihaozhongg", "nihaozhonggu", "nihaozhongguo"] {
            let rows = compose_sentences(raw, &lex, true);
            assert_eq!(rows[0].0, "你好中国", "{raw}");
            assert_eq!(rows[0].1.join("'"), "ni'hao'zhong'guo", "{raw}");
        }
        assert!(compose_sentences("nihaozhongg'", &lex, true).is_empty());
        // `gu` is valid in the middle, but must not be stretched to `guo`.
        assert!(compose_sentences("nihaozhongguren", &lex, true)
            .iter()
            .all(|(_, reading, _)| reading.join("'") != "ni'hao'zhong'guo'ren"));
    }
}

#[cfg(test)]
mod preference_tests {
    use super::*;
    #[test]
    fn fuzzy_groups_are_bidirectional_and_independent() {
        for (i, a, b) in [
            (0, "zai", "zhai"),
            (1, "cao", "chao"),
            (2, "san", "shan"),
            (3, "ban", "bang"),
            (4, "ben", "beng"),
            (5, "bin", "bing"),
            (6, "na", "la"),
            (7, "fa", "ha"),
            (8, "rao", "lao"),
        ] {
            let mut prefs = crate::InputPreferences {
                enabled: true,
                groups: [false; 10],
                ..Default::default()
            };
            prefs.groups[i] = true;
            for (typed, reading) in [(a, b), (b, a)] {
                assert!(
                    piece_match(prefs, typed, reading).is_some(),
                    "{typed}/{reading}"
                );
                prefs.enabled = false;
                assert!(!matches!(
                    piece_match(prefs, typed, reading),
                    Some(Kind::Fuzzy)
                ));
                prefs.enabled = true;
            }
        }
        let prefs = crate::InputPreferences::default();
        assert!(matches!(
            piece_match(prefs, "zan", "zhang"),
            Some(Kind::Fuzzy)
        ));
        assert!(matches!(
            piece_match(prefs, "zhang", "zan"),
            Some(Kind::Fuzzy)
        ));
        assert!(piece_match(prefs, "san", "chang").is_none());
        assert_eq!(
            match_imported_reading(prefs, "zhang", "zan").unwrap().span,
            5
        );
        let off = crate::InputPreferences {
            enabled: false,
            ..prefs
        };
        assert!(match_imported_reading(off, "zg", "zhong'guo").is_some());
        assert!(matches!(piece_match(off, "lue", "lve"), Some(Kind::Exact)));
    }
}

#[cfg(test)]
mod ou_uo_tests {
    use super::*;
    #[test]
    fn ou_uo_combines_with_retroflex_and_can_be_disabled() {
        let prefs = crate::InputPreferences::default();
        for a in ["sou", "suo", "shou", "shuo"] {
            for b in ["sou", "suo", "shou", "shuo"] {
                assert!(
                    matches!(piece_match(prefs, a, b), Some(Kind::Exact | Kind::Fuzzy)),
                    "{a}/{b}"
                );
                assert_eq!(match_imported_reading(prefs, a, b).unwrap().span, a.len());
            }
        }
        let mut off = prefs;
        off.groups[9] = false;
        assert!(piece_match(off, "sou", "suo").is_none());
        assert!(piece_match(off, "shuo", "shou").is_none());
        off = prefs;
        off.groups[2] = false;
        assert!(piece_match(off, "sou", "shuo").is_none());
        assert!(matches!(
            piece_match(off, "shou", "shuo"),
            Some(Kind::Fuzzy)
        ));
        off = prefs;
        off.enabled = false;
        assert!(piece_match(off, "sou", "suo").is_none());
        assert!(piece_match(prefs, "shou", "shuang").is_none());
        let lex = Lexicon::load();
        assert!(suggest("sou", &lex).iter().any(|w| w == "说"));
        assert!(suggest("shuo", &lex).iter().any(|w| w == "手"));
    }
}

#[cfg(all(test, waypinyin_full_lexicon))]
#[path = "lexicon_reference.rs"]
mod reference;
