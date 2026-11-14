//! Reproducible native import using the same parser and storage as Settings.
use waypinyin::imports::{root, Dictionary};
fn main() -> Result<(), String> {
    let mut install = false;
    for arg in std::env::args().skip(1) {
        if arg == "--install" {
            install = true;
            continue;
        }
        let d = Dictionary::parse(std::path::Path::new(&arg))?;
        println!(
            "{}: {} records, {} skipped",
            d.name,
            d.records.len(),
            d.skipped
        );
        for r in d.records.iter().take(20) {
            println!("{}\t{}", r.code, r.word);
        }
        if install {
            d.install(&root())?;
            println!("Installed in {}", root().display());
        }
    }
    Ok(())
}
