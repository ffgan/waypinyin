use std::env;
use std::fs;
use std::path::PathBuf;
use std::time::Instant;

#[allow(dead_code)]
#[path = "src/image.rs"]
mod image;
#[path = "src/lexicon_build.rs"]
mod lexicon_build;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/image.rs");
    println!("cargo:rerun-if-changed=src/lexicon_build.rs");
    println!("cargo:rustc-check-cfg=cfg(waypinyin_full_lexicon)");
    println!("cargo:rerun-if-env-changed=WAYPINYIN_SOURCE_DATA");
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let source = lexicon_build::source_dir(&manifest);
    let demo =
        source.canonicalize().ok() == manifest.join("tests/fixtures/demo").canonicalize().ok();
    if demo {
        println!(
            "cargo:warning=Using the small original demo lexicon, not a production dictionary"
        );
    } else {
        println!("cargo:rustc-cfg=waypinyin_full_lexicon");
    }
    println!(
        "cargo:rustc-env=WAYPINYIN_LEXICON_KIND={}",
        if demo { "demo" } else { "external" }
    );
    println!("cargo:rerun-if-changed={}", manifest.join("data").display());
    for name in [
        "cedict.txt",
        "dict.txt",
        "extra.txt",
        "terms.txt",
        "freq.txt",
        "frost.txt",
        "gb2312.txt",
        "pinyin.txt",
        "ts-characters.txt",
    ] {
        println!("cargo:rerun-if-changed={}", source.join(name).display());
    }

    println!("cargo:rerun-if-env-changed=WAYPINYIN_DATADIR");
    let datadir =
        env::var("WAYPINYIN_DATADIR").unwrap_or_else(|_| "/usr/local/share/waypinyin".into());
    println!("cargo:rustc-env=WAYPINYIN_DATADIR={datadir}");
    assert_eq!(
        env::var("HOST").ok(),
        env::var("TARGET").ok(),
        "cross compilation of native lexicon images is not supported"
    );
    let started = Instant::now();
    let bytes = lexicon_build::compile();
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let image_path = out_dir.join("lexicon.bin");
    let len = bytes.len();
    fs::write(&image_path, &bytes).expect("write lexicon image");
    drop(bytes);
    println!("cargo:rustc-env=WAYPINYIN_LEXICON={}", image_path.display());
    eprintln!(
        "waypinyin: lexicon image {:.1} MiB in {:.2}s",
        len as f64 / (1024.0 * 1024.0),
        started.elapsed().as_secs_f64()
    );
}
