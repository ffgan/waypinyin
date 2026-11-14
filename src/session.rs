//! Composition state. Key handling, paging, and the user dictionary live here.
//! The bus adapter applies [`Effect`] and does not read the session again.

use crate::lexicon::{match_reading, plan, Choice, Lexicon, Plan, Pool};
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Read, Write};
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
        if let Some(dir) = path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        let Ok(mut file) = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
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
            let tmp = path.with_extension("new");
            if fs::write(&tmp, body).is_ok() {
                let _ = fs::rename(&tmp, &path);
            }
        }
        drop(file);
        if let Ok(appended) = OpenOptions::new().create(true).append(true).open(&path) {
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
    plans: HashMap<String, Plan>,
    /// Pieces already committed from the current composition.
    pending: Vec<Piece>,
    /// Last committed Han word. The next commit is learned as its follower.
    context: Option<String>,
    /// Reading of `context`, used to find longer lexicon words.
    context_reading: Option<String>,
    /// Post-commit association list. Kept off so the lookup closes with the word.
    associating: bool,
    /// The next `"` is an opening quote.
    quote_open: bool,
    reload_preferences: bool,
}

impl Session {
    #[cfg(all(test, waypinyin_full_lexicon))]
    pub fn new(lexicon: Arc<Lexicon>) -> Self {
        Self::with_user(lexicon, UserDict::default())
    }

    pub fn with_user(lexicon: Arc<Lexicon>, user: UserDict) -> Self {
        {
            let mut session =
                Self::with_preferences(lexicon, user, crate::InputPreferences::load());
            session.reload_preferences = true;
            session
        }
    }

    /// Creates a session with fixed preferences; this constructor does not reload disk settings.
    pub fn with_preferences(
        lexicon: Arc<Lexicon>,
        user: UserDict,
        preferences: crate::InputPreferences,
    ) -> Self {
        let lexicon = Arc::new(lexicon.with_preferences(preferences));
        Self {
            raw: String::new(),
            caret: 0,
            cursor: 0,
            lexicon,
            user,
            imported: crate::imports::Catalog::load(),
            board: None,
            plans: HashMap::new(),
            pending: Vec::new(),
            context: None,
            context_reading: None,
            associating: false,
            quote_open: true,
            reload_preferences: false,
        }
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
        self.raw.clear();
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
        self.caret = 0;
        self.cursor = 0;
        self.invalidate();
    }

    fn invalidate(&mut self) {
        self.board = None;
    }

    fn recall(&mut self, raw: &str) -> Plan {
        if let Some(found) = self.plans.get(raw) {
            return found.clone();
        }
        let planned = plan(raw, &self.lexicon);
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
        let planned = self.recall(&raw);
        let (mut users, partials) = self
            .user
            .overlay(&raw, self.context.as_deref(), &self.lexicon);
        users.extend(
            self.imported
                .choices_with_preferences(&raw, self.lexicon.preferences),
        );
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
        Effect::Commit {
            text,
            next: self.view(),
        }
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
        if self.raw.is_empty() && (0x61..=0x7a).contains(&keyval) {
            self.imported.refresh();
            if self.reload_preferences {
                let prefs = crate::InputPreferences::load();
                if prefs != self.lexicon.preferences {
                    self.lexicon = Arc::new(self.lexicon.with_preferences(prefs));
                    self.plans.clear();
                    self.invalidate();
                }
            }
        }
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
        if self.raw.is_empty() {
            return self.handle_idle(keyval);
        }
        self.handle_composing(keyval)
    }

    fn forget_chord(&mut self, keyval: u32) -> Effect {
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
        let mut text: String = pieces.iter().map(|choice| choice.word.as_str()).collect();
        if let Some(ch) = punct {
            text.push(chinese_punct(ch).unwrap_or(ch));
        }
        let learnable: Vec<Choice> = pieces
            .iter()
            .filter(|choice| !choice.reading.is_empty() && han_word(&choice.word))
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
        self.wipe_preedit();
        self.finish(text)
    }

    fn note(&mut self, choice: &Choice, done: bool) {
        if choice.reading.is_empty() || !han_word(&choice.word) {
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
    (page + 2) * PAGE_SIZE
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
    out: Vec<Choice>,
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
            out,
            done: true,
        }
    }

    fn paged(pool: Pool) -> Self {
        Self {
            pool: Some(pool),
            out: Vec::new(),
            done: false,
        }
    }

    fn ensure(&mut self, end: usize, lex: &Lexicon) {
        let Some(pool) = self.pool.as_mut() else {
            return;
        };
        while self.out.len() < end && !self.done {
            if let Some(choice) = pool.next(lex) {
                self.out.push(choice);
            } else {
                self.done = true;
            }
        }
    }
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
