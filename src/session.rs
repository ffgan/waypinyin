//! Composition state. Key handling, paging, and the user dictionary live here.
//! The bus adapter applies [`Effect`] and does not read the session again.

use crate::lexicon::{match_reading, plan, Choice, Lexicon, Plan, Pool};
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::sync::Arc;

pub(crate) const PAGE_SIZE: usize = 5;
const RELEASE_MASK: u32 = 1 << 30;
const CONTROL_MASK: u32 = 1 << 2;
const MOD1_MASK: u32 = 1 << 3;
const SUPER_MASK: u32 = 1 << 26;
const KEY_BACKSPACE: u32 = 0xff08;
const KEY_RETURN: u32 = 0xff0d;
const KEY_ESCAPE: u32 = 0xff1b;
const KEY_KP_ENTER: u32 = 0xff8d;
const KEY_LEFT: u32 = 0xff51;
const KEY_UP: u32 = 0xff52;
const KEY_RIGHT: u32 = 0xff53;
const KEY_DOWN: u32 = 0xff54;
const KEY_PAGE_UP: u32 = 0xff55;
const KEY_PAGE_DOWN: u32 = 0xff56;
const KEY_SPACE: u32 = 0x20;
const KEY_MINUS: u32 = 0x2d;
const KEY_EQUAL: u32 = 0x3d;
const KEY_SHIFT_L: u32 = 0xffe1;
const KEY_SHIFT_R: u32 = 0xffe2;
/// User weight halves once the clock has moved this many commits.
const HALF_LIFE: u32 = 32;
const MAX_USER_WORDS: usize = 4000;
const LISTING_CACHE: usize = 32;
const MAX_FREQUENT_INPUTS: usize = 128;
const PRELOAD_FAVORITES: usize = 16;
const INPUT_HALF_LIFE: u32 = 512;

#[derive(Debug, Clone)]
struct InputCount {
    count: u32,
    tick: u32,
}

fn input_weight(count: &InputCount, now: u32) -> u32 {
    let steps = now.saturating_sub(count.tick) / INPUT_HALF_LIFE;
    if steps >= 31 {
        0
    } else {
        count.count >> steps
    }
}

fn valid_input_pattern(raw: &str) -> bool {
    (1..=128).contains(&raw.len())
        && valid_reading(raw)
        && !raw.starts_with('\'')
        && !raw.ends_with('\'')
        && !raw.contains("''")
        && !matches!(raw, "rq" | "sj" | "xq" | "jt" | "mt" | "zt")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct View {
    pub preedit: String,
    pub candidates: Vec<String>,
    /// How many bytes of `preedit` each candidate consumes.
    pub spans: Vec<usize>,
    pub cursor: usize,
    /// Byte index into `preedit`. The bus draws syllable breaks around it.
    pub caret: usize,
    /// Reading of the highlighted candidate. Empty when it has no spelling.
    pub reading: String,
    pub page_size: usize,
    /// Lookup stays up with an empty preedit while association candidates show.
    pub show_lookup: bool,
}

/// What the host should do after one session event. `Commit.next` is the
/// composition that stays on screen, so the host does not read the session again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    Ignore,
    /// The key belongs to the client, and the lookup changed.
    Forward(View),
    Update(View),
    Commit {
        text: String,
        next: View,
    },
}

#[derive(Debug, Clone)]
struct Entry {
    count: u32,
    tick: u32,
    coined: bool,
}

#[derive(Debug, Clone)]
struct Link {
    count: u32,
    tick: u32,
    reading: String,
}

#[derive(Debug)]
struct Piece {
    reading: String,
    word: String,
}

#[derive(Debug, Default)]
pub struct UserDict {
    tick: u32,
    inputs: HashMap<String, InputCount>,
    /// Reading `ni'hao` → word → decayed-at-`tick` count.
    words: HashMap<String, HashMap<String, Entry>>,
    /// Old logs keyed by the exact letters the user typed.
    legacy: HashMap<String, HashMap<String, u32>>,
    /// Previous commit → following word.
    links: HashMap<String, HashMap<String, Link>>,
    /// Readings grouped by their first letter. Fuzzy initials share a query.
    heads: HashMap<u8, Vec<Arc<str>>>,
    /// Kept open so a commit appends a line instead of opening the file again.
    log: Option<BufWriter<File>>,
}

impl UserDict {
    pub fn load() -> Self {
        let Some(path) = user_dict_path() else {
            return Self::default();
        };
        Self::load_from(&path)
    }

    fn load_from(path: &std::path::Path) -> Self {
        if let Some(dir) = path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        let Ok(mut file) = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(path)
        else {
            return Self::default();
        };
        let _ = file.lock();
        let mut text = String::new();
        let _ = file.read_to_string(&mut text);
        let mut dict = Self::from_log(&text);
        let lines = text.lines().filter(|line| !line.is_empty()).count();
        let overflow = dict.word_count() > MAX_USER_WORDS;
        dict.evict(false);
        let live = dict.live_len();
        if overflow || (lines > 32 && lines > live.saturating_mul(4)) {
            let body = dict.snapshot();
            let tmp = path.with_extension(format!(
                "new-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
            ));
            if let Ok(mut compact) = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&tmp)
            {
                if compact.write_all(body.as_bytes()).is_ok() {
                    let _ = fs::rename(&tmp, path);
                }
                let _ = fs::remove_file(&tmp);
            }
        }
        drop(file);
        if let Ok(appended) = OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(path)
        {
            dict.log = Some(BufWriter::new(appended));
        }
        dict
    }

    pub(crate) fn from_log(text: &str) -> Self {
        let mut dict = Self::default();
        for line in text.lines() {
            if line.is_empty() {
                continue;
            }
            dict.ingest(line);
        }
        dict
    }

    fn ingest(&mut self, line: &str) {
        let cols: Vec<&str> = line.split('\t').collect();
        match cols.as_slice() {
            ["P", tick, raw, count] => {
                let (Ok(tick), Ok(count)) = (tick.parse::<u32>(), count.parse::<u32>()) else {
                    return;
                };
                if valid_input_pattern(raw) && count > 0 {
                    self.fold_input(raw, tick, count);
                    self.trim_inputs(false);
                }
            }
            ["R", raw] => {
                self.inputs.remove(*raw);
            }
            ["U", tick, reading, word, count, coined] => {
                let (Ok(tick), Ok(count)) = (tick.parse::<u32>(), count.parse::<u32>()) else {
                    return;
                };
                if !valid_reading(reading) || word.is_empty() || count == 0 {
                    return;
                }
                self.fold_word(reading, word, tick, count, *coined == "1");
            }
            ["B", tick, prev, next, count, reading] => {
                let (Ok(tick), Ok(count)) = (tick.parse::<u32>(), count.parse::<u32>()) else {
                    return;
                };
                if prev.is_empty() || next.is_empty() || count == 0 {
                    return;
                }
                self.fold_link(prev, next, reading, tick, count);
            }
            ["D", key, word] => {
                self.remove_word(key, word);
                self.remove_legacy(key, word);
            }
            ["X", prev, next] => {
                self.remove_link(prev, next);
            }
            [spelling, word, count] => {
                let Ok(count) = count.parse::<u32>() else {
                    return;
                };
                if spelling.is_empty() || word.is_empty() || count == 0 {
                    return;
                }
                let slot = self.legacy.entry((*spelling).to_string()).or_default();
                let cell = slot.entry((*word).to_string()).or_insert(0);
                *cell = cell.saturating_add(count);
            }
            _ => {}
        }
    }

    fn fold_input(&mut self, raw: &str, tick: u32, count: u32) {
        self.tick = self.tick.max(tick);
        let entry = self
            .inputs
            .entry(raw.into())
            .or_insert(InputCount { count: 0, tick });
        if tick >= entry.tick {
            entry.count = input_weight(entry, tick).saturating_add(count);
            entry.tick = tick;
        } else {
            entry.count = entry.count.saturating_add(count);
        }
    }

    fn record_input(&mut self, raw: &str) {
        if !valid_input_pattern(raw) {
            return;
        }
        let tick = self.tick;
        self.fold_input(raw, tick, 1);
        self.append(&format!("P\t{tick}\t{raw}\t1"));
        self.trim_inputs(true);
    }

    fn trim_inputs(&mut self, persist: bool) {
        while self.inputs.len() > MAX_FREQUENT_INPUTS {
            let raw = self
                .inputs
                .iter()
                .min_by(|(a, x), (b, y)| {
                    input_weight(x, self.tick)
                        .cmp(&input_weight(y, self.tick))
                        .then(x.tick.cmp(&y.tick))
                        .then(a.cmp(b))
                })
                .unwrap()
                .0
                .clone();
            self.inputs.remove(&raw);
            if persist {
                self.append(&format!("R\t{raw}"));
            }
        }
    }

    /// Up to sixteen confirmed, repeated input patterns, most useful first.
    pub fn frequent_inputs(&self) -> Vec<String> {
        let mut rows: Vec<_> = self
            .inputs
            .iter()
            .filter_map(|(raw, entry)| {
                let weight = input_weight(entry, self.tick);
                (weight >= 2).then_some((raw, weight, entry.tick))
            })
            .collect();
        rows.sort_by(|a, b| b.1.cmp(&a.1).then(b.2.cmp(&a.2)).then(a.0.cmp(b.0)));
        rows.into_iter()
            .take(PRELOAD_FAVORITES)
            .map(|(raw, _, _)| raw.clone())
            .collect()
    }

    fn preload_inputs(&self) -> Arc<[String]> {
        let mut favorite = self.frequent_inputs();
        // Older logs already contain useful frequencies, even without P rows.
        let mut older: HashMap<String, u32> = HashMap::new();
        for (reading, words) in &self.words {
            let raw = reading.replace('\'', "");
            if !valid_input_pattern(&raw) {
                continue;
            }
            let count = words
                .values()
                .map(|e| decayed(e.count, e.tick, self.tick))
                .fold(0u32, u32::saturating_add);
            if count >= 2 {
                older
                    .entry(raw)
                    .and_modify(|c| *c = c.saturating_add(count))
                    .or_insert(count);
            }
        }
        for (raw, words) in &self.legacy {
            if !valid_input_pattern(raw) {
                continue;
            }
            let count = words.values().copied().fold(0u32, u32::saturating_add);
            if count >= 2 {
                older
                    .entry(raw.clone())
                    .and_modify(|c| *c = c.saturating_add(count))
                    .or_insert(count);
            }
        }
        let mut older: Vec<_> = older.into_iter().collect();
        older.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        for (raw, _) in older {
            if favorite.len() == PRELOAD_FAVORITES {
                break;
            }
            if !favorite.contains(&raw) {
                favorite.push(raw);
            }
        }
        favorite.into()
    }

    fn fold_word(&mut self, reading: &str, word: &str, tick: u32, count: u32, coined: bool) {
        self.tick = self.tick.max(tick);
        let fresh = !self.words.contains_key(reading);
        let entry = self
            .words
            .entry(reading.to_string())
            .or_default()
            .entry(word.to_string())
            .or_insert(Entry {
                count: 0,
                tick,
                coined: false,
            });
        if tick >= entry.tick {
            let base = decayed(entry.count, entry.tick, tick);
            entry.count = base.saturating_add(count);
            entry.tick = tick;
        } else {
            entry.count = entry.count.saturating_add(count);
        }
        if coined {
            entry.coined = true;
        }
        if fresh {
            self.track_reading(reading);
        }
    }

    fn track_reading(&mut self, reading: &str) {
        let Some(&head) = reading.as_bytes().first() else {
            return;
        };
        let bucket = self.heads.entry(head).or_default();
        if bucket.iter().all(|item| item.as_ref() != reading) {
            bucket.push(Arc::from(reading));
        }
    }

    fn untrack_reading(&mut self, reading: &str) {
        let Some(&head) = reading.as_bytes().first() else {
            return;
        };
        let Some(bucket) = self.heads.get_mut(&head) else {
            return;
        };
        bucket.retain(|item| item.as_ref() != reading);
        if bucket.is_empty() {
            self.heads.remove(&head);
        }
    }

    fn readings_for(&self, raw: &str, prefs: crate::InputPreferences) -> Vec<Arc<str>> {
        let mut found = Vec::new();
        for head in prefs.heads(raw.bytes().next().unwrap_or(0)) {
            if head == 0 {
                continue;
            }
            if let Some(bucket) = self.heads.get(&head) {
                found.extend(bucket.iter().cloned());
            }
        }
        found
    }

    fn fold_link(&mut self, prev: &str, next: &str, reading: &str, tick: u32, count: u32) {
        self.tick = self.tick.max(tick);
        let link = self
            .links
            .entry(prev.to_string())
            .or_default()
            .entry(next.to_string())
            .or_insert(Link {
                count: 0,
                tick,
                reading: reading.to_string(),
            });
        if tick >= link.tick {
            let base = decayed(link.count, link.tick, tick);
            link.count = base.saturating_add(count);
            link.tick = tick;
        } else {
            link.count = link.count.saturating_add(count);
        }
        if !reading.is_empty() {
            link.reading = reading.to_string();
        }
    }

    fn advance(&mut self) -> u32 {
        self.tick = self.tick.saturating_add(1);
        self.tick
    }

    #[cfg(all(test, waypinyin_full_lexicon))]
    fn elapse(&mut self, commits: u32) {
        self.tick = self.tick.saturating_add(commits);
    }

    fn bump(&mut self, now: u32, reading: &str, word: &str, coined: bool) {
        if !valid_reading(reading) || word.is_empty() || reading == word {
            return;
        }
        let fresh = !self.words.contains_key(reading);
        let entry = self
            .words
            .entry(reading.to_string())
            .or_default()
            .entry(word.to_string())
            .or_insert(Entry {
                count: 0,
                tick: now,
                coined: false,
            });
        let base = decayed(entry.count, entry.tick, now);
        entry.count = base.saturating_add(1);
        entry.tick = now;
        if coined {
            entry.coined = true;
        }
        let bit = u8::from(entry.coined);
        if fresh {
            self.track_reading(reading);
        }
        self.append(&format!("U\t{now}\t{reading}\t{word}\t1\t{bit}"));
        self.evict(true);
    }

    fn link(&mut self, now: u32, prev: &str, next: &str, reading: &str) {
        if prev.is_empty() || next.is_empty() {
            return;
        }
        let link = self
            .links
            .entry(prev.to_string())
            .or_default()
            .entry(next.to_string())
            .or_insert(Link {
                count: 0,
                tick: now,
                reading: reading.to_string(),
            });
        let base = decayed(link.count, link.tick, now);
        link.count = base.saturating_add(1);
        link.tick = now;
        if !reading.is_empty() {
            link.reading = reading.to_string();
        }
        let stored = link.reading.clone();
        self.append(&format!("B\t{now}\t{prev}\t{next}\t1\t{stored}"));
    }

    fn forget_word(&mut self, raw: &str, reading: &str, word: &str) {
        if self.remove_word(reading, word) || self.remove_legacy(reading, word) {
            self.append(&format!("D\t{reading}\t{word}"));
        }
        if !raw.is_empty()
            && raw != reading
            && (self.remove_word(raw, word) || self.remove_legacy(raw, word))
        {
            self.append(&format!("D\t{raw}\t{word}"));
        }
    }

    fn forget_link(&mut self, prev: &str, next: &str) {
        if self.remove_link(prev, next) {
            self.append(&format!("X\t{prev}\t{next}"));
        }
    }

    fn remove_word(&mut self, reading: &str, word: &str) -> bool {
        let Some(map) = self.words.get_mut(reading) else {
            return false;
        };
        let removed = map.remove(word).is_some();
        if map.is_empty() {
            self.words.remove(reading);
            self.untrack_reading(reading);
        }
        removed
    }

    fn remove_legacy(&mut self, spelling: &str, word: &str) -> bool {
        let Some(map) = self.legacy.get_mut(spelling) else {
            return false;
        };
        let removed = map.remove(word).is_some();
        if map.is_empty() {
            self.legacy.remove(spelling);
        }
        removed
    }

    fn remove_link(&mut self, prev: &str, next: &str) -> bool {
        let Some(map) = self.links.get_mut(prev) else {
            return false;
        };
        let removed = map.remove(next).is_some();
        if map.is_empty() {
            self.links.remove(prev);
        }
        removed
    }

    fn follow(&self, prev: &str) -> Vec<Choice> {
        let Some(map) = self.links.get(prev) else {
            return Vec::new();
        };
        let mut rows = Vec::new();
        for (word, link) in map {
            let weight = decayed(link.count, link.tick, self.tick);
            // A pair typed once is noise. The bar opens after the same
            // continuation is chosen again.
            if weight < 2 {
                continue;
            }
            rows.push(Choice {
                word: word.clone(),
                span: 0,
                reading: link.reading.clone(),
                tier: 0,
                score: 0,
                user_weight: weight,
            });
        }
        rows.sort_by(|left, right| {
            right
                .user_weight
                .cmp(&left.user_weight)
                .then(left.word.cmp(&right.word))
        });
        rows
    }

    /// Personal rows for this spelling. They lead their tier; the lexicon
    /// stream supplies everything else without being scanned.
    fn overlay(&self, raw: &str, prev: Option<&str>, lex: &Lexicon) -> (Vec<Choice>, Vec<Choice>) {
        let mut full: HashMap<String, Choice> = HashMap::new();
        let mut partials = Vec::new();
        let remember = |full: &mut HashMap<String, Choice>,
                        partials: &mut Vec<Choice>,
                        choice: Choice,
                        complete: bool| {
            if complete {
                full.entry(choice.word.clone())
                    .and_modify(|slot| {
                        if choice.user_weight > slot.user_weight {
                            *slot = choice.clone();
                        } else {
                            slot.user_weight = slot.user_weight.saturating_add(choice.user_weight);
                        }
                    })
                    .or_insert(choice);
            } else if !full.contains_key(&choice.word)
                && !partials.iter().any(|item| item.word == choice.word)
            {
                partials.push(choice);
            }
        };
        for reading in self.readings_for(raw, lex.preferences) {
            let Some(map) = self.words.get(reading.as_ref()) else {
                continue;
            };
            let Some(hit) = match_reading(lex.preferences, raw, reading.as_ref()) else {
                continue;
            };
            for (word, entry) in map {
                let weight = decayed(entry.count, entry.tick, self.tick);
                if weight == 0 {
                    continue;
                }
                let score = lexicon_score(lex, reading.as_ref(), word);
                let choice = Choice {
                    word: word.clone(),
                    span: hit.span,
                    reading: reading.to_string(),
                    tier: hit.tier,
                    score,
                    user_weight: weight,
                };
                remember(&mut full, &mut partials, choice, hit.span >= raw.len());
            }
        }
        if let Some(map) = self.legacy.get(raw) {
            for (word, count) in map {
                let weight = decayed(*count, 0, self.tick);
                if weight == 0 {
                    continue;
                }
                let choice = Choice {
                    word: word.clone(),
                    span: raw.len(),
                    reading: raw.to_string(),
                    tier: 0,
                    score: 0,
                    user_weight: weight,
                };
                remember(&mut full, &mut partials, choice, true);
            }
        }
        if let Some(prev) = prev {
            if let Some(map) = self.links.get(prev) {
                for (word, link) in map {
                    let weight = decayed(link.count, link.tick, self.tick);
                    if weight < 2 || link.reading.is_empty() {
                        continue;
                    }
                    let Some(hit) = match_reading(lex.preferences, raw, &link.reading) else {
                        continue;
                    };
                    if hit.span < raw.len() {
                        continue;
                    }
                    let score = lexicon_score(lex, &link.reading, word);
                    let choice = Choice {
                        word: word.clone(),
                        span: raw.len(),
                        reading: link.reading.clone(),
                        tier: hit.tier,
                        score,
                        user_weight: weight,
                    };
                    if let Some(slot) = full.get_mut(word) {
                        slot.user_weight = slot.user_weight.saturating_add(weight);
                        if slot.score < score {
                            slot.score = score;
                        }
                    } else {
                        full.insert(word.clone(), choice);
                    }
                }
            }
        }
        partials.sort_by(|left, right| {
            right
                .user_weight
                .cmp(&left.user_weight)
                .then(left.word.cmp(&right.word))
        });
        (full.into_values().collect(), partials)
    }

    fn word_count(&self) -> usize {
        self.words.values().map(HashMap::len).sum()
    }

    fn live_len(&self) -> usize {
        self.word_count()
            + self.legacy.values().map(HashMap::len).sum::<usize>()
            + self.links.values().map(HashMap::len).sum::<usize>()
            + self.inputs.len()
    }

    fn evict(&mut self, persist: bool) {
        let extra = self.word_count().saturating_sub(MAX_USER_WORDS);
        if extra == 0 {
            return;
        }
        let mut rows = Vec::new();
        for (reading, map) in &self.words {
            for (word, entry) in map {
                let weight = decayed(entry.count, entry.tick, self.tick);
                rows.push((weight, entry.tick, reading.clone(), word.clone()));
            }
        }
        rows.sort_by(|left, right| {
            left.0
                .cmp(&right.0)
                .then(left.1.cmp(&right.1))
                .then(left.2.cmp(&right.2))
                .then(left.3.cmp(&right.3))
        });
        for (_, _, reading, word) in rows.into_iter().take(extra) {
            self.remove_word(&reading, &word);
            if persist {
                self.append(&format!("D\t{reading}\t{word}"));
            }
        }
    }

    fn snapshot(&self) -> String {
        let mut lines = Vec::new();
        let mut inputs: Vec<_> = self.inputs.iter().collect();
        inputs.sort_by(|a, b| a.0.cmp(b.0));
        for (raw, entry) in inputs {
            lines.push(format!("P\t{}\t{raw}\t{}", entry.tick, entry.count));
        }
        let mut words = Vec::new();
        for (reading, map) in &self.words {
            for (word, entry) in map {
                if entry.count == 0 {
                    continue;
                }
                words.push((reading.as_str(), word.as_str(), entry));
            }
        }
        words.sort_by(|left, right| left.0.cmp(right.0).then(left.1.cmp(right.1)));
        for (reading, word, entry) in words {
            let bit = u8::from(entry.coined);
            lines.push(format!(
                "U\t{}\t{reading}\t{word}\t{}\t{bit}",
                entry.tick, entry.count
            ));
        }
        let mut legacy = Vec::new();
        for (spelling, map) in &self.legacy {
            for (word, count) in map {
                if *count == 0 {
                    continue;
                }
                legacy.push((spelling.as_str(), word.as_str(), *count));
            }
        }
        legacy.sort();
        for (spelling, word, count) in legacy {
            lines.push(format!("{spelling}\t{word}\t{count}"));
        }
        let mut links = Vec::new();
        for (prev, map) in &self.links {
            for (next, link) in map {
                if link.count == 0 {
                    continue;
                }
                links.push((prev.as_str(), next.as_str(), link));
            }
        }
        links.sort_by(|left, right| left.0.cmp(right.0).then(left.1.cmp(right.1)));
        for (prev, next, link) in links {
            lines.push(format!(
                "B\t{}\t{prev}\t{next}\t{}\t{}",
                link.tick, link.count, link.reading
            ));
        }
        if lines.is_empty() {
            String::new()
        } else {
            let mut body = lines.join("\n");
            body.push('\n');
            body
        }
    }

    fn append(&mut self, line: &str) {
        let Some(log) = self.log.as_mut() else {
            return;
        };
        let _ = writeln!(log, "{line}");
        let _ = log.flush();
    }
}

fn decayed(count: u32, tick: u32, now: u32) -> u32 {
    if count == 0 {
        return 0;
    }
    let steps = now.saturating_sub(tick) / HALF_LIFE;
    if steps >= 31 {
        0
    } else {
        count >> steps
    }
}

fn valid_reading(text: &str) -> bool {
    !text.is_empty()
        && text
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte == b'\'')
}

fn user_dict_path() -> Option<PathBuf> {
    let path = crate::paths::data()?.join("user.txt");
    if let Some(home) = std::env::var_os("HOME") {
        let old = PathBuf::from(home).join(".local/share/waypinyin/user.txt");
        if let Err(e) = crate::paths::migrate(&old, &path) {
            eprintln!("waypinyin: cannot migrate learning records: {e}");
            return None;
        }
    }
    Some(path)
}

fn han_word(text: &str) -> bool {
    let mut count = 0usize;
    for ch in text.chars() {
        if !is_han(ch) {
            return false;
        }
        count += 1;
        if count > 8 {
            return false;
        }
    }
    count >= 1
}

fn is_han(ch: char) -> bool {
    matches!(
        ch,
        '\u{3400}'..='\u{4DBF}' | '\u{4E00}'..='\u{9FFF}' | '\u{F900}'..='\u{FAFF}'
    )
}

#[derive(Debug)]
pub struct Session {
    raw: String,
    /// Byte index into `raw` where the next letter is inserted.
    caret: usize,
    cursor: usize,
    lexicon: Arc<Lexicon>,
    user: UserDict,
    imported: crate::imports::Catalog,
    /// Materialized pages for the current preedit or association list.
    board: Option<Board>,
    /// Node lists for recent spellings. Pages are merged from these.
    plans: HashMap<String, Arc<Plan>>,
    preloader: Option<Arc<crate::preload::Preloader>>,
    frequent: Arc<[String]>,
    learning_enabled: bool,
    learn_composition: bool,
    /// Pieces already committed from the current composition.
    pending: Vec<Piece>,
    /// Last committed Han word. The next commit is learned as its follower.
    context: Option<String>,
    /// Reading of `context`, used to find longer lexicon words.
    context_reading: Option<String>,
    surrounding_context: Option<String>,
    local_context: String,
    /// Post-commit association list. Kept off so the lookup closes with the word.
    associating: bool,
    /// The next `"` is an opening quote.
    quote_open: bool,
    preferences_source: Option<crate::preferences::PreferencesSource>,
}

impl Session {
    #[cfg(all(test, waypinyin_full_lexicon))]
    pub fn new(lexicon: Arc<Lexicon>) -> Self {
        Self::with_user(lexicon, UserDict::default())
    }

    pub fn with_user(lexicon: Arc<Lexicon>, user: UserDict) -> Self {
        let source = crate::preferences::PreferencesSource::load();
        let mut session = Self::with_preferences(lexicon, user, source.current);
        session.preferences_source = Some(source);
        session.preload();
        session
    }

    /// Creates a session with fixed preferences; this constructor does not reload disk settings.
    pub fn with_preferences(
        lexicon: Arc<Lexicon>,
        user: UserDict,
        preferences: crate::InputPreferences,
    ) -> Self {
        let lexicon = Arc::new(lexicon.with_preferences(preferences));
        let frequent = user.preload_inputs();
        Self {
            raw: String::new(),
            caret: 0,
            cursor: 0,
            lexicon,
            user,
            imported: crate::imports::Catalog::load(),
            board: None,
            plans: HashMap::new(),
            preloader: None,
            frequent,
            learning_enabled: true,
            learn_composition: true,
            pending: Vec::new(),
            context: None,
            context_reading: None,
            surrounding_context: None,
            local_context: String::new(),
            associating: false,
            quote_open: true,
            preferences_source: None,
        }
    }

    /// Request dictionary-only first pages on the idle worker. Returns whether
    /// this composition is ready (or all single letters when the input is empty).
    /// This never waits for a page and does not cache personal/imported ranking.
    pub fn preload(&mut self) -> bool {
        if self.raw.is_empty() {
            self.learn_composition = self.learning_enabled;
        }
        if !self.can_learn() {
            return false;
        }
        if self.preloader.is_none() {
            self.preloader = crate::preload::Preloader::shared(self.lexicon.clone());
        }
        self.schedule_preload(None);
        self.preloader
            .as_ref()
            .is_some_and(|p| p.ready(&self.raw, self.lexicon.preferences))
    }

    pub fn learning_enabled(&self) -> bool {
        self.learning_enabled
    }

    pub fn surrounding_enabled(&self) -> bool {
        self.can_learn() && self.lexicon.context_enabled() && self.lexicon.preferences.surrounding
    }

    /// IBus positions count Unicode characters, not UTF-8 bytes. App text is
    /// ephemeral and never enters the learning journal or speculative worker.
    pub fn set_surrounding_text(&mut self, text: &str, cursor: u32, anchor: u32) {
        if !self.surrounding_enabled() {
            return;
        }
        if text.len() > 65536 {
            self.clear_ranking_context();
            return;
        }
        let count = text.chars().count();
        if cursor != anchor || cursor as usize > count || anchor as usize > count {
            self.clear_ranking_context();
            return;
        }
        let next = context_tail(text.chars().take(cursor as usize));
        let moved = self
            .surrounding_context
            .as_ref()
            .is_some_and(|old| old != &next)
            || (self.surrounding_context.is_none()
                && self
                    .context
                    .as_ref()
                    .is_some_and(|last| !next.ends_with(last)));
        if moved {
            // An external edit/cursor move is not an adjacent personal choice.
            // Do not learn a link or coin a word across those locations.
            self.context = None;
            self.context_reading = None;
            self.local_context.clear();
            self.pending.clear();
            self.invalidate();
        }
        self.surrounding_context = Some(next);
        self.refresh_ranking_context();
    }

    pub fn clear_ranking_context(&mut self) {
        self.surrounding_context = None;
        self.local_context.clear();
        self.context = None;
        self.context_reading = None;
        self.refresh_ranking_context();
        self.invalidate();
    }

    fn refresh_ranking_context(&mut self) {
        let text = if self.can_learn() && self.lexicon.context_enabled() {
            if self.lexicon.preferences.surrounding {
                self.surrounding_context
                    .as_deref()
                    .unwrap_or(&self.local_context)
            } else {
                &self.local_context
            }
        } else {
            ""
        };
        if self.lexicon.ranking_context() == text {
            return;
        }
        self.cursor = 0;
        self.lexicon = Arc::new(self.lexicon.with_context(text));
        self.plans.clear();
        self.invalidate();
    }

    /// Hosts should disable learning for private/password input contexts.
    pub fn set_learning_enabled(&mut self, enabled: bool) {
        if self.learning_enabled == enabled {
            return;
        }
        self.learning_enabled = enabled;
        if !enabled {
            self.learn_composition = false;
            self.pending.clear();
            self.context = None;
            self.context_reading = None;
            self.clear_ranking_context();
        } else if self.raw.is_empty() {
            self.learn_composition = true;
        }
        self.plans.clear();
    }

    fn can_learn(&self) -> bool {
        self.learning_enabled && self.learn_composition
    }

    fn record_input(&mut self, raw: &str) {
        if !self.can_learn() || !valid_input_pattern(raw) {
            return;
        }
        self.user.record_input(raw);
        self.frequent = self.user.preload_inputs();
    }

    fn schedule_preload(&self, reading: Option<&str>) {
        if self.lexicon.context_enabled()
            && (!self.local_context.is_empty()
                || self
                    .surrounding_context
                    .as_ref()
                    .is_some_and(|s| !s.is_empty()))
        {
            return;
        }
        if !self.can_learn() {
            return;
        }
        let Some(preloader) = &self.preloader else {
            return;
        };
        let mut hints = [0u8; 5];
        let mut count = 0;
        let mut remember = |reading: &str| {
            let mut letters = reading.bytes().filter(|b| *b != b'\'');
            if self.caret == self.raw.len()
                && self
                    .raw
                    .bytes()
                    .filter(|b| *b != b'\'')
                    .all(|b| letters.next() == Some(b))
            {
                if let Some(c) = letters.next().filter(u8::is_ascii_lowercase) {
                    if count < hints.len() && !hints[..count].contains(&c) {
                        hints[count] = c;
                        count += 1;
                    }
                }
            }
        };
        if let Some(reading) = reading {
            remember(reading);
        }
        if let Some(board) = &self.board {
            for choice in board.out.iter().take(PAGE_SIZE) {
                remember(&choice.reading);
            }
        }
        preloader.request(
            &self.raw,
            self.caret,
            self.lexicon.preferences,
            self.plans.get(&self.raw).map(Arc::downgrade),
            hints,
            self.frequent.clone(),
        );
    }

    #[cfg(all(test, waypinyin_full_lexicon))]
    pub(crate) fn set_imported(&mut self, records: Vec<crate::imports::Record>) {
        self.imported = crate::imports::Catalog::from_records(records);
    }

    #[cfg(all(test, waypinyin_full_lexicon))]
    pub(crate) fn elapse(&mut self, commits: u32) {
        self.user.elapse(commits);
        self.invalidate();
    }

    pub fn clear(&mut self) {
        self.clear_ranking_context();
        self.raw.clear();
        self.learn_composition = self.learning_enabled;
        self.caret = 0;
        self.cursor = 0;
        self.board = None;
        self.pending.clear();
        self.context = None;
        self.context_reading = None;
        self.associating = false;
    }

    fn wipe_preedit(&mut self) {
        self.raw.clear();
        self.learn_composition = self.learning_enabled;
        self.caret = 0;
        self.cursor = 0;
        self.invalidate();
    }

    fn invalidate(&mut self) {
        self.board = None;
    }

    fn recall(&mut self, raw: &str) -> Arc<Plan> {
        if let Some(found) = self.plans.get(raw) {
            return found.clone();
        }
        let planned = Arc::new(plan(raw, &self.lexicon));
        if self.plans.len() >= LISTING_CACHE {
            self.plans.clear();
        }
        self.plans.insert(raw.to_string(), planned.clone());
        planned
    }

    fn fill(&mut self) {
        if self.board.is_some() {
            return;
        }
        if self.raw.is_empty() {
            let list = if self.associating {
                self.prediction()
            } else {
                Vec::new()
            };
            if list.is_empty() {
                self.associating = false;
            }
            self.board = Some(Board::fixed(list));
            return;
        }
        let raw = self.raw.clone();
        let prepared = self
            .preloader
            .as_ref()
            .filter(|_| {
                !self.lexicon.context_enabled()
                    || (self.local_context.is_empty()
                        && self
                            .surrounding_context
                            .as_ref()
                            .is_none_or(String::is_empty))
            })
            .and_then(|p| p.get(&raw, self.lexicon.preferences));
        let (mut users, partials) = self
            .user
            .overlay(&raw, self.context.as_deref(), &self.lexicon);
        users.extend(
            self.imported
                .choices_with_preferences(&raw, self.lexicon.preferences),
        );
        if let Some(ready) = &prepared {
            if !self.plans.contains_key(&raw) {
                if self.plans.len() >= LISTING_CACHE {
                    self.plans.clear();
                }
                self.plans.insert(raw.clone(), ready.plan());
            }
        }
        if users.is_empty() && partials.is_empty() {
            if let Some(prepared) = prepared {
                self.board = Some(Board::prepared(prepared));
                return;
            }
        }
        let planned = prepared
            .as_ref()
            .map_or_else(|| self.recall(&raw), |p| p.plan());
        let pool = Pool::open(planned, &self.lexicon, users, partials);
        self.board = Some(Board::paged(pool));
    }

    fn ensure_through(&mut self, end: usize) {
        let Some(board) = self.board.as_mut() else {
            return;
        };
        let lex = Arc::clone(&self.lexicon);
        board.ensure(end, &lex);
    }

    fn ensure_window(&mut self) {
        let end = window_end(self.cursor);
        self.ensure_through(end);
    }

    fn choice(&mut self, index: usize) -> Option<Choice> {
        self.fill();
        self.ensure_through(index + 1);
        self.board.as_ref()?.out.get(index).cloned()
    }

    pub fn view(&mut self) -> View {
        self.fill();
        self.ensure_window();
        let list = &self.board.as_ref().unwrap().out;
        let cursor = if list.is_empty() {
            0
        } else {
            self.cursor.min(list.len() - 1)
        };
        let show_lookup = !self.raw.is_empty() || (self.associating && !list.is_empty());
        let reading = list
            .get(cursor)
            .map(|choice| choice.reading.clone())
            .unwrap_or_default();
        View {
            preedit: self.raw.clone(),
            candidates: list.iter().map(|choice| choice.word.clone()).collect(),
            spans: list.iter().map(|choice| choice.span).collect(),
            cursor,
            caret: self.caret.min(self.raw.len()),
            reading,
            page_size: PAGE_SIZE,
            show_lookup,
        }
    }

    /// Personal repeats first, then the lexicon's longer words. A pair typed
    /// once stays out; `北京` still offers `大学` from the dictionary.
    fn prediction(&self) -> Vec<Choice> {
        let Some(prev) = self.context.as_deref() else {
            return Vec::new();
        };
        let mut list = self.user.follow(prev);
        list.truncate(3);
        let Some(reading) = self.context_reading.as_deref() else {
            return list;
        };
        for (suffix, suffix_reading, score) in self.lexicon.extensions(prev, reading) {
            if list.iter().any(|choice| choice.word == suffix) {
                continue;
            }
            list.push(Choice {
                word: suffix,
                span: 0,
                reading: suffix_reading,
                tier: 0,
                score,
                user_weight: 0,
            });
            if list.len() >= PAGE_SIZE {
                break;
            }
        }
        list
    }

    /// Drop the composition and report an empty preedit.
    pub fn reset(&mut self) -> Effect {
        self.clear();
        Effect::Update(self.view())
    }

    /// Commit the candidate at `index` in the full list, not the current page.
    pub fn pick(&mut self, index: usize) -> Effect {
        if index >= self.view().candidates.len() {
            return Effect::Update(self.view());
        }
        self.commit_at(index)
    }

    /// Commit the highlighted candidate and any spelling it did not cover.
    /// Focus loss uses this so a preedit is not discarded. Nothing is learned:
    /// the commit was not a choice the user made.
    pub fn commit_preedit(&mut self) -> Effect {
        self.context = None;
        self.context_reading = None;
        self.associating = false;
        self.pending.clear();
        if self.raw.is_empty() {
            self.invalidate();
            return Effect::Update(self.view());
        }
        let index = self.view().cursor;
        let text = self
            .pieces_from(index)
            .into_iter()
            .map(|choice| choice.word)
            .collect::<String>();
        self.wipe_preedit();
        self.pending.clear();
        self.finish(text)
    }

    pub fn page(&mut self, direction: isize) -> Effect {
        self.page_by(direction)
    }

    fn finish(&mut self, text: String) -> Effect {
        if self.raw.is_empty() {
            self.learn_composition = self.learning_enabled;
        }
        let next = self.view();
        self.schedule_preload(Some(&next.reading));
        Effect::Commit { text, next }
    }

    fn page_by(&mut self, direction: isize) -> Effect {
        self.fill();
        if direction < 0 {
            self.cursor = self.cursor.saturating_sub(PAGE_SIZE);
        } else {
            let next = self.cursor + PAGE_SIZE;
            self.cursor = next;
            self.ensure_window();
            let loaded = self
                .board
                .as_ref()
                .map(|board| board.out.len())
                .unwrap_or(0);
            if next >= loaded {
                self.cursor = next - PAGE_SIZE;
            }
        }
        Effect::Update(self.view())
    }

    pub fn handle(&mut self, keyval: u32, state: u32) -> Effect {
        let effect = self.handle_inner(keyval, state);
        let view = match &effect {
            Effect::Update(v) | Effect::Forward(v) => Some(v),
            Effect::Commit { .. } | Effect::Ignore => None,
        };
        if let Some(view) = view {
            self.schedule_preload(Some(&view.reading));
        }
        effect
    }

    fn handle_inner(&mut self, keyval: u32, state: u32) -> Effect {
        // Shift remains a modifier; it never changes the input mode or preedit.
        if keyval == KEY_SHIFT_L || keyval == KEY_SHIFT_R {
            return Effect::Ignore;
        }
        if state & RELEASE_MASK != 0 || state & (MOD1_MASK | SUPER_MASK) != 0 {
            return Effect::Ignore;
        }
        if state & CONTROL_MASK != 0 {
            return self.forget_chord(keyval);
        }
        if self.raw.is_empty() && (0x61..=0x7a).contains(&keyval) {
            self.imported.refresh();
            if let Some(prefs) = self
                .preferences_source
                .as_mut()
                .and_then(|source| source.refresh())
            {
                self.lexicon = Arc::new(self.lexicon.with_preferences(prefs));
                if !prefs.contextual || !prefs.surrounding {
                    self.clear_ranking_context();
                } else {
                    self.refresh_ranking_context();
                }
                self.plans.clear();
                self.invalidate();
            }
        }
        if self.raw.is_empty() {
            return self.handle_idle(keyval);
        }
        self.handle_composing(keyval)
    }

    fn forget_chord(&mut self, keyval: u32) -> Effect {
        if !self.can_learn() {
            return Effect::Ignore;
        }
        let Some(digit) = page_digit(keyval) else {
            return Effect::Ignore;
        };
        let view = self.view();
        let index = (view.cursor / PAGE_SIZE) * PAGE_SIZE + digit;
        let Some(choice) = self.choice(index) else {
            return Effect::Ignore;
        };
        if choice.user_weight == 0 {
            return Effect::Ignore;
        }
        if self.associating {
            if let Some(prev) = self.context.clone() {
                self.user.forget_link(&prev, &choice.word);
            }
        } else {
            self.user
                .forget_word(&self.raw, &choice.reading, &choice.word);
        }
        self.invalidate();
        Effect::Update(self.view())
    }

    fn handle_idle(&mut self, keyval: u32) -> Effect {
        if self.associating {
            return self.handle_association(keyval);
        }
        match key_char(keyval) {
            Some(ch) if ch.is_ascii_lowercase() => {
                self.insert_char(ch);
                Effect::Update(self.view())
            }
            Some(ch) => match self.map_punct(ch, false) {
                Some(punct) => {
                    if ends_sentence(punct) {
                        self.close_sentence();
                    }
                    self.finish(punct.to_string())
                }
                None => Effect::Ignore,
            },
            _ => Effect::Ignore,
        }
    }

    fn handle_association(&mut self, keyval: u32) -> Effect {
        match keyval {
            KEY_ESCAPE => {
                self.associating = false;
                self.invalidate();
                Effect::Update(self.view())
            }
            KEY_BACKSPACE => {
                self.associating = false;
                self.context = None;
                self.context_reading = None;
                self.invalidate();
                Effect::Forward(self.view())
            }
            KEY_SPACE => {
                self.associating = false;
                self.invalidate();
                Effect::Forward(self.view())
            }
            0x31..=0x39 => {
                let Some(digit) = page_digit(keyval) else {
                    return Effect::Update(self.view());
                };
                let view = self.view();
                let index = (view.cursor / PAGE_SIZE) * PAGE_SIZE + digit;
                if index < view.candidates.len() {
                    self.commit_at(index)
                } else {
                    Effect::Update(view)
                }
            }
            KEY_PAGE_UP | KEY_MINUS => self.page_by(-1),
            KEY_PAGE_DOWN | KEY_EQUAL => self.page_by(1),
            KEY_UP => self.move_candidate(-1),
            KEY_DOWN => self.move_candidate(1),
            KEY_LEFT | KEY_RIGHT => Effect::Update(self.view()),
            _ => match key_char(keyval) {
                Some(ch) if ch.is_ascii_lowercase() => {
                    self.associating = false;
                    self.insert_char(ch);
                    Effect::Update(self.view())
                }
                Some(ch) => match self.map_punct(ch, false) {
                    Some(punct) => {
                        if ends_sentence(punct) {
                            self.close_sentence();
                        } else {
                            self.associating = false;
                        }
                        self.invalidate();
                        self.finish(punct.to_string())
                    }
                    None => Effect::Ignore,
                },
                _ => Effect::Ignore,
            },
        }
    }

    fn handle_composing(&mut self, keyval: u32) -> Effect {
        match keyval {
            KEY_BACKSPACE => {
                self.delete_before_caret();
                Effect::Update(self.view())
            }
            KEY_ESCAPE => {
                self.pending.clear();
                self.associating = false;
                self.wipe_preedit();
                Effect::Update(self.view())
            }
            KEY_RETURN | KEY_KP_ENTER => {
                let raw = std::mem::take(&mut self.raw);
                self.caret = 0;
                self.cursor = 0;
                self.pending.clear();
                self.context = None;
                self.context_reading = None;
                self.associating = false;
                self.invalidate();
                self.finish(raw)
            }
            KEY_SPACE => {
                let cursor = self.view().cursor;
                self.commit_at(cursor)
            }
            0x31..=0x39 => {
                let Some(digit) = page_digit(keyval) else {
                    return Effect::Update(self.view());
                };
                let view = self.view();
                let index = (view.cursor / PAGE_SIZE) * PAGE_SIZE + digit;
                if index < view.candidates.len() {
                    self.commit_at(index)
                } else {
                    Effect::Update(view)
                }
            }
            KEY_PAGE_UP | KEY_MINUS => self.page_by(-1),
            KEY_PAGE_DOWN | KEY_EQUAL => self.page_by(1),
            KEY_UP => self.move_candidate(-1),
            KEY_DOWN => self.move_candidate(1),
            KEY_LEFT => {
                if self.caret > 0 {
                    self.caret -= 1;
                }
                Effect::Update(self.view())
            }
            KEY_RIGHT => {
                if self.caret < self.raw.len() {
                    self.caret += 1;
                }
                Effect::Update(self.view())
            }
            _ => match key_char(keyval) {
                Some(ch) if ch.is_ascii_lowercase() || ch == '\'' => {
                    self.insert_char(ch);
                    Effect::Update(self.view())
                }
                Some(ch) => match self.map_punct(ch, true) {
                    Some(punct) => {
                        let cursor = self.view().cursor;
                        self.commit_pieces(cursor, Some(punct))
                    }
                    None => Effect::Ignore,
                },
                _ => Effect::Ignore,
            },
        }
    }

    fn map_punct(&mut self, ch: char, composing: bool) -> Option<char> {
        if ch == '"' {
            let punct = if self.quote_open { '“' } else { '”' };
            self.quote_open = !self.quote_open;
            return Some(punct);
        }
        if !composing && (ch == '<' || ch == '>') {
            return None;
        }
        chinese_punct(ch)
    }

    fn insert_char(&mut self, ch: char) {
        if self.raw.is_empty() {
            self.learn_composition = self.learning_enabled;
        }
        let at = self.caret.min(self.raw.len());
        self.raw.insert(at, ch);
        self.caret = at + ch.len_utf8();
        self.cursor = 0;
        self.invalidate();
    }

    fn delete_before_caret(&mut self) {
        if self.caret == 0 || self.raw.is_empty() {
            return;
        }
        let at = self.caret.min(self.raw.len());
        let drop_at = at - 1;
        self.raw.remove(drop_at);
        self.caret = drop_at;
        self.cursor = 0;
        if self.raw.is_empty() {
            self.pending.clear();
            self.caret = 0;
        }
        self.invalidate();
    }

    fn move_candidate(&mut self, delta: isize) -> Effect {
        self.fill();
        if delta > 0 {
            self.ensure_through(window_end(self.cursor + 1));
        }
        let len = self
            .board
            .as_ref()
            .map(|board| board.out.len())
            .unwrap_or(0);
        if len == 0 {
            return Effect::Update(self.view());
        }
        let current = self.cursor.min(len - 1) as isize;
        let next = (current + delta).clamp(0, len as isize - 1) as usize;
        self.cursor = next;
        Effect::Update(self.view())
    }

    fn commit_at(&mut self, index: usize) -> Effect {
        if self.raw.is_empty() {
            return self.commit_association(index);
        }
        let Some(choice) = self.choice(index) else {
            return Effect::Update(self.view());
        };
        let span = choice.span.min(self.raw.len());
        if span == 0 {
            return Effect::Update(self.view());
        }
        let word = choice.word.clone();
        let done = span >= self.raw.len();
        self.note(&choice, done);
        if choice.score > 0 {
            let input = self.raw[..span].trim_end_matches('\'').to_string();
            self.record_input(&input);
        }
        if done {
            self.wipe_preedit();
        } else {
            self.raw.replace_range(..span, "");
            self.caret = self.raw.len();
            self.cursor = 0;
            self.associating = false;
            self.invalidate();
        }
        self.finish(word)
    }

    fn commit_association(&mut self, index: usize) -> Effect {
        let Some(choice) = self.choice(index) else {
            return Effect::Update(self.view());
        };
        let word = choice.word.clone();
        if !self.can_learn() {
            self.pending.clear();
            self.associating = false;
            self.invalidate();
            return self.finish(word);
        }
        let now = self.user.advance();
        if !choice.reading.is_empty() && han_word(&choice.word) {
            self.user.bump(now, &choice.reading, &choice.word, false);
        }
        self.tie(now, &choice);
        self.pending.clear();
        self.associating = false;
        self.invalidate();
        self.finish(word)
    }

    fn commit_pieces(&mut self, index: usize, punct: Option<char>) -> Effect {
        let pieces = self.pieces_from(index);
        let confirmed_input =
            (self.can_learn() && !pieces.is_empty() && pieces.iter().all(|c| c.score > 0))
                .then(|| self.raw.clone());
        let mut text: String = pieces.iter().map(|choice| choice.word.as_str()).collect();
        if let Some(ch) = punct {
            text.push(chinese_punct(ch).unwrap_or(ch));
        }
        let learnable: Vec<Choice> = pieces
            .iter()
            .filter(|choice| {
                self.can_learn() && !choice.reading.is_empty() && han_word(&choice.word)
            })
            .cloned()
            .collect();
        if learnable.is_empty() {
            self.pending.clear();
            self.associating = false;
        } else {
            let now = self.user.advance();
            for choice in &learnable {
                self.user.bump(now, &choice.reading, &choice.word, false);
                self.pending.push(Piece {
                    reading: choice.reading.clone(),
                    word: choice.word.clone(),
                });
                self.tie(now, choice);
            }
            self.seal(now);
            self.associating = false;
            if punct.is_some_and(ends_sentence) {
                self.close_sentence();
            }
        }
        if let Some(input) = confirmed_input {
            self.record_input(&input);
        }
        self.wipe_preedit();
        self.finish(text)
    }

    fn note(&mut self, choice: &Choice, done: bool) {
        if !self.can_learn() || choice.reading.is_empty() || !han_word(&choice.word) {
            if done {
                self.pending.clear();
                self.context = None;
                self.context_reading = None;
                self.associating = false;
            }
            return;
        }
        let now = self.user.advance();
        self.user.bump(now, &choice.reading, &choice.word, false);
        self.pending.push(Piece {
            reading: choice.reading.clone(),
            word: choice.word.clone(),
        });
        self.tie(now, choice);
        if done {
            self.seal(now);
            self.associating = false;
        }
    }

    fn tie(&mut self, now: u32, choice: &Choice) {
        if let Some(prev) = self.context.clone() {
            if prev != choice.word {
                self.user.link(now, &prev, &choice.word, &choice.reading);
            }
        }
        self.context = Some(choice.word.clone());
        self.context_reading = if choice.reading.is_empty() {
            None
        } else {
            Some(choice.reading.clone())
        };
        self.local_context.push_str(&choice.word);
        self.local_context = context_tail(self.local_context.chars());
        if let Some(context) = &mut self.surrounding_context {
            context.push_str(&choice.word);
            *context = context_tail(context.chars());
        }
        self.refresh_ranking_context();
    }

    fn seal(&mut self, now: u32) {
        if self.pending.len() >= 2 {
            let word: String = self
                .pending
                .iter()
                .map(|piece| piece.word.as_str())
                .collect();
            if han_word(&word) && word.chars().count() >= 2 {
                let reading = self
                    .pending
                    .iter()
                    .map(|piece| piece.reading.as_str())
                    .collect::<Vec<_>>()
                    .join("'");
                self.user.bump(now, &reading, &word, true);
            }
        }
        self.pending.clear();
    }

    fn pieces_from(&mut self, index: usize) -> Vec<Choice> {
        let Some(choice) = self.choice(index) else {
            return vec![bare(self.raw.clone(), self.raw.len())];
        };
        let span = choice.span.min(self.raw.len());
        if span >= self.raw.len() {
            return vec![choice];
        }
        let rest = self.raw[span..].to_string();
        let mut pieces = vec![choice];
        let prev = pieces[0].word.clone();
        let planned = self.recall(&rest);
        let (users, partials) = self.user.overlay(&rest, Some(prev.as_str()), &self.lexicon);
        let mut pool = Pool::open(planned, &self.lexicon, users, partials);
        if let Some(next) = pool.next(&self.lexicon) {
            pieces.push(next);
        } else if !rest.is_empty() {
            pieces.push(bare(rest.clone(), rest.len()));
        }
        pieces
    }

    fn close_sentence(&mut self) {
        self.clear_ranking_context();
        self.context = None;
        self.context_reading = None;
        self.associating = false;
    }

    #[cfg(all(test, waypinyin_full_lexicon))]
    pub(crate) fn loaded(&mut self) -> usize {
        self.fill();
        self.ensure_window();
        self.board
            .as_ref()
            .map(|board| board.out.len())
            .unwrap_or(0)
    }
}

fn bare(word: String, span: usize) -> Choice {
    Choice {
        word,
        span,
        reading: String::new(),
        tier: 0,
        score: 0,
        user_weight: 0,
    }
}

fn page_digit(keyval: u32) -> Option<usize> {
    if !(0x31..=0x39).contains(&keyval) {
        return None;
    }
    let digit = (keyval - 0x31) as usize;
    if digit >= PAGE_SIZE {
        None
    } else {
        Some(digit)
    }
}

fn key_char(keyval: u32) -> Option<char> {
    match keyval {
        0x20..=0x7e => char::from_u32(keyval),
        0xff0e => Some('。'),
        0xff0c => Some('，'),
        _ => None,
    }
}

fn ends_sentence(ch: char) -> bool {
    matches!(ch, '。' | '！' | '？' | '；' | '…')
}

fn window_end(cursor: usize) -> usize {
    let page = cursor / PAGE_SIZE;
    (page + 1) * PAGE_SIZE
}

fn lexicon_score(lex: &Lexicon, reading: &str, word: &str) -> i64 {
    let mut node = 0u32;
    for syl in reading.split('\'') {
        if syl.is_empty() {
            return 0;
        }
        match lex.walk_public(node, syl) {
            Some(next) => node = next,
            None => return 0,
        }
    }
    lex.score_of(node, word)
}

struct Board {
    pool: Option<Pool>,
    deferred: Option<Arc<crate::lexicon::Prepared>>,
    out: Arc<Vec<Choice>>,
    done: bool,
}

impl std::fmt::Debug for Board {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Board")
            .field("loaded", &self.out.len())
            .field("done", &self.done)
            .finish()
    }
}

impl Board {
    fn fixed(out: Vec<Choice>) -> Self {
        Self {
            pool: None,
            deferred: None,
            out: Arc::new(out),
            done: true,
        }
    }

    fn paged(pool: Pool) -> Self {
        Self {
            pool: Some(pool),
            deferred: None,
            out: Arc::new(Vec::new()),
            done: false,
        }
    }

    fn prepared(ready: Arc<crate::lexicon::Prepared>) -> Self {
        Self {
            pool: None,
            out: ready.out.clone(),
            done: ready.done,
            deferred: Some(ready),
        }
    }

    fn ensure(&mut self, end: usize, lex: &Lexicon) {
        if self.out.len() >= end || self.done {
            return;
        }
        if let Some(ready) = self.deferred.take() {
            self.pool = Some(ready.pool.clone());
        }
        let Some(pool) = self.pool.as_mut() else {
            return;
        };
        while self.out.len() < end && !self.done {
            if let Some(choice) = pool.next(lex) {
                Arc::make_mut(&mut self.out).push(choice);
            } else {
                self.done = true;
            }
        }
    }
}

fn context_tail(chars: impl Iterator<Item = char>) -> String {
    let mut tail = std::collections::VecDeque::with_capacity(64);
    for ch in chars {
        if matches!(
            ch,
            '。' | '！' | '？' | '\n' | '\r' | '.' | '!' | '?' | ';' | '；'
        ) {
            tail.clear();
        } else {
            if tail.len() == 64 {
                tail.pop_front();
            }
            tail.push_back(ch);
        }
    }
    tail.into_iter().collect()
}

fn chinese_punct(ch: char) -> Option<char> {
    Some(match ch {
        '.' | '。' => '。',
        ',' | '，' => '，',
        '?' | '？' => '？',
        '!' | '！' => '！',
        ';' | '；' => '；',
        ':' | '：' => '：',
        '\\' | '、' => '、',
        '(' | '（' => '（',
        ')' | '）' => '）',
        '[' | '【' => '【',
        ']' | '】' => '】',
        '<' | '《' => '《',
        '>' | '》' => '》',
        '…' => '…',
        _ => return None,
    })
}

#[cfg(test)]
mod refresh_tests {
    use super::*;

    #[test]
    fn disabled_context_ignores_app_text_without_disabling_personal_links() {
        let lex = Arc::new(Lexicon::load());
        let mut s = Session::with_preferences(
            lex,
            UserDict::default(),
            crate::InputPreferences {
                contextual: false,
                ..Default::default()
            },
        );
        for ch in "nihao".chars() {
            s.handle(ch as u32, 0);
        }
        s.handle(0x20, 0);
        s.set_surrounding_text("不参与排序的应用内容", 10, 10);
        for ch in "zhongguo".chars() {
            s.handle(ch as u32, 0);
        }
        s.handle(0x20, 0);
        assert!(s
            .user
            .links
            .get("你好")
            .is_some_and(|links| links.contains_key("中国")));
        assert!(s.surrounding_context.is_none());
    }

    #[test]
    fn application_cursor_moves_do_not_train_false_personal_links() {
        let lex = Arc::new(Lexicon::load().with_test_model());
        let mut s = Session::with_preferences(lex, UserDict::default(), Default::default());
        s.set_surrounding_text("我", 1, 1);
        for ch in "nihao".chars() {
            s.handle(ch as u32, 0);
        }
        s.handle(0x20, 0);
        s.set_surrounding_text("我你好", 3, 3);
        assert_eq!(
            s.context.as_deref(),
            Some("你好"),
            "application acknowledgement must preserve adjacency"
        );
        s.set_surrounding_text("别处", 2, 2);
        for ch in "zhongguo".chars() {
            s.handle(ch as u32, 0);
        }
        s.handle(0x20, 0);
        assert!(
            !s.user.links.contains_key("你好"),
            "cursor movement trained a false link"
        );
        assert_eq!(
            s.user.words.len(),
            2,
            "surrounding application text became a learned entry"
        );
    }

    #[test]
    fn clearing_an_empty_ranking_context_keeps_dictionary_plans() {
        let lex = Arc::new(Lexicon::load());
        let mut session = Session::with_preferences(lex, UserDict::default(), Default::default());
        for ch in "nihao".chars() {
            session.handle(ch as u32, 0);
        }
        let first = session.plans["nihao"].clone();
        session.clear();
        for ch in "nihao".chars() {
            session.handle(ch as u32, 0);
        }
        assert!(Arc::ptr_eq(&first, &session.plans["nihao"]));
    }

    #[test]
    #[cfg(waypinyin_full_lexicon)]
    fn surrounding_context_is_unicode_bounded_ephemeral_and_invalidates_ranking() {
        let lex = Arc::new(Lexicon::load().with_test_model());
        let mut session = Session::with_preferences(lex, UserDict::default(), Default::default());
        session.set_surrounding_text("🙂我想去", 4, 4);
        assert_eq!(session.surrounding_context.as_deref(), Some("🙂我想去"));
        for ch in "beijing".chars() {
            session.handle(ch as u32, 0);
        }
        assert_eq!(session.view().candidates[0], "北京");
        session.set_surrounding_text("看", 1, 1);
        assert_eq!(session.view().candidates[0], "背景");
        session.set_surrounding_text("去", 1, 0);
        assert!(session.surrounding_context.is_none());
        session.set_surrounding_text(&"字".repeat(100), 100, 100);
        assert_eq!(
            session
                .surrounding_context
                .as_ref()
                .unwrap()
                .chars()
                .count(),
            64
        );
        session.set_surrounding_text("去。看", 3, 3);
        assert_eq!(session.surrounding_context.as_deref(), Some("看"));
        session.set_learning_enabled(false);
        session.set_surrounding_text("去", 1, 1);
        assert!(session.surrounding_context.is_none());
        assert!(session.local_context.is_empty());
        assert!(session.user.words.is_empty() && session.user.inputs.is_empty());
        session.set_learning_enabled(true);
        session.set_surrounding_text("去", 1, 1);
        session.reset();
        assert!(session.surrounding_context.is_none());
        session.set_surrounding_text("去", 5, 5);
        assert!(session.surrounding_context.is_none());
    }

    #[test]
    fn prepared_page_is_shared_until_paging_and_learning_stays_live() {
        let lex = Arc::new(Lexicon::load());
        let mut session =
            Session::with_preferences(lex.clone(), UserDict::default(), Default::default());
        session.raw = "ni".into();
        session.caret = 2;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let ready = loop {
            session.preload();
            session.invalidate();
            session.view();
            if let Some(ready) = session.board.as_ref().and_then(|b| b.deferred.clone()) {
                break ready;
            }
            // A ready check is a snapshot: another worker operation may make
            // the next try_lock miss. Verify the actual hit, not two lock reads.
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(2));
        };
        let cold = Arc::new(plan("ni", &lex));
        let mut expected = Board::paged(Pool::open(cold, &lex, Vec::new(), Vec::new()));
        expected.ensure(10, &lex);
        session.view();
        let board = session.board.as_ref().unwrap();
        assert!(board.pool.is_none() && board.deferred.is_some());
        assert!(Arc::ptr_eq(&board.out, &ready.out));
        assert!(board.out.len() <= PAGE_SIZE);
        session.page(1);
        let board = session.board.as_ref().unwrap();
        if ready.done {
            assert!(Arc::ptr_eq(&board.out, &ready.out));
        } else {
            assert!(board.deferred.is_none());
            assert!(!Arc::ptr_eq(&board.out, &ready.out));
        }
        assert_eq!(
            board.out.iter().map(|c| &c.word).collect::<Vec<_>>(),
            expected.out.iter().map(|c| &c.word).collect::<Vec<_>>()
        );
        assert!(ready.out.len() <= PAGE_SIZE);
        session.user = UserDict::from_log("U\t1\tni\t自选词\t8\t0\n");
        session.clear();
        session.handle('n' as u32, 0);
        session.handle('i' as u32, 0);
        assert_eq!(session.view().candidates[0], "自选词");
        assert!(session.board.as_ref().unwrap().deferred.is_none());
    }

    #[test]
    fn shared_plan_keeps_pagination_independent() {
        let lex = Arc::new(Lexicon::load());
        let mut session =
            Session::with_preferences(lex.clone(), UserDict::default(), Default::default());
        let first = session.recall("ni");
        let second = session.recall("ni");
        assert!(Arc::ptr_eq(&first, &second));
        let mut a = Board::paged(Pool::open(first, &lex, Vec::new(), Vec::new()));
        let mut b = Board::paged(Pool::open(second, &lex, Vec::new(), Vec::new()));
        a.ensure(10, &lex);
        assert!(b.out.is_empty());
        b.ensure(1, &lex);
        assert_eq!(a.out[0].word, b.out[0].word);
        assert_eq!(a.out[0].reading, b.out[0].reading);
        assert_eq!(a.out[0].span, b.out[0].span);
        // Cache invalidation must not invalidate an already materializing pool.
        session.plans.clear();
        b.ensure(10, &lex);
        let rows = |board: &Board| {
            board
                .out
                .iter()
                .map(|c| (c.word.clone(), c.reading.clone(), c.span))
                .collect::<Vec<_>>()
        };
        assert_eq!(rows(&a), rows(&b));
    }

    #[test]
    fn settings_change_at_composition_boundary_and_clear_plans() {
        let root =
            std::env::temp_dir().join(format!("waypinyin-session-settings-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("config.toml");
        fs::write(&path, "enabled = true\n").unwrap();
        let source = crate::preferences::PreferencesSource::from_path(path.clone());
        let mut session = Session::with_preferences(
            Arc::new(Lexicon::load()),
            UserDict::default(),
            source.current,
        );
        session.preferences_source = Some(source);
        session.preload();
        session.handle('n' as u32, 0);
        fs::write(root.join("new"), "enabled = false\n").unwrap();
        fs::rename(root.join("new"), &path).unwrap();
        session.handle('i' as u32, 0);
        assert!(session.lexicon.preferences.enabled);
        assert!(session.plans.contains_key("ni"));
        session.clear();
        assert!(matches!(
            session.handle('n' as u32, RELEASE_MASK),
            Effect::Ignore
        ));
        assert!(matches!(
            session.handle('n' as u32, MOD1_MASK),
            Effect::Ignore
        ));
        assert!(session.lexicon.preferences.enabled);
        session.handle('n' as u32, 0);
        assert!(!session.lexicon.preferences.enabled);
        assert!(!session.plans.contains_key("ni"));
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod input_history_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn type_in(s: &mut Session, raw: &str) {
        for c in raw.chars() {
            s.handle(c as u32, 0);
        }
    }

    #[test]
    fn confirmation_records_actual_spelling_but_not_drafts_or_return() {
        let lex = Arc::new(Lexicon::load());
        let mut s = Session::with_preferences(lex, UserDict::default(), Default::default());
        type_in(&mut s, "nihao");
        s.handle(KEY_ESCAPE, 0);
        assert!(s.user.inputs.is_empty());
        type_in(&mut s, "nihao");
        s.handle(KEY_RETURN, 0);
        assert!(s.user.inputs.is_empty());
        for _ in 0..2 {
            type_in(&mut s, "nh");
            assert!(matches!(s.pick(0), Effect::Commit { .. }));
        }
        assert_eq!(s.user.frequent_inputs(), ["nh"]);
        assert_eq!(s.frequent[0], "nh");
        assert_eq!(s.user.inputs["nh"].count, 2);
    }

    #[test]
    fn partial_confirmation_records_the_consumed_prefix_without_separator() {
        let mut s = Session::with_preferences(
            Arc::new(Lexicon::load()),
            UserDict::default(),
            Default::default(),
        );
        type_in(&mut s, "ni'hao");
        let view = s.view();
        let index = view
            .spans
            .iter()
            .position(|span| *span > 0 && *span < view.preedit.len())
            .unwrap();
        s.pick(index);
        assert!(s.user.inputs.contains_key("ni"));
        assert!(!s.user.inputs.contains_key("ni'"));
        s.clear();
        assert!(!s.user.inputs.contains_key("hao"));
    }

    #[test]
    fn disk_roundtrip_and_compaction_retain_input_statistics() {
        let root =
            std::env::temp_dir().join(format!("waypinyin-input-history-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("user.txt");
        let mut user = UserDict::load_from(&path);
        user.record_input("nihao");
        user.record_input("nihao");
        drop(user);
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(UserDict::load_from(&path).frequent_inputs(), ["nihao"]);
        let body = format!(
            "U\t1\tni'hao\t你好\t3\t0\nzg\t中国\t4\n{}",
            "P\t1\tnihao\t1\n".repeat(600)
        );
        fs::write(&path, body).unwrap();
        let compacted = UserDict::load_from(&path);
        assert_eq!(compacted.inputs["nihao"].count, 600);
        assert!(fs::read_to_string(&path).unwrap().lines().count() < 10);
        assert_eq!(
            UserDict::from_log(&compacted.snapshot()).snapshot(),
            compacted.snapshot()
        );
        assert!(compacted.preload_inputs().iter().any(|r| r == "zg"));
        drop(compacted);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn history_is_bounded_decays_and_rejects_non_pinyin_and_clock_codes() {
        let mut user = UserDict::default();
        for raw in [
            "", "sj", "rq", "jt", "mt", "zt", "xq", "NiHao", "ni\thao", "ni''hao", "'ni", "ni'",
        ] {
            user.record_input(raw);
        }
        user.record_input(&"a".repeat(129));
        assert!(user.inputs.is_empty());
        user.fold_input("nihao", 1, 20);
        for a in b'a'..=b'z' {
            for b in b'a'..=b'z' {
                user.record_input(&format!("p{}{}", a as char, b as char));
            }
        }
        assert!(user.inputs.len() <= MAX_FREQUENT_INPUTS);
        assert_eq!(user.frequent_inputs()[0], "nihao");
        user.tick = INPUT_HALF_LIFE * 4 + 1;
        assert!(user.frequent_inputs().is_empty());
        let malformed = UserDict::from_log(
            "P\tbad\tnihao\t3\nP\t1\tUPPER\t3\nP\t1\tnihao\t0\nP\t1\tnihao\t2\nR\tnihao\n",
        );
        assert!(malformed.inputs.is_empty());
    }

    #[test]
    fn private_contexts_and_mixed_context_compositions_never_learn() {
        let mut s = Session::with_preferences(
            Arc::new(Lexicon::load()),
            UserDict::default(),
            Default::default(),
        );
        s.set_learning_enabled(false);
        assert!(!s.preload());
        type_in(&mut s, "nihao");
        s.set_learning_enabled(true);
        s.pick(0);
        assert!(s.user.inputs.is_empty() && s.user.words.is_empty() && s.user.links.is_empty());
        type_in(&mut s, "nihao");
        s.set_learning_enabled(false);
        s.pick(0);
        assert!(s.user.inputs.is_empty() && s.user.words.is_empty());
        s.set_learning_enabled(true);
        for _ in 0..2 {
            type_in(&mut s, "nihao");
            s.pick(0);
        }
        assert_eq!(s.user.frequent_inputs(), ["nihao"]);
    }

    #[test]
    fn existing_word_and_legacy_records_seed_preload_without_new_history() {
        let user = UserDict::from_log("U\t1\tni'hao\t你好\t5\t0\nzg\t中国\t3\n");
        assert!(user.frequent_inputs().is_empty());
        let seed = user.preload_inputs();
        assert_eq!(&*seed, ["nihao", "zg"]);
    }
}
