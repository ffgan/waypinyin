#[tokio::main(flavor = "current_thread")]
async fn main() -> zbus::Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    match args.as_slice() {
        [] => waypinyin::run().await,
        [arg] if arg == "--help" || arg == "-h" => {
            println!("Waypinyin {}\n\nUsage: waypinyin [--settings | --check-data | --help | --version]\n\nWithout arguments, serve as an IBus engine.\n  --settings  Open input and dictionary settings\n  --check-data Validate the installed lexicon without a display or IBus\n  --help      Show this help\n  --version   Show version", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        [arg] if arg == "--version" || arg == "-V" => {
            println!(
                "waypinyin {} ({} lexicon)",
                env!("CARGO_PKG_VERSION"),
                env!("WAYPINYIN_LEXICON_KIND")
            );
            Ok(())
        }
        [arg] if arg == "--check-data" => {
            waypinyin::Lexicon::load();
            println!(
                "waypinyin: {} lexicon validated",
                env!("WAYPINYIN_LEXICON_KIND")
            );
            Ok(())
        }
        [arg] if arg == "--settings" => {
            if let Err(e) = waypinyin::settings::run() {
                eprintln!("waypinyin: {e}");
                std::process::exit(1);
            }
            Ok(())
        }
        _ => {
            eprintln!("waypinyin: invalid arguments; see --help");
            std::process::exit(2);
        }
    }
}
