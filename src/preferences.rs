//! Persisted input preferences, shared by settings and input sessions.
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputPreferences {
    pub enabled: bool,
    /// z/zh, c/ch, s/sh, an/ang, en/eng, in/ing, n/l, f/h, r/l, ou/uo.
    pub groups: [bool; 10],
}
impl Default for InputPreferences {
    fn default() -> Self {
        Self {
            enabled: true,
            groups: [
                true, true, true, true, true, true, false, false, false, true,
            ],
        }
    }
}
impl InputPreferences {
    pub const LABELS: [&'static str; 10] = [
        "z / zh", "c / ch", "s / sh", "an / ang", "en / eng", "in / ing", "n / l", "f / h",
        "r / l", "ou / uo",
    ];
    const KEYS: [&'static str; 10] = [
        "z_zh", "c_ch", "s_sh", "an_ang", "en_eng", "in_ing", "n_l", "f_h", "r_l", "ou_uo",
    ];
    pub fn path() -> PathBuf {
        crate::paths::config()
            .expect("set HOME or an absolute XDG_CONFIG_HOME")
            .join("config.toml")
    }
    pub fn load() -> Self {
        match std::fs::read_to_string(Self::path()) {
            Ok(text) => Self::parse(&text).unwrap_or_else(|e| {
                eprintln!("waypinyin: {e}");
                Self::default()
            }),
            Err(e) => {
                if e.kind() != std::io::ErrorKind::NotFound {
                    eprintln!("waypinyin: {e}");
                }
                Self::default()
            }
        }
    }
    fn parse(text: &str) -> Result<Self, String> {
        let mut out = Self::default();
        for line in text.lines() {
            let line = line.split('#').next().unwrap().trim();
            if line.is_empty() {
                continue;
            }
            let (key, value) = line.split_once('=').ok_or("无效的输入配置")?;
            let value = match value.trim() {
                "true" => true,
                "false" => false,
                _ => return Err("输入配置必须为布尔值".into()),
            };
            if key.trim() == "enabled" {
                out.enabled = value;
            } else if let Some(i) = Self::KEYS.iter().position(|k| *k == key.trim()) {
                out.groups[i] = value;
            } else {
                return Err(format!("未知输入配置：{}", key.trim()));
            }
        }
        Ok(out)
    }
    pub fn save(&self) -> Result<(), String> {
        self.save_to(&Self::path())
    }
    fn save_to(&self, path: &std::path::Path) -> Result<(), String> {
        let parent = path.parent().unwrap();
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        let mut text = format!("enabled = {}\n", self.enabled);
        for (key, value) in Self::KEYS.iter().zip(self.groups) {
            text.push_str(&format!("{key} = {value}\n"));
        }
        // Unique temporary file permits concurrent settings windows.
        let temporary = parent.join(format!(
            ".config-{}-{}.tmp",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let result = (|| {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            f.write_all(text.as_bytes())?;
            f.sync_all()?;
            std::fs::rename(&temporary, path)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        result.map_err(|e: std::io::Error| e.to_string())
    }
    pub(crate) fn group(self, i: usize) -> bool {
        self.enabled && self.groups[i]
    }
    pub(crate) fn heads(self, letter: u8) -> [u8; 3] {
        let mut out = [letter, 0, 0];
        let mut n = 1;
        for (i, a, b) in [(6, b'n', b'l'), (7, b'f', b'h'), (8, b'r', b'l')] {
            if self.group(i) && (letter == a || letter == b) {
                out[n] = if letter == a { b } else { a };
                n += 1;
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn atomic_save_roundtrip_and_failure() {
        let root = std::env::temp_dir().join(format!("waypinyin-prefs-{}", std::process::id()));
        let path = root.join("config.toml");
        let mut p = InputPreferences::default();
        p.groups[6] = true;
        p.save_to(&path).unwrap();
        assert_eq!(
            InputPreferences::parse(&std::fs::read_to_string(&path).unwrap()).unwrap(),
            p
        );
        p.enabled = false;
        p.save_to(&path).unwrap();
        assert_eq!(
            InputPreferences::parse(&std::fs::read_to_string(&path).unwrap()).unwrap(),
            p
        );
        assert!(p.save_to(&path.join("invalid.toml")).is_err());
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn configuration_parsing() {
        assert_eq!(
            InputPreferences::parse("").unwrap(),
            InputPreferences::default()
        );
        let p = InputPreferences::parse("enabled = false\nn_l = true # comment\n").unwrap();
        assert!(!p.enabled);
        assert!(p.groups[6]);
        assert!(InputPreferences::parse("enabled = yes").is_err());
        assert!(InputPreferences::parse("bad syntax").is_err());
        assert!(InputPreferences::parse("unknown = true").is_err());
        let mut p = InputPreferences::default();
        p.groups[6] = true;
        p.groups[8] = true;
        assert_eq!(p.heads(b'l'), [b'l', b'n', b'r']);
    }
}
