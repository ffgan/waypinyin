//! Bounded, offline backoff statistics. Never receives or persists user text.
use rustc_hash::FxHashMap;
use rustc_hash::FxHashSet;
use std::path::Path;

pub(crate) fn fingerprint(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x100000001b3)
    })
}

#[derive(Debug, Default)]
pub(crate) struct LanguageModel {
    pub(crate) weight: f64,
    pub(crate) prior_weight: f64,
    chars: FxHashMap<[u64; 3], f64>,
    words: FxHashMap<[u64; 3], f64>,
    unigrams: FxHashMap<u64, f64>,
    tokens: FxHashSet<u64>,
    prior_pairs: FxHashSet<[u64; 2]>,
}

#[derive(Clone, Copy)]
pub(crate) struct WordFeatures {
    token: u64,
    first: [u64; 2],
    length: usize,
    internal: f64,
}

impl LanguageModel {
    pub(crate) fn load(path: &Path, dictionary: u64) -> Result<Self, String> {
        let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        if file.metadata().map_err(|e| e.to_string())?.len() > 128 * 1024 * 1024 {
            return Err("model exceeds 128 MiB".into());
        }
        use std::io::Read;
        let mut bytes = Vec::new();
        file.take(128 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        Self::parse(&bytes, dictionary)
    }

    pub(crate) fn parse(bytes: &[u8], dictionary: u64) -> Result<Self, String> {
        let text = std::str::from_utf8(bytes).map_err(|_| "model is not UTF-8")?;
        let (header, body) = text.split_once('\n').ok_or("missing model header")?;
        if header.len() > 128 {
            return Err("oversized model header".into());
        }
        let fields: Vec<_> = header.split('\t').collect();
        if fields.len() != 4
            || fields[0] != "WAYPINYIN-LM"
            || fields[1] != "1"
            || fields[2].parse::<u64>().ok() != Some(dictionary)
            || fields[3].parse::<u64>().ok() != Some(fingerprint(body.as_bytes()))
        {
            return Err("model version, dictionary fingerprint or checksum mismatch".into());
        }
        let mut model = Self::default();
        for line in include_str!("../data/sentences.txt")
            .lines()
            .filter(|l| !l.starts_with('#'))
        {
            let chars: Vec<_> = line.chars().map(|c| c as u64).collect();
            for pair in chars.windows(2) {
                model.prior_pairs.insert([pair[0], pair[1]]);
            }
        }
        model.weight = std::env::var("WAYPINYIN_MODEL_WEIGHT")
            .ok()
            .and_then(|s| s.parse::<f64>().ok())
            .filter(|w| w.is_finite() && (0.0..=1.0).contains(w))
            .unwrap_or(0.9);
        model.prior_weight = std::env::var("WAYPINYIN_PRIOR_WEIGHT")
            .ok()
            .and_then(|s| s.parse::<f64>().ok())
            .filter(|w| w.is_finite() && (0.0..=1.0).contains(w))
            .unwrap_or(1.0);
        for (index, line) in body.lines().enumerate() {
            if index >= 200_000 {
                return Err("too many model records".into());
            }
            if line.len() > 128 {
                return Err("oversized model record".into());
            }
            let fields: Vec<_> = line.split('\t').collect();
            if fields.len() != 5 {
                return Err("invalid model record".into());
            }
            let mut key = [0; 3];
            for i in 0..3 {
                key[i] = fields[i + 1].parse().map_err(|_| "invalid model key")?;
            }
            let weight: f64 = fields[4].parse().map_err(|_| "invalid model weight")?;
            if !weight.is_finite() || !(-8.0..=20.0).contains(&weight) {
                return Err("invalid model weight".into());
            }
            if fields[0] == "U" {
                if key[0] != 0
                    || key[1] != 0
                    || !(0.0..=20.0).contains(&weight)
                    || model.unigrams.insert(key[2], weight).is_some()
                {
                    return Err("invalid unigram record".into());
                }
                continue;
            }
            if weight > 8.0 {
                return Err("invalid transition weight".into());
            }
            if fields[0] == "W" && key[2] != 0 {
                model.tokens.insert(key[2]);
            }
            let table = match fields[0] {
                "C" => &mut model.chars,
                "W" => &mut model.words,
                _ => return Err("unknown model table".into()),
            };
            if table.insert(key, weight).is_some() {
                return Err("duplicate model record".into());
            }
        }
        if model.chars.is_empty() && model.words.is_empty() && model.unigrams.is_empty() {
            return Err("empty model".into());
        }
        Ok(model)
    }

    pub(crate) fn token(word: &str) -> u64 {
        fingerprint(word.as_bytes())
    }

    pub(crate) fn history(&self, text: &str) -> ([u64; 2], [u64; 2]) {
        let chars: Vec<_> = text.chars().collect();
        let mut words = [0; 2];
        let mut at = 0;
        while at < chars.len() {
            let mut next = at + 1;
            let mut token = Self::token(&chars[at].to_string());
            for end in (at + 1..=(at + 8).min(chars.len())).rev() {
                let candidate = Self::token(&chars[at..end].iter().collect::<String>());
                if self.tokens.contains(&candidate) {
                    next = end;
                    token = candidate;
                    break;
                }
            }
            words = [words[1], token];
            at = next;
        }
        let mut tail = [0; 2];
        for ch in chars {
            tail = [tail[1], ch as u64];
        }
        (tail, words)
    }

    /// Interpolated conditional/unigram log-ratio; unseen contexts back off.
    pub(crate) fn bonus(&self, chars: [u64; 2], words: [u64; 2], text: &str) -> f64 {
        self.bonus_features(chars, words, self.features(text))
    }

    fn char_bonus(&self, previous: [u64; 2], c: u64) -> f64 {
        let gain = Self::gain(&self.chars, previous, c);
        // Sparse external counts must not turn an established conversational
        // pair into negative evidence simply because its trigram was pruned.
        0.5 * if gain < 0.0 && self.prior_pairs.contains(&[previous[1], c]) {
            0.0
        } else {
            gain
        }
    }

    fn gain(table: &FxHashMap<[u64; 3], f64>, previous: [u64; 2], next: u64) -> f64 {
        if let Some(value) = table.get(&[previous[0], previous[1], next]) {
            return *value;
        }
        let bigram = table
            .get(&[0, previous[1], next])
            .or_else(|| table.get(&[0, previous[1], 0]))
            .copied()
            .unwrap_or(0.0);
        if previous[0] == 0 {
            bigram
        } else {
            bigram
                + table
                    .get(&[previous[0], previous[1], 0])
                    .copied()
                    .unwrap_or(0.0)
        }
    }

    pub(crate) fn features(&self, text: &str) -> WordFeatures {
        let mut first = [0; 2];
        let mut previous = [0; 2];
        let mut length = 0;
        let mut internal = 0.0;
        for c in text.chars().map(|c| c as u64) {
            if length < 2 {
                first[length] = c;
            } else {
                internal += self.char_bonus(previous, c);
            }
            previous = [previous[1], c];
            length += 1;
        }
        let token = Self::token(text);
        WordFeatures {
            token,
            first,
            length,
            internal,
        }
    }

    pub(crate) fn word_cost(&self, dictionary_cost: f64, text: &str) -> f64 {
        let learned = self
            .unigrams
            .get(&Self::token(text))
            .copied()
            .unwrap_or(dictionary_cost + 2.0);
        dictionary_cost + self.weight * (learned - dictionary_cost)
    }

    pub(crate) fn bonus_features(
        &self,
        mut chars: [u64; 2],
        words: [u64; 2],
        f: WordFeatures,
    ) -> f64 {
        // Tokenization differs between dictionary paths and the training
        // segmenter. Missing token edges therefore back off to characters.
        let mut bonus = Self::gain(&self.words, words, f.token).max(0.0) + f.internal;
        for c in f.first.into_iter().take(f.length.min(2)) {
            bonus += self.char_bonus(chars, c);
            chars = [chars[1], c];
        }
        bonus
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_corruption_and_backs_off_without_user_data() {
        let body = "C\t0\t20320\t22909\t2.5\n";
        let data = format!(
            "WAYPINYIN-LM\t1\t42\t{}\n{body}",
            fingerprint(body.as_bytes())
        );
        let model = LanguageModel::parse(data.as_bytes(), 42).unwrap();
        assert_eq!(model.bonus([0, '你' as u64], [0; 2], "好"), 1.25);
        assert_eq!(model.bonus([0; 2], [0; 2], "好"), 0.0);
        assert!(LanguageModel::parse(data.as_bytes(), 43).is_err());
        assert!(LanguageModel::parse(data.replace("2.5", "NaN").as_bytes(), 42).is_err());
        assert!(LanguageModel::parse(b"invalid", 42).is_err());
    }
}
