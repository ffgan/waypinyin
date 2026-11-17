//! IBus adapter. It turns a [`crate::session::Effect`] into bus signals and
//! does not choose candidates or edit the composition.

use crate::lexicon::Lexicon;
use crate::session::View;
use crate::session::{Effect, Session, UserDict};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{Array, Dict, OwnedObjectPath, Signature, StructureBuilder, Value as Variant};
use zbus::{connection::Builder, fdo, interface, Connection};

fn empty_dict() -> Dict<'static, 'static> {
    Dict::new(&Signature::Str, &Signature::Variant)
}

fn empty_variants() -> Array<'static> {
    Array::new(&Signature::Variant)
}

fn shared_attrs() -> Variant<'static> {
    static ATTRS: OnceLock<Variant<'static>> = OnceLock::new();
    ATTRS
        .get_or_init(|| {
            let attrs = StructureBuilder::new()
                .append_field(Variant::from("IBusAttrList"))
                .append_field(Variant::Dict(empty_dict()))
                .append_field(Variant::Array(empty_variants()))
                .build()
                .expect("attr list");
            Variant::Structure(attrs)
        })
        .clone()
}

fn ibus_text(text: &str) -> Variant<'static> {
    Variant::Structure(
        StructureBuilder::new()
            .append_field(Variant::from("IBusText"))
            .append_field(Variant::Dict(empty_dict()))
            .append_field(Variant::from(text.to_owned()))
            .append_field(Variant::Value(Box::new(shared_attrs())))
            .build()
            .expect("ibus text"),
    )
}

fn shared_label(index: usize) -> Variant<'static> {
    static LABELS: OnceLock<Vec<Variant<'static>>> = OnceLock::new();
    let labels = LABELS.get_or_init(|| (1..=9).map(|n| ibus_text(&n.to_string())).collect());
    labels
        .get(index.saturating_sub(1))
        .cloned()
        .unwrap_or_else(|| ibus_text(&index.to_string()))
}

/// `(page start, cursor within the page, words on the page)`.
fn page_window(view: &View) -> (usize, u32, &[String]) {
    if view.candidates.is_empty() || view.page_size == 0 {
        return (0, 0, &[]);
    }
    let start = (view.cursor / view.page_size) * view.page_size;
    let start = start.min(view.candidates.len());
    let end = (start + view.page_size).min(view.candidates.len());
    let cursor = view
        .cursor
        .saturating_sub(start)
        .min(end.saturating_sub(start));
    (start, cursor as u32, &view.candidates[start..end])
}

fn ibus_lookup(view: &View) -> Variant<'static> {
    let (_start, cursor, page) = page_window(view);
    let mut candidates = Array::new(&Signature::Variant);
    for word in page {
        candidates
            .append(Variant::Value(Box::new(ibus_text(word))))
            .expect("candidate variant");
    }
    let mut labels = Array::new(&Signature::Variant);
    for n in 1..=view.page_size {
        labels
            .append(Variant::Value(Box::new(shared_label(n))))
            .expect("label variant");
    }
    Variant::Structure(
        StructureBuilder::new()
            .append_field(Variant::from("IBusLookupTable"))
            .append_field(Variant::Dict(empty_dict()))
            .append_field(Variant::from(view.page_size as u32))
            .append_field(Variant::from(cursor))
            .append_field(Variant::from(true))
            .append_field(Variant::from(false))
            .append_field(Variant::from(0i32))
            .append_field(Variant::Array(candidates))
            .append_field(Variant::Array(labels))
            .build()
            .expect("lookup table"),
    )
}

struct Shown {
    preedit: String,
    caret: u32,
    candidates: Vec<String>,
    cursor: u32,
    show_lookup: bool,
    start: usize,
}

fn same_shown(shown: &Shown, view: &View) -> bool {
    let (start, cursor, page) = page_window(view);
    let (preedit, caret) = displayed_preedit(view);
    shown.show_lookup == view.show_lookup
        && shown.cursor == cursor
        && shown.start == start
        && shown.caret == caret
        && shown.preedit == preedit
        && shown.candidates.iter().eq(page)
}

/// Draw `ni'hao` when the highlighted reading spells the consumed letters.
/// The caret stays on the same raw letter the user is editing.
fn displayed_preedit(view: &View) -> (String, u32) {
    let span = view.spans.get(view.cursor).copied().unwrap_or(0);
    let letters: String = view.reading.chars().filter(|ch| *ch != '\'').collect();
    let raw_caret = view.caret.min(view.preedit.len());
    let aligned = !letters.is_empty()
        && span > 0
        && letters.len() == span
        && view.preedit.len() >= span
        && view.preedit.as_bytes()[..span] == *letters.as_bytes();
    if !aligned {
        return (view.preedit.clone(), raw_caret as u32);
    }
    let mut shown = view.reading.clone();
    shown.push_str(&view.preedit[span..]);
    let cursor = if raw_caret <= span {
        display_caret(&view.reading, raw_caret)
    } else {
        view.reading.chars().count() + (raw_caret - span)
    };
    (shown, cursor as u32)
}

fn display_caret(reading: &str, raw_caret: usize) -> usize {
    let chars: Vec<char> = reading.chars().collect();
    let mut raw = 0usize;
    let mut index = 0usize;
    while index < chars.len() {
        if chars[index] == '\'' {
            index += 1;
            continue;
        }
        if raw == raw_caret {
            return index;
        }
        raw += 1;
        index += 1;
        if raw == raw_caret {
            if chars.get(index) == Some(&'\'') {
                return index + 1;
            }
            return index;
        }
    }
    chars.len()
}

struct Factory {
    lexicon: Arc<Lexicon>,
    next_id: AtomicU64,
}

struct Engine {
    session: Session,
    focused: bool,
    content_type: (u32, u32),
    shown: Option<Shown>,
    /// Absolute index of the first candidate in the published page.
    page_at: usize,
}

#[interface(name = "org.freedesktop.IBus.Factory")]
impl Factory {
    async fn create_engine(
        &self,
        #[zbus(connection)] conn: &Connection,
        name: &str,
    ) -> fdo::Result<OwnedObjectPath> {
        if name != "waypinyin" {
            return Err(fdo::Error::Failed(format!("unknown engine {name}")));
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let path = OwnedObjectPath::try_from(format!("/org/freedesktop/IBus/Engine/{id}"))
            .map_err(|err| fdo::Error::Failed(err.to_string()))?;
        let engine = Engine {
            session: Session::with_user(Arc::clone(&self.lexicon), UserDict::load()),
            focused: false,
            content_type: (0, 0),
            shown: None,
            page_at: 0,
        };
        conn.object_server()
            .at(path.clone(), engine)
            .await
            .map_err(|err| fdo::Error::Failed(err.to_string()))?;
        eprintln!("waypinyin: engine {path}");
        Ok(path)
    }
}

#[interface(name = "org.freedesktop.IBus.Engine")]
impl Engine {
    async fn process_key_event(
        &mut self,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
        keyval: u32,
        _keycode: u32,
        state: u32,
    ) -> fdo::Result<bool> {
        let effect = self.session.handle(keyval, state);
        self.present(&emitter, effect).await
    }

    async fn set_cursor_location(&mut self, _x: i32, _y: i32, _w: i32, _h: i32) {}

    async fn process_hand_writing_event(&mut self, _coordinates: Vec<f64>) {}

    async fn cancel_hand_writing(&mut self, _n_strokes: u32) {}

    async fn set_capabilities(&mut self, _caps: u32) {}

    async fn property_activate(&mut self, _name: &str, _state: u32) {}

    async fn property_show(&mut self, _name: &str) {}

    async fn property_hide(&mut self, _name: &str) {}

    async fn candidate_clicked(
        &mut self,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
        index: u32,
        _button: u32,
        state: u32,
    ) -> fdo::Result<()> {
        let effect = self.session.pick(self.page_at + index as usize);
        if let effect @ Effect::Commit { .. } = effect {
            self.show(&emitter, effect).await?;
        }
        let _ = state;
        Ok(())
    }

    async fn focus_in(&mut self) {
        self.focused = true;
        self.session.preload();
    }

    async fn focus_in_id(&mut self, _object_path: &str, _client: &str) {
        self.focused = true;
        self.session.preload();
    }

    async fn focus_out(
        &mut self,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> fdo::Result<()> {
        self.focused = false;
        let effect = self.session.commit_preedit();
        self.show(&emitter, effect).await
    }

    async fn focus_out_id(
        &mut self,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
        _object_path: &str,
    ) -> fdo::Result<()> {
        self.focus_out(emitter).await
    }

    async fn reset(
        &mut self,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> fdo::Result<()> {
        let effect = self.session.reset();
        self.show(&emitter, effect).await
    }

    async fn enable(&mut self) {}

    async fn disable(
        &mut self,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> fdo::Result<()> {
        let effect = self.session.reset();
        self.show(&emitter, effect).await
    }

    async fn page_up(
        &mut self,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> fdo::Result<()> {
        let effect = self.session.page(-1);
        self.show(&emitter, effect).await
    }

    async fn page_down(
        &mut self,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> fdo::Result<()> {
        let effect = self.session.page(1);
        self.show(&emitter, effect).await
    }

    async fn cursor_up(
        &mut self,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> fdo::Result<()> {
        let effect = self.session.page(-1);
        self.show(&emitter, effect).await
    }

    async fn cursor_down(
        &mut self,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> fdo::Result<()> {
        let effect = self.session.page(1);
        self.show(&emitter, effect).await
    }

    async fn set_surrounding_text(
        &mut self,
        _text: Variant<'_>,
        _cursor_pos: u32,
        _anchor_pos: u32,
    ) {
    }

    async fn panel_extension_received(&mut self, _event: Variant<'_>) {}

    async fn panel_extension_register_keys(&mut self, _data: Variant<'_>) {}

    #[zbus(property)]
    fn focus_id(&self) -> bool {
        self.focused
    }

    #[zbus(property)]
    fn active_surrounding_text(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn content_type(&self) -> (u32, u32) {
        self.content_type
    }

    #[zbus(property)]
    fn set_content_type(&mut self, content_type: (u32, u32)) {
        self.content_type = content_type;
        // IBusInputPurpose PASSWORD=8/PIN=9, hints PRIVATE=1<<11 and
        // HIDDEN_TEXT=1<<12 (ibustypes.h). These fields never update learning.
        let private =
            matches!(content_type.0, 8 | 9) || content_type.1 & ((1 << 11) | (1 << 12)) != 0;
        self.session.set_learning_enabled(!private);
    }

    #[zbus(signal)]
    async fn commit_text(emitter: &SignalEmitter<'_>, text: Variant<'_>) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn update_preedit_text(
        emitter: &SignalEmitter<'_>,
        text: Variant<'_>,
        cursor_pos: u32,
        visible: bool,
        mode: u32,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn update_lookup_table(
        emitter: &SignalEmitter<'_>,
        table: Variant<'_>,
        visible: bool,
    ) -> zbus::Result<()>;
}

impl Engine {
    async fn publish(&mut self, emitter: &SignalEmitter<'_>, view: &View) -> fdo::Result<()> {
        if self
            .shown
            .as_ref()
            .is_some_and(|shown| same_shown(shown, view))
        {
            return Ok(());
        }
        let (start, cursor, page) = page_window(view);
        let (preedit, caret) = displayed_preedit(view);
        let preedit_visible = !preedit.is_empty();
        Self::update_preedit_text(emitter, ibus_text(&preedit), caret, preedit_visible, 0)
            .await
            .map_err(|err| fdo::Error::Failed(err.to_string()))?;
        Self::update_lookup_table(emitter, ibus_lookup(view), view.show_lookup)
            .await
            .map_err(|err| fdo::Error::Failed(err.to_string()))?;
        self.page_at = start;
        self.shown = Some(Shown {
            preedit,
            caret,
            candidates: page.to_vec(),
            cursor,
            show_lookup: view.show_lookup,
            start,
        });
        Ok(())
    }

    /// Apply one session result. A commit carries the text and the preedit
    /// that remains, so this method does not look at the session.
    async fn present(&mut self, emitter: &SignalEmitter<'_>, effect: Effect) -> fdo::Result<bool> {
        match effect {
            Effect::Ignore => Ok(false),
            Effect::Forward(view) => {
                self.publish(emitter, &view).await?;
                Ok(false)
            }
            Effect::Update(view) => {
                self.publish(emitter, &view).await?;
                Ok(true)
            }
            Effect::Commit { text, next } => {
                Self::commit_text(emitter, ibus_text(&text))
                    .await
                    .map_err(|err| fdo::Error::Failed(err.to_string()))?;
                self.publish(emitter, &next).await?;
                Ok(true)
            }
        }
    }

    async fn show(&mut self, emitter: &SignalEmitter<'_>, effect: Effect) -> fdo::Result<()> {
        self.present(emitter, effect).await.map(|_| ())
    }
}

pub(crate) async fn serve() -> zbus::Result<()> {
    let Some(address) = ibus_address() else {
        eprintln!("waypinyin: IBUS_ADDRESS is not set; this program is started by ibus-daemon");
        std::process::exit(1);
    };
    let lexicon = Arc::new(Lexicon::load());
    if let Some(preloader) = crate::preload::Preloader::shared(lexicon.clone()) {
        preloader.request(
            "",
            0,
            crate::InputPreferences::load(),
            None,
            [0; 5],
            Arc::from([]),
        );
    }
    let factory = Factory {
        lexicon,
        next_id: AtomicU64::new(1),
    };
    let _conn = Builder::address(address.as_str())?
        .name("org.freedesktop.IBus.Waypinyin")?
        .serve_at("/org/freedesktop/IBus/Factory", factory)?
        .build()
        .await?;
    eprintln!("waypinyin: registered org.freedesktop.IBus.Waypinyin");
    std::future::pending::<()>().await;
    Ok(())
}

fn ibus_address() -> Option<String> {
    if let Ok(addr) = std::env::var("IBUS_ADDRESS") {
        if !addr.is_empty() {
            return Some(addr);
        }
    }
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if let Some(addr) = arg.strip_prefix("--address=") {
            return Some(addr.to_string());
        }
        if arg == "--address" {
            return args.next();
        }
    }
    address_from_file()
}

/// Same lookup as `ibus_get_address`: the daemon spawns engines without
/// `IBUS_ADDRESS`, and they read `~/.config/ibus/bus/<machine>-unix-<display>`.
fn address_from_file() -> Option<String> {
    let path = std::env::var("IBUS_ADDRESS_FILE")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(socket_path);
    let text = std::fs::read_to_string(path).ok()?;
    let mut address = None;
    let mut pid = None;
    for line in text.lines() {
        if let Some(value) = line.strip_prefix("IBUS_ADDRESS=") {
            if !value.is_empty() {
                address = Some(value.to_string());
            }
        } else if let Some(value) = line.strip_prefix("IBUS_DAEMON_PID=") {
            pid = value.trim().parse::<i32>().ok();
        }
    }
    let pid = pid?;
    if !std::path::Path::new(&format!("/proc/{pid}")).exists() {
        return None;
    }
    address
}

fn socket_path() -> String {
    let config = std::env::var("XDG_CONFIG_HOME")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
            format!("{home}/.config")
        });
    let (hostname, display) = display_parts();
    format!("{config}/ibus/bus/{}-{hostname}-{display}", machine_id())
}

fn machine_id() -> String {
    for path in ["/var/lib/dbus/machine-id", "/etc/machine-id"] {
        if let Ok(text) = std::fs::read_to_string(path) {
            let id = text.trim();
            if !id.is_empty() {
                return id.to_string();
            }
        }
    }
    "machine-id".into()
}

fn display_parts() -> (String, String) {
    if let Ok(display) = std::env::var("WAYLAND_DISPLAY") {
        if !display.is_empty() {
            return ("unix".into(), display);
        }
    }
    let display = std::env::var("DISPLAY").unwrap_or_default();
    let Some((host, rest)) = display.split_once(':') else {
        return ("unix".into(), "0".into());
    };
    let number = rest.split('.').next().unwrap_or("0");
    let hostname = if host.is_empty() { "unix" } else { host };
    (hostname.to_string(), number.to_string())
}

#[cfg(test)]
mod sig_tests {
    use super::*;

    #[test]
    fn password_pin_private_and_hidden_content_types_disable_learning() {
        let mut engine = Engine {
            session: Session::with_preferences(
                Arc::new(Lexicon::load()),
                UserDict::default(),
                Default::default(),
            ),
            focused: false,
            content_type: (0, 0),
            shown: None,
            page_at: 0,
        };
        for content in [(8, 0), (9, 0), (0, 1 << 11), (0, 1 << 12)] {
            engine.set_content_type(content);
            assert_eq!(engine.content_type(), content);
            assert!(!engine.session.learning_enabled());
            engine.set_content_type((0, 0));
            assert!(engine.session.learning_enabled());
        }
    }

    #[test]
    fn text_and_lookup_signatures() {
        assert_eq!(
            ibus_text("\u{4f60}\u{597d}").value_signature().to_string(),
            "(sa{sv}sv)"
        );
        let view = View {
            preedit: "ni".into(),
            candidates: vec!["\u{4f60}".into()],
            spans: vec![2],
            cursor: 0,
            caret: 2,
            reading: String::new(),
            page_size: 7,
            show_lookup: true,
        };
        assert_eq!(
            ibus_lookup(&view).value_signature().to_string(),
            "(sa{sv}uubbiavav)"
        );
    }

    #[test]
    fn lookup_window_is_the_current_page() {
        let view = View {
            preedit: "nihao".into(),
            candidates: (0..12).map(|n| format!("c{n}")).collect(),
            spans: vec![5; 12],
            cursor: 7,
            caret: 5,
            reading: String::new(),
            page_size: 5,
            show_lookup: true,
        };
        let (start, cursor, page) = page_window(&view);
        assert_eq!(start, 5);
        assert_eq!(cursor, 2);
        assert_eq!(
            page.iter().map(String::as_str).collect::<Vec<_>>(),
            vec!["c5", "c6", "c7", "c8", "c9"]
        );
        let shown = Shown {
            preedit: view.preedit.clone(),
            caret: 5,
            candidates: page.to_vec(),
            cursor,
            show_lookup: true,
            start,
        };
        assert!(same_shown(&shown, &view));
        let mut moved = view.clone();
        moved.cursor = 8;
        assert!(!same_shown(&shown, &moved));
    }

    #[test]
    fn segmented_preedit_keeps_the_caret_on_the_raw_letter() {
        let view = View {
            preedit: "nihao".into(),
            candidates: vec!["你好".into()],
            spans: vec![5],
            cursor: 0,
            caret: 2,
            reading: "ni'hao".into(),
            page_size: 5,
            show_lookup: true,
        };
        let (text, caret) = displayed_preedit(&view);
        assert_eq!(text, "ni'hao");
        assert_eq!(caret, 3);
    }
}
