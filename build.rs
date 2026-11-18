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
    // The optional sidecar leaves the dictionary ABI unchanged. Its header
    // binds both the dictionary and the entire model payload.
    println!("cargo:rerun-if-changed=data/context-model.txt");
    let model_path = out_dir.join("lexicon.lm");
    println!("cargo:rerun-if-env-changed=WAYPINYIN_MODEL_BODY");
    let model_source = env::var_os("WAYPINYIN_MODEL_BODY").map(PathBuf::from);
    if let Some(path) = &model_source {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    if let Some(model_source) = model_source {
        let body = fs::read(&model_source).expect("read explicit WAYPINYIN_MODEL_BODY");
        let fingerprint = |bytes: &[u8]| {
            bytes.iter().fold(0xcbf29ce484222325u64, |h, b| {
                (h ^ u64::from(*b)).wrapping_mul(0x100000001b3)
            })
        };
        let header = format!(
            "WAYPINYIN-LM\t1\t{}\t{}\n",
            fingerprint(&bytes),
            fingerprint(&body)
        );
        let mut model = header.into_bytes();
        model.extend_from_slice(&body);
        fs::write(&model_path, model).expect("write language model");
    } else {
        let _ = fs::remove_file(&model_path);
    }
    drop(bytes);
    println!("cargo:rustc-env=WAYPINYIN_LEXICON={}", image_path.display());
    eprintln!(
        "waypinyin: lexicon image {:.1} MiB in {:.2}s",
        len as f64 / (1024.0 * 1024.0),
        started.elapsed().as_secs_f64()
    );
}
