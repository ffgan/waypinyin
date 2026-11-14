//! Local, independently removable dictionaries. Imported records never enter the learning log.
use crate::lexicon::{match_imported_reading, Choice};
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};
type Result<T> = std::result::Result<T, String>;
#[derive(Clone, Debug)]
pub struct Record {
    pub code: String,
    pub word: String,
    pub shortcut: bool,
}
#[derive(Clone, Debug, Default)]
pub struct Dictionary {
    pub name: String,
    pub records: Vec<Record>,
    pub skipped: usize,
}
const LIMIT: usize = 128 * 1024 * 1024;
fn slice(b: &[u8], p: usize, n: usize) -> Result<&[u8]> {
    b.get(p..p.checked_add(n).ok_or("长度溢出")?)
        .ok_or("文件已截断".into())
}
fn u16at(b: &[u8], p: usize) -> Result<usize> {
    Ok(u16::from_le_bytes(slice(b, p, 2)?.try_into().unwrap()) as usize)
}
fn u32at(b: &[u8], p: usize) -> Result<usize> {
    Ok(u32::from_le_bytes(slice(b, p, 4)?.try_into().unwrap()) as usize)
}
fn utf16(b: &[u8]) -> Result<String> {
    if !b.len().is_multiple_of(2) {
        return Err("UTF-16 长度错误".into());
    }
    String::from_utf16(
        &b.as_chunks::<2>()
            .0
            .iter()
            .map(|s| u16::from_le_bytes([s[0], s[1]]))
            .collect::<Vec<_>>(),
    )
    .map_err(|e| e.to_string())
}
impl Dictionary {
    fn push(&mut self, code: String, word: String, shortcut: bool) {
        let code = code
            .trim()
            .to_lowercase()
            .replace(' ', "'")
            .replace('ü', "v");
        if code.is_empty()
            || code.len() > 256
            || !code.bytes().all(|c| c.is_ascii_lowercase() || c == b'\'')
            || word.is_empty()
            || word.chars().any(char::is_control)
            || word.len() > 4096
        {
            self.skipped += 1;
            return;
        }
        self.records.push(Record {
            code,
            word,
            shortcut,
        });
    }
    pub fn parse(path: &Path) -> Result<Self> {
        let file = fs::File::open(path).map_err(|e| e.to_string())?;
        let mut b = Vec::new();
        file.take((LIMIT + 1) as u64)
            .read_to_end(&mut b)
            .map_err(|e| e.to_string())?;
        if b.len() > LIMIT {
            return Err("文件超过 128 MiB".into());
        }
        let mut d = Self {
            name: path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into(),
            ..Self::default()
        };
        let ext = path
            .extension()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase();
        match ext.as_str() {
            "plist" => {
                let v = plist::Value::from_reader(std::io::Cursor::new(&b))
                    .map_err(|e| e.to_string())?;
                for row in v.as_array().ok_or("plist 必须为短语数组")? {
                    let row = row.as_dictionary().ok_or("短语结构错误")?;
                    match (
                        row.get("shortcut").and_then(|v| v.as_string()),
                        row.get("phrase").and_then(|v| v.as_string()),
                    ) {
                        (Some(c), Some(w)) => d.push(c.into(), w.into(), true),
                        _ => d.skipped += 1,
                    }
                }
            }
            "dat" if b.starts_with(b"mschxudp") => {
                let table = u32at(&b, 16)?;
                let start = u32at(&b, 20)?;
                let end = u32at(&b, 24)?;
                let count = u32at(&b, 28)?;
                if table < 64
                    || count > b.len() / 4
                    || start < table + count * 4
                    || end < start
                    || end > b.len()
                {
                    return Err("微软词库结构错误".into());
                }
                for i in 0..count {
                    let a = start
                        .checked_add(u32at(&b, table + i * 4)?)
                        .ok_or("偏移溢出")?;
                    let z = if i + 1 < count {
                        start
                            .checked_add(u32at(&b, table + (i + 1) * 4)?)
                            .ok_or("偏移溢出")?
                    } else {
                        end
                    };
                    if z < a || z > end {
                        return Err("记录偏移错误".into());
                    }
                    let r = slice(&b, a, z - a)?;
                    if u32at(r, 0)? != 0x00100010 {
                        return Err("未知微软记录版本".into());
                    }
                    let h = u16at(r, 4)?;
                    if h < 18 || r.len() < h + 2 {
                        return Err("短语长度错误".into());
                    }
                    d.push(
                        utf16(slice(r, 16, h - 18)?)?,
                        utf16(slice(r, h, r.len() - h - 2)?)?,
                        true,
                    );
                }
            }
            "scel" | "qcel" => {
                let qq = ext == "qcel";
                if !b.starts_with(&[0x40, 0x15, 0, 0]) {
                    return Err("未知 scel 文件头".into());
                }
                let expected = u32at(&b, 0x124)?;
                if expected > b.len() / 2 {
                    return Err("词条数错误".into());
                }
                let word_start = if qq {
                    None
                } else {
                    Some(match b.get(4) {
                        Some(0x44) => 0x2628,
                        Some(0x45) => 0x26c4,
                        _ => return Err("未知 scel 版本".into()),
                    })
                };
                let mut p = 0x1544;
                let mut py = HashMap::new();
                loop {
                    let id = u16at(&b, p)?;
                    let n = u16at(&b, p + 2)?;
                    p += 4;
                    let s = utf16(slice(&b, p, n)?)?;
                    p += n;
                    let last = s == "zuo";
                    py.insert(id, s);
                    if word_start.is_some_and(|end| p > end) {
                        return Err("拼音表超出词条区".into());
                    }
                    if (qq && last) || word_start == Some(p) {
                        break;
                    }
                    if py.len() > 1024 {
                        return Err("拼音表错误".into());
                    }
                }
                while p < b.len() && d.records.len() + d.skipped < expected {
                    let count = u16at(&b, p)?;
                    let n = u16at(&b, p + 2)?;
                    p += 4;
                    if count == 0 && n == 0 {
                        break;
                    }
                    if n % 2 != 0 {
                        return Err("读音长度错误".into());
                    }
                    let mut codes = Vec::new();
                    for off in (0..n).step_by(2) {
                        let id = u16at(&b, p + off)?;
                        let code = if let Some(code) = py.get(&id) {
                            code.clone()
                        } else if qq && id >= py.len() && id < py.len() + 26 {
                            char::from(b'a' + (id - py.len()) as u8).to_string()
                        } else {
                            return Err("未知拼音索引".into());
                        };
                        codes.push(code)
                    }
                    p += n;
                    for _ in 0..count {
                        let n = u16at(&b, p)?;
                        p += 2;
                        let w = utf16(slice(&b, p, n)?)?;
                        p += n;
                        if qq {
                            slice(&b, p, 12)?;
                            p += 12;
                        } else {
                            let n = u16at(&b, p)?;
                            p += 2;
                            slice(&b, p, n)?;
                            p += n;
                        }
                        d.push(codes.join("'"), w, false);
                    }
                }
                if d.records.len() + d.skipped != expected {
                    return Err("词条数与记录不一致".into());
                }
            }
            "qpyd" => {
                if !b.starts_with(&[0x09, 0xa6, 0x1e, 0x7d]) {
                    return Err("未知 qpyd 文件头".into());
                }
                let start = u32at(&b, 0x38)?;
                let mut raw = Vec::new();
                flate2::read::ZlibDecoder::new(slice(
                    &b,
                    start,
                    b.len().checked_sub(start).ok_or("偏移错误")?,
                )?)
                .take((LIMIT + 1) as u64)
                .read_to_end(&mut raw)
                .map_err(|e| e.to_string())?;
                if raw.len() > LIMIT {
                    return Err("解压数据过大".into());
                }
                let end = u32at(&raw, 6)?;
                if end > raw.len() || end % 10 != 0 {
                    return Err("QQ 索引错误".into());
                }
                for i in (0..end).step_by(10) {
                    let p = u32at(&raw, i + 6)?;
                    let n = slice(&raw, i, 2)?;
                    let c = std::str::from_utf8(slice(&raw, p, n[0] as usize)?)
                        .map_err(|e| e.to_string())?;
                    let w = utf16(slice(&raw, p + n[0] as usize, n[1] as usize)?)?;
                    d.push(c.into(), w, false);
                }
            }
            "txt" | "tsv" | "yaml" => {
                let text = if b.starts_with(&[255, 254]) {
                    utf16(&b[2..])?
                } else {
                    match std::str::from_utf8(&b) {
                        Ok(s) => s.into(),
                        Err(_) => {
                            let (s, _, bad) = encoding_rs::GBK.decode(&b);
                            if bad {
                                return Err("文本编码错误".into());
                            }
                            s.into_owned()
                        }
                    }
                };
                for line in text.lines() {
                    let line = line.trim().trim_start_matches('\u{feff}');
                    if line.is_empty() || line.starts_with('#') || line == "..." || line == "---" {
                        continue;
                    }
                    let cols: Vec<_> = line.split('\t').collect();
                    if cols.len() < 2 {
                        d.skipped += 1;
                        continue;
                    }
                    let (w, c) = if cols[0]
                        .bytes()
                        .all(|c| c.is_ascii_lowercase() || c == b'\'' || c == b' ')
                    {
                        (cols[1], cols[0])
                    } else {
                        (cols[0], cols[1])
                    };
                    d.push(c.into(), w.into(), false);
                }
            }
            _ => return Err("不支持的文件或版本".into()),
        }
        let mut seen = std::collections::HashSet::new();
        d.records
            .retain(|r| seen.insert((r.code.clone(), r.word.clone(), r.shortcut)));
        if d.records.is_empty() {
            return Err("没有可导入的词条".into());
        }
        Ok(d)
    }
    pub fn install(&self, root: &Path) -> Result<()> {
        fs::create_dir_all(root).map_err(|e| e.to_string())?;
        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(root.join(".lock"))
            .map_err(|e| e.to_string())?;
        lock.lock().map_err(|e| e.to_string())?;
        let name = self.name.replace(['/', '\\', '\t', '\n'], "_");
        let path = root.join(format!("{name}.tsv"));
        let tmp = path.with_extension("tmp");
        let mut f = fs::File::create(&tmp).map_err(|e| e.to_string())?;
        for r in &self.records {
            writeln!(f, "{}\t{}\t{}", r.code, r.word, u8::from(r.shortcut))
                .map_err(|e| e.to_string())?
        }
        f.sync_all().map_err(|e| e.to_string())?;
        fs::rename(tmp, path).map_err(|e| e.to_string())
    }
}
pub fn root() -> PathBuf {
    crate::paths::data()
        .expect("set HOME or an absolute XDG_DATA_HOME")
        .join("dictionaries")
}
#[derive(Default, Debug)]
pub(crate) struct Catalog {
    rows: BTreeMap<String, Vec<Record>>,
    stamp: Vec<(PathBuf, u64, Option<std::time::SystemTime>)>,
}
impl Catalog {
    #[cfg(all(test, waypinyin_full_lexicon))]
    pub(crate) fn from_records(records: Vec<Record>) -> Self {
        let mut c = Self::default();
        for r in records {
            if let Some(first) = r.code.get(..1) {
                c.rows.entry(first.into()).or_default().push(r);
            }
        }
        c
    }
    fn stamp() -> Vec<(PathBuf, u64, Option<std::time::SystemTime>)> {
        let mut files = Vec::new();
        if let Ok(entries) = fs::read_dir(root()) {
            for e in entries.flatten() {
                if let Ok(m) = e.metadata() {
                    files.push((e.path(), m.len(), m.modified().ok()));
                }
            }
        }
        files.sort();
        files
    }
    pub fn refresh(&mut self) {
        if self.stamp != Self::stamp() {
            *self = Self::load();
        }
    }
    pub fn load() -> Self {
        let mut c = Self {
            stamp: Self::stamp(),
            ..Default::default()
        };
        if let Ok(files) = fs::read_dir(root()) {
            for f in files.flatten() {
                if f.path().extension().is_some_and(|e| e == "tsv") {
                    if let Ok(t) = fs::read_to_string(f.path()) {
                        for l in t.lines() {
                            let v: Vec<_> = l.split('\t').collect();
                            if v.len() == 3 {
                                let r = Record {
                                    code: v[0].into(),
                                    word: v[1].into(),
                                    shortcut: v[2] == "1",
                                };
                                if let Some(ch) = r.code.chars().next() {
                                    c.rows.entry(ch.to_string()).or_default().push(r)
                                }
                            }
                        }
                    }
                }
            }
        }
        c
    }
    #[cfg(test)]
    pub fn choices(&self, raw: &str) -> Vec<Choice> {
        self.choices_with_preferences(raw, Default::default())
    }

    pub(crate) fn choices_with_preferences(
        &self,
        raw: &str,
        prefs: crate::InputPreferences,
    ) -> Vec<Choice> {
        let mut out = Vec::new();
        for head in prefs
            .heads(raw.bytes().next().unwrap_or(0))
            .into_iter()
            .filter(|h| *h != 0)
        {
            let key = char::from(head).to_string();
            let Some(rows) = self.rows.get(&key) else {
                continue;
            };
            for r in rows {
                let hit = if r.shortcut {
                    if r.code != raw {
                        continue;
                    }
                    None
                } else {
                    match match_imported_reading(prefs, raw, &r.code) {
                        Some(h) if h.span == raw.len() => Some(h),
                        _ => continue,
                    }
                };
                out.push(Choice {
                    word: r.word.clone(),
                    reading: if r.shortcut {
                        String::new()
                    } else {
                        r.code.clone()
                    },
                    span: raw.len(),
                    tier: hit.map_or(0, |h| h.tier),
                    score: if r.shortcut { i64::MAX / 2 } else { 1 },
                    user_weight: 0,
                });
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parse(ext: &str, b: &[u8]) -> Result<Dictionary> {
        let dir = std::env::temp_dir().join(format!(
            "waypinyin-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join(format!("fixture.{ext}"));
        fs::write(&p, b).unwrap();
        let r = Dictionary::parse(&p);
        fs::remove_dir_all(dir).unwrap();
        r
    }
    #[test]
    fn text_normalizes_and_deduplicates() {
        let d = parse(
            "yaml",
            "---\nname: test\n...\n你好\tni hao\t100\n你好\tni hao\t200\n女儿\tnü er\t2\n"
                .as_bytes(),
        )
        .unwrap();
        assert_eq!(d.records.len(), 2);
        assert_eq!(d.records[1].code, "nv'er");
    }
    #[test]
    fn apple_keeps_shortcuts() {
        let d=parse("plist",br#"<?xml version="1.0"?><plist version="1.0"><array><dict><key>shortcut</key><string>msd</string><key>phrase</key><string>On my way!</string></dict></array></plist>"#).unwrap();
        assert!(d.records[0].shortcut);
        let c = Catalog {
            rows: BTreeMap::from([("m".into(), d.records)]),
            ..Default::default()
        };
        assert_eq!(c.choices("msd")[0].word, "On my way!");
        assert!(c.choices("ms").is_empty());
    }
    #[test]
    fn corrupted_binary_never_panics() {
        for ext in ["dat", "scel", "qcel", "qpyd", "plist"] {
            for n in [0, 1, 16, 64, 1024, 6000] {
                assert!(parse(ext, &vec![0; n]).is_err(), "{ext} {n}");
            }
        }
    }
    #[test]
    fn microsoft_offsets_are_checked() {
        let mut b = vec![0; 64];
        b[..8].copy_from_slice(b"mschxudp");
        b[16..20].copy_from_slice(&64u32.to_le_bytes());
        b[20..24].copy_from_slice(&68u32.to_le_bytes());
        b[28..32].copy_from_slice(&1u32.to_le_bytes());
        let mut r = vec![0; 16];
        r[..4].copy_from_slice(&0x00100010u32.to_le_bytes());
        r[4..6].copy_from_slice(&24u16.to_le_bytes());
        for s in ["msd\0", "马上到\0"] {
            r.extend(s.encode_utf16().flat_map(u16::to_le_bytes));
        }
        b.extend(0u32.to_le_bytes());
        b.extend(r);
        let end = b.len() as u32;
        b[24..28].copy_from_slice(&end.to_le_bytes());
        let d = parse("dat", &b).unwrap();
        assert_eq!(d.records[0].word, "马上到");
        assert_eq!(d.records[0].code, "msd");
        b[64..68].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(parse("dat", &b).is_err());
    }
    #[test]
    fn large_import_round_trip() {
        let dir = std::env::temp_dir().join(format!("waypinyin-large-{}", std::process::id()));
        let mut d = Dictionary {
            name: "large".into(),
            ..Default::default()
        };
        for i in 0..100_000 {
            d.records.push(Record {
                code: "ce'shi".into(),
                word: format!("测试{i}"),
                shortcut: false,
            });
        }
        d.install(&dir).unwrap();
        let t = fs::read_to_string(dir.join("large.tsv")).unwrap();
        assert_eq!(t.lines().count(), 100_000);
        d.install(&dir).unwrap();
        assert_eq!(
            fs::read_dir(&dir)
                .unwrap()
                .filter_map(|e| e.ok())
                .filter(|e| e.path().extension().is_some_and(|s| s == "tsv"))
                .count(),
            1
        );
        fs::remove_dir_all(dir).unwrap();
    }
}

#[cfg(test)]
mod external_fixtures {
    #[test]
    #[ignore = "set WAYPINYIN_FIXTURES to a directory containing external native dictionaries"]
    fn native_dictionaries() {
        let root = std::env::var_os("WAYPINYIN_FIXTURES").expect("fixture directory");
        for f in std::fs::read_dir(root).unwrap().flatten() {
            let d = super::Dictionary::parse(&f.path())
                .unwrap_or_else(|e| panic!("{}: {e}", f.path().display()));
            assert!(!d.records.is_empty());
            eprintln!("{}: {} records", d.name, d.records.len());
        }
    }
}

#[cfg(test)]
mod scel_version_tests {
    use super::*;
    #[test]
    fn e_version_reads_letters_after_zuo_and_stops_before_footer() {
        let mut b = vec![0; 0x1544];
        b[..5].copy_from_slice(&[0x40, 0x15, 0, 0, 0x45]);
        b[0x124..0x128].copy_from_slice(&1u32.to_le_bytes());
        // Fill the table region with three synthetic entries. The E version
        // has letters after zuo; record count excludes the trailing metadata.
        let filler = (0x26c4usize - 0x1544 - 4 - 10 - 6) / 2;
        for (id, s) in [
            (0u16, "a".repeat(filler)),
            (1, "zuo".into()),
            (2, "A".into()),
        ] {
            b.extend(id.to_le_bytes());
            b.extend(((s.encode_utf16().count() * 2) as u16).to_le_bytes());
            b.extend(s.encode_utf16().flat_map(u16::to_le_bytes));
        }
        assert_eq!(b.len(), 0x26c4);
        for n in [1u16, 2, 2, 2] {
            b.extend(n.to_le_bytes());
        }
        b.extend("阿".encode_utf16().flat_map(u16::to_le_bytes));
        b.extend(2u16.to_le_bytes());
        b.extend(1u16.to_le_bytes());
        b.extend([33, 0, 1, 0]);
        let p =
            std::env::temp_dir().join(format!("waypinyin-e-version-{}.scel", std::process::id()));
        fs::write(&p, b).unwrap();
        let parsed = Dictionary::parse(&p);
        fs::remove_file(p).unwrap();
        let d = parsed.unwrap();
        assert_eq!(d.records.len(), 1);
        assert_eq!(d.records[0].code, "a");
        assert_eq!(d.records[0].word, "阿");
    }
}
