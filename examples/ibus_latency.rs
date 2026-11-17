//! Private-bus adapter probe. Run through scripts/ibus_perf.py, never on the desktop bus.
use futures_util::StreamExt;
use std::collections::BTreeMap;
use std::error::Error;
use std::time::{Duration, Instant};
use zbus::message::Type;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};
use zbus::{Connection, Message, MessageStream};

const NAME: &str = "org.freedesktop.IBus.Waypinyin";
const IFACE: &str = "org.freedesktop.IBus.Engine";

fn routed() -> bool {
    static ROUTED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ROUTED.get_or_init(|| std::env::var_os("WAYPINYIN_PROBE_ROUTED").is_some())
}

fn destination() -> &'static str {
    if routed() {
        "org.freedesktop.IBus"
    } else {
        NAME
    }
}

fn interface() -> &'static str {
    if routed() {
        "org.freedesktop.IBus.InputContext"
    } else {
        IFACE
    }
}

#[derive(Default, Debug)]
struct Display {
    preedit: Option<(OwnedValue, u32, bool, u32)>,
    lookup: Option<(OwnedValue, bool)>,
    commits: Vec<OwnedValue>,
    signals: usize,
    bytes: usize,
}

async fn exchange(
    conn: &Connection,
    stream: &mut MessageStream,
    request: Message,
    display: &mut Display,
) -> Result<(Message, Duration), Box<dyn Error>> {
    let serial = request.primary_header().serial_num();
    let started = Instant::now();
    conn.send(&request).await?;
    while let Some(message) = stream.next().await {
        let message = message?;
        let header = message.header();
        if header.reply_serial() == Some(serial) {
            let elapsed = started.elapsed();
            if header.message_type() == Type::Error {
                return Err(format!(
                    "D-Bus error: {:?}: {:?}",
                    header.error_name(),
                    message.body().deserialize::<String>(),
                )
                .into());
            }
            return Ok((message, elapsed));
        }
        if header.message_type() != Type::Signal
            || header.interface().map(|v| v.as_str()) != Some(interface())
        {
            continue;
        }
        display.signals += 1;
        display.bytes += message.data().len();
        match header.member().map(|v| v.as_str()) {
            Some("UpdatePreeditText") if routed() => {
                let (text, cursor, visible) = message.body().deserialize()?;
                display.preedit = Some((text, cursor, visible, 0));
            }
            Some("UpdatePreeditText") => display.preedit = Some(message.body().deserialize()?),
            Some("UpdatePreeditTextWithMode") => {
                display.preedit = Some(message.body().deserialize()?)
            }
            Some("UpdateLookupTable") => display.lookup = Some(message.body().deserialize()?),
            Some("CommitText") => display.commits.push(message.body().deserialize()?),
            Some("RequireSurroundingText" | "Enabled" | "Disabled") if routed() => {}
            Some("HidePreeditText" | "ShowPreeditText") if routed() => {
                if let Some(preedit) = &mut display.preedit {
                    preedit.2 = header.member().unwrap().as_str() == "ShowPreeditText";
                }
            }
            Some("HideLookupTable" | "ShowLookupTable") if routed() => {
                if let Some(lookup) = &mut display.lookup {
                    lookup.1 = header.member().unwrap().as_str() == "ShowLookupTable";
                }
            }
            other => return Err(format!("unexpected signal: {other:?}").into()),
        }
    }
    Err("bus closed before reply".into())
}

fn request(path: &OwnedObjectPath, member: &str) -> zbus::Result<zbus::message::Builder<'static>> {
    Message::method_call(path.clone(), member.to_string())?
        .destination(destination())?
        .interface(interface())
}

async fn verify(
    conn: &Connection,
    stream: &mut MessageStream,
    path: &OwnedObjectPath,
    display: &mut Display,
) -> Result<(), Box<dyn Error>> {
    exchange(conn, stream, request(path, "Reset")?.build(&())?, display).await?;
    assert_eq!(display.signals, 2, "initial empty state must be published");
    for ch in "nihao".chars() {
        exchange(
            conn,
            stream,
            request(path, "ProcessKeyEvent")?.build(&(ch as u32, 0u32, 0u32))?,
            display,
        )
        .await?;
    }
    for (key, state, expected_signals) in [
        (0xff51, 0, 1),           // left: caret changes, candidate table stays the same
        (0xff53, 0, 1),           // right
        (0xff53, 0, 0),           // right at the end: no display change
        ('x' as u32, 1 << 30, 0), // release: no display change
        ('c' as u32, 1 << 2, 0),  // shortcut: no display change
    ] {
        let before = display.signals;
        exchange(
            conn,
            stream,
            request(path, "ProcessKeyEvent")?.build(&(key, 0u32, state as u32))?,
            display,
        )
        .await?;
        assert_eq!(display.signals - before, expected_signals);
    }
    let before = display.signals;
    exchange(
        conn,
        stream,
        request(path, "FocusOut")?.build(&())?,
        display,
    )
    .await?;
    assert_eq!(display.commits.len(), 1, "focus loss must commit the draft");
    assert_eq!(
        display.signals - before,
        3,
        "commit and clear both UI components"
    );
    assert!(!display.preedit.as_ref().unwrap().2);
    assert!(!display.lookup.as_ref().unwrap().1);
    let before = display.signals;
    exchange(conn, stream, request(path, "Reset")?.build(&())?, display).await?;
    assert_eq!(display.signals, before, "unchanged reset must send nothing");
    println!("independent signals, ignored keys, focus-loss commit and reset passed");
    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    // A separate address is mandatory: no fallback to the user's session bus.
    let address = std::env::var("WAYPINYIN_PROBE_BUS")?;
    let mode = std::env::args().nth(1).unwrap_or_else(|| "bench".into());
    assert!(matches!(
        mode.as_str(),
        "bench" | "trace" | "verify" | "lifecycle"
    ));
    let conn = zbus::connection::Builder::address(address.as_str())?
        .build()
        .await?;
    let path: OwnedObjectPath = if routed() {
        let bus = zbus::Proxy::new(
            &conn,
            destination(),
            "/org/freedesktop/IBus",
            "org.freedesktop.IBus",
        )
        .await?;
        let path: OwnedObjectPath = bus
            .call("CreateInputContext", &("waypinyin-latency-probe",))
            .await?;
        let context = zbus::Proxy::new(&conn, destination(), path.clone(), interface()).await?;
        context
            .call::<_, _, ()>("SetCapabilities", &(13u32,))
            .await?;
        context.call::<_, _, ()>("FocusIn", &()).await?;
        bus.call::<_, _, ()>("SetGlobalEngine", &("waypinyin",))
            .await?;
        path
    } else {
        let mut result = None;
        for _ in 0..500 {
            match conn
                .call_method(
                    Some(NAME),
                    "/org/freedesktop/IBus/Factory",
                    Some("org.freedesktop.IBus.Factory"),
                    "CreateEngine",
                    &("waypinyin",),
                )
                .await
            {
                Ok(reply) => {
                    result = Some(reply.body().deserialize()?);
                    break;
                }
                Err(_) => std::thread::sleep(Duration::from_millis(10)),
            }
        }
        result.ok_or("engine did not register within startup retries")?
    };
    if mode == "lifecycle" {
        assert!(
            !routed(),
            "lifecycle verification calls the engine service directly"
        );
        let factory = zbus::Proxy::new(
            &conn,
            NAME,
            "/org/freedesktop/IBus/Factory",
            "org.freedesktop.IBus.Factory",
        )
        .await?;
        for index in 0..100 {
            let path: OwnedObjectPath = if index == 0 {
                path.clone()
            } else {
                factory.call("CreateEngine", &("waypinyin",)).await?
            };
            let service =
                zbus::Proxy::new(&conn, NAME, path.clone(), "org.freedesktop.IBus.Service").await?;
            service.call::<_, _, ()>("Destroy", &()).await?;
            let reply = conn
                .call_method(
                    Some(NAME),
                    path,
                    Some(IFACE),
                    "ProcessKeyEvent",
                    &(0u32, 0u32, 0u32),
                )
                .await;
            assert!(
                matches!(reply, Err(zbus::Error::MethodError(ref name, ..))
                if name.as_str() == "org.freedesktop.DBus.Error.UnknownObject"),
                "destroyed engine must no longer be callable: {reply:?}"
            );
        }
        println!("100 engine create/destroy cycles released their bus objects");
        return Ok(());
    }
    let rule = zbus::MatchRule::builder()
        .msg_type(Type::Signal)
        .sender(destination())?
        .path(path.clone())?
        .interface(interface())?
        .build();
    // The unfiltered stream observes signals and method replies in socket order,
    // without a timeout-based signal drain or a second, undrained signal queue.
    zbus::fdo::DBusProxy::new(&conn)
        .await?
        .add_match_rule(rule)
        .await?;
    let mut stream = MessageStream::from(&conn);
    let mut display = Display::default();
    if mode == "verify" {
        return verify(&conn, &mut stream, &path, &mut display).await;
    }
    let mut timings: BTreeMap<&str, Vec<u128>> = BTreeMap::new();
    let mut wire: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    exchange(
        &conn,
        &mut stream,
        request(&path, "FocusIn")?.build(&())?,
        &mut display,
    )
    .await?;
    let cases = include_str!("../tests/common.tsv")
        .lines()
        .map(|line| ("short", line.split_once('\t').unwrap().0))
        .chain(
            include_str!("../tests/sentences.tsv")
                .lines()
                .filter(|line| !line.is_empty() && !line.starts_with('#'))
                .flat_map(|line| line.split('\t').skip(1).map(|raw| ("sentence", raw))),
        );
    for (group, raw) in cases {
        exchange(
            &conn,
            &mut stream,
            request(&path, "Reset")?.build(&())?,
            &mut display,
        )
        .await?;
        let keys = raw.chars().map(|ch| (group, ch as u32, 0u32)).chain([
            ("edit", 0xff51, 0),
            ("edit", 0xff53, 0),
            ("select", 0xff54, 0),
            ("select", 0xff52, 0),
            ("page", 0xff56, 0),
            ("page", 0xff55, 0),
            ("edit", 0xff08, 0),
            ("edit", raw.chars().last().unwrap() as u32, 0),
            ("edit", 0xffff, 0),
            ("ignored", 'x' as u32, 1 << 30), // release
            ("ignored", 'c' as u32, 1 << 2),  // Ctrl+C
            ("commit", ' ' as u32, 0),
        ]);
        for (group, key, state) in keys {
            display.commits.clear();
            let previous_wire = (display.signals, display.bytes);
            let (reply, elapsed) = exchange(
                &conn,
                &mut stream,
                request(&path, "ProcessKeyEvent")?.build(&(key, 0u32, state))?,
                &mut display,
            )
            .await?;
            let consumed: bool = reply.body().deserialize()?;
            if matches!(group, "short" | "sentence" | "commit") {
                assert!(consumed, "active engine must consume {group} keys");
            }
            if group == "commit" {
                assert!(
                    !display.commits.is_empty(),
                    "commit signal must precede its reply"
                );
            }
            if mode == "trace" {
                println!(
                    "{raw}\t{group}\t{key}\t{consumed}\t{:?}\t{:?}\t{:?}",
                    display.preedit, display.lookup, display.commits,
                );
            } else {
                timings.entry(group).or_default().push(elapsed.as_nanos());
                let counters = wire.entry(group).or_default();
                counters.0 += display.signals - previous_wire.0;
                counters.1 += display.bytes - previous_wire.1;
            }
        }
    }
    if mode == "bench" {
        for (group, mut values) in timings {
            values.sort_unstable();
            let percentile = |p: usize| values[(values.len() * p).div_ceil(100) - 1];
            println!(
                "{group} count={} p50_ns={} p95_ns={} p99_ns={} max_ns={}",
                values.len(),
                percentile(50),
                percentile(95),
                percentile(99),
                values.last().unwrap()
            );
        }
        for (group, (signals, bytes)) in wire {
            println!("wire_{group} signals={signals} bytes={bytes}");
        }
        println!("wire signals={} bytes={}", display.signals, display.bytes);
    }
    Ok(())
}
