//! On-disk lexicon image. The build script writes it; the input method only maps it.

use std::mem::{align_of, size_of, size_of_val};

pub(crate) const HEADER_LEN: usize = 128;

const OFF_TEXT: usize = 16;
const OFF_TEXT_BYTES: usize = 24;
const OFF_SYL: usize = 32;
const OFF_SYL_BYTES: usize = 40;
const OFF_NODES: usize = 48;
const OFF_WORDS: usize = 56;
const OFF_EDGES: usize = 64;
const OFF_AUTO: usize = 72;
const OFF_TERM: usize = 80;
const OFF_EXACT: usize = 88;
const OFF_EXACT_AT: usize = 96;
const OFF_EXTEND: usize = 104;
const OFF_EXTEND_AT: usize = 112;
const OFF_WORD_AT: usize = 120;

const _: () = assert!(cfg!(target_endian = "little"));

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct NodeRec {
    pub parent: u32,
    pub edge_lo: u32,
    pub word_lo: u32,
    pub word_n: u32,
    pub syl: u16,
    pub depth: u16,
    pub edge_n: u16,
    pub _pad: u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct WordRec {
    pub text: u32,
    /// `u32::MAX` minus the subtitle count. A lower value sorts first when
    /// two rows share a score.
    pub seq: u32,
    pub score: i64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct EdgeRec {
    pub node: u32,
    pub syl: u16,
    pub letter: u8,
    pub _pad: u8,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct BigramRec {
    pub text: u32,
    pub _pad: u32,
    pub score: i64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct RangeRec {
    pub start: u32,
    pub end: u32,
}

const _: () = assert!(size_of::<NodeRec>() == 24);
const _: () = assert!(align_of::<NodeRec>() == 4);
const _: () = assert!(size_of::<WordRec>() == 16);
const _: () = assert!(align_of::<WordRec>() == 8);
const _: () = assert!(size_of::<EdgeRec>() == 8);
const _: () = assert!(size_of::<BigramRec>() == 16);
const _: () = assert!(align_of::<BigramRec>() == 8);
const _: () = assert!(size_of::<RangeRec>() == 8);

pub(crate) struct Tables<'a> {
    pub texts: &'a [String],
    pub syls: &'a [String],
    pub nodes: &'a [NodeRec],
    pub words: &'a [WordRec],
    pub edges: &'a [EdgeRec],
    pub auto: &'a [[u32; 26]],
    pub auto_term: &'a [u16],
    pub exact: &'a [BigramRec],
    pub exact_at: &'a [RangeRec],
    pub extend: &'a [BigramRec],
    pub extend_at: &'a [RangeRec],
    pub word_at: &'a [u32],
    pub ln_corpus: f64,
}

pub(crate) struct Mapped<'a> {
    pub texts_off: &'a [u32],
    pub text_bytes: &'a [u8],
    pub syls_off: &'a [u32],
    pub syl_bytes: &'a [u8],
    pub nodes: &'a [NodeRec],
    pub words: &'a [WordRec],
    pub edges: &'a [EdgeRec],
    pub auto: &'a [[u32; 26]],
    pub auto_term: &'a [u16],
    pub exact: &'a [BigramRec],
    pub exact_at: &'a [RangeRec],
    pub extend: &'a [BigramRec],
    pub extend_at: &'a [RangeRec],
    pub word_at: &'a [u32],
    pub ln_corpus: f64,
}

pub(crate) fn encode(src: &Tables<'_>) -> Vec<u8> {
    let mut buf = vec![0u8; HEADER_LEN];
    let (text_off, text_len, text_bytes_off, text_bytes_len) = push_strings(&mut buf, src.texts);
    let (syl_off, syl_len, syl_bytes_off, syl_bytes_len) = push_strings(&mut buf, src.syls);
    let (nodes_off, nodes_len) = push_pods(&mut buf, src.nodes);
    let (words_off, words_len) = push_pods(&mut buf, src.words);
    let (edges_off, edges_len) = push_pods(&mut buf, src.edges);
    let (auto_off, auto_len) = push_pods(&mut buf, src.auto);
    let (term_off, term_len) = push_pods(&mut buf, src.auto_term);
    let (exact_off, exact_len) = push_pods(&mut buf, src.exact);
    let (exact_at_off, exact_at_len) = push_pods(&mut buf, src.exact_at);
    let (extend_off, extend_len) = push_pods(&mut buf, src.extend);
    let (extend_at_off, extend_at_len) = push_pods(&mut buf, src.extend_at);
    let (word_at_off, word_at_len) = push_pods(&mut buf, src.word_at);

    buf[..4].copy_from_slice(b"WPY1");
    put_u32(&mut buf, 4, 1);
    put_f64(&mut buf, 8, src.ln_corpus);
    put_pair(&mut buf, OFF_TEXT, text_off, text_len);
    put_pair(&mut buf, OFF_TEXT_BYTES, text_bytes_off, text_bytes_len);
    put_pair(&mut buf, OFF_SYL, syl_off, syl_len);
    put_pair(&mut buf, OFF_SYL_BYTES, syl_bytes_off, syl_bytes_len);
    put_pair(&mut buf, OFF_NODES, nodes_off, nodes_len);
    put_pair(&mut buf, OFF_WORDS, words_off, words_len);
    put_pair(&mut buf, OFF_EDGES, edges_off, edges_len);
    put_pair(&mut buf, OFF_AUTO, auto_off, auto_len);
    put_pair(&mut buf, OFF_TERM, term_off, term_len);
    put_pair(&mut buf, OFF_EXACT, exact_off, exact_len);
    put_pair(&mut buf, OFF_EXACT_AT, exact_at_off, exact_at_len);
    put_pair(&mut buf, OFF_EXTEND, extend_off, extend_len);
    put_pair(&mut buf, OFF_EXTEND_AT, extend_at_off, extend_at_len);
    put_pair(&mut buf, OFF_WORD_AT, word_at_off, word_at_len);
    buf
}

pub(crate) fn open(bytes: &[u8]) -> Mapped<'_> {
    assert!(
        (bytes.as_ptr() as usize).is_multiple_of(8),
        "lexicon image is not aligned"
    );
    assert!(bytes.len() >= HEADER_LEN, "lexicon image is truncated");
    assert!(&bytes[..4] == b"WPY1", "lexicon image has the wrong magic");
    assert!(u32_at(bytes, 4) == 1, "lexicon image has the wrong version");
    let texts_off = pod_slice(bytes, u32_at(bytes, OFF_TEXT), u32_at(bytes, OFF_TEXT + 4));
    let text_bytes = byte_slice(
        bytes,
        u32_at(bytes, OFF_TEXT_BYTES),
        u32_at(bytes, OFF_TEXT_BYTES + 4),
    );
    let syls_off = pod_slice(bytes, u32_at(bytes, OFF_SYL), u32_at(bytes, OFF_SYL + 4));
    let syl_bytes = byte_slice(
        bytes,
        u32_at(bytes, OFF_SYL_BYTES),
        u32_at(bytes, OFF_SYL_BYTES + 4),
    );
    assert!(
        texts_off.len() >= 2 && texts_off[texts_off.len() - 1] as usize == text_bytes.len(),
        "lexicon image text table is truncated"
    );
    assert!(
        syls_off.len() >= 2 && syls_off[syls_off.len() - 1] as usize == syl_bytes.len(),
        "lexicon image syllable table is truncated"
    );
    let auto = pod_slice(bytes, u32_at(bytes, OFF_AUTO), u32_at(bytes, OFF_AUTO + 4));
    let auto_term = pod_slice(bytes, u32_at(bytes, OFF_TERM), u32_at(bytes, OFF_TERM + 4));
    let exact_at = pod_slice(
        bytes,
        u32_at(bytes, OFF_EXACT_AT),
        u32_at(bytes, OFF_EXACT_AT + 4),
    );
    let extend_at = pod_slice(
        bytes,
        u32_at(bytes, OFF_EXTEND_AT),
        u32_at(bytes, OFF_EXTEND_AT + 4),
    );
    let word_at = pod_slice(
        bytes,
        u32_at(bytes, OFF_WORD_AT),
        u32_at(bytes, OFF_WORD_AT + 4),
    );
    assert!(
        auto_term.len() == auto.len()
            && exact_at.len() == auto.len()
            && extend_at.len() == auto.len(),
        "lexicon image automaton tables disagree"
    );
    assert!(
        word_at.len() + 1 == texts_off.len(),
        "lexicon image word index does not match the text table"
    );
    let mapped = Mapped {
        texts_off,
        text_bytes,
        syls_off,
        syl_bytes,
        nodes: pod_slice(
            bytes,
            u32_at(bytes, OFF_NODES),
            u32_at(bytes, OFF_NODES + 4),
        ),
        words: pod_slice(
            bytes,
            u32_at(bytes, OFF_WORDS),
            u32_at(bytes, OFF_WORDS + 4),
        ),
        edges: pod_slice(
            bytes,
            u32_at(bytes, OFF_EDGES),
            u32_at(bytes, OFF_EDGES + 4),
        ),
        auto,
        auto_term,
        exact: pod_slice(
            bytes,
            u32_at(bytes, OFF_EXACT),
            u32_at(bytes, OFF_EXACT + 4),
        ),
        exact_at,
        extend: pod_slice(
            bytes,
            u32_at(bytes, OFF_EXTEND),
            u32_at(bytes, OFF_EXTEND + 4),
        ),
        extend_at,
        word_at,
        ln_corpus: f64::from_le_bytes(bytes[8..16].try_into().unwrap()),
    };
    validate(&mapped);
    mapped
}

fn validate(m: &Mapped<'_>) {
    for (offsets, bytes) in [(m.texts_off, m.text_bytes), (m.syls_off, m.syl_bytes)] {
        let text = std::str::from_utf8(bytes).expect("lexicon image contains invalid UTF-8");
        assert!(
            offsets.first() == Some(&0),
            "lexicon string table must start at zero"
        );
        assert!(
            offsets.windows(2).all(|p| p[0] <= p[1]),
            "lexicon string offsets are unordered"
        );
        assert!(
            offsets.iter().all(|&p| text.is_char_boundary(p as usize)),
            "lexicon string offset is invalid"
        );
    }
    let text_count = m.texts_off.len() - 1;
    let syl_count = m.syls_off.len() - 1;
    let range = |start: u32, len: u32, total: usize| {
        (start as usize)
            .checked_add(len as usize)
            .is_some_and(|end| end <= total)
    };
    assert!(m.ln_corpus.is_finite(), "lexicon corpus weight is invalid");
    assert!(
        !m.nodes.is_empty() && !m.auto.is_empty(),
        "lexicon root is missing"
    );
    for n in m.nodes {
        assert!(
            (n.parent == u32::MAX || (n.parent as usize) < m.nodes.len())
                && (n.syl == u16::MAX || (n.syl as usize) < syl_count),
            "lexicon node index is invalid"
        );
        assert!(
            range(n.edge_lo, n.edge_n as u32, m.edges.len())
                && range(n.word_lo, n.word_n, m.words.len()),
            "lexicon node range is invalid"
        );
    }
    assert!(
        m.words.iter().all(|w| (w.text as usize) < text_count),
        "lexicon word index is invalid"
    );
    assert!(
        m.edges
            .iter()
            .all(|e| (e.node as usize) < m.nodes.len() && (e.syl as usize) < syl_count),
        "lexicon edge index is invalid"
    );
    assert!(
        m.auto
            .iter()
            .flatten()
            .all(|&i| i == u32::MAX || (i as usize) < m.auto.len()),
        "lexicon automaton index is invalid"
    );
    assert!(
        m.auto_term
            .iter()
            .all(|&i| i == u16::MAX || (i as usize) < syl_count),
        "lexicon syllable index is invalid"
    );
    for (ranges, items) in [(m.exact_at, m.exact), (m.extend_at, m.extend)] {
        assert!(
            ranges
                .iter()
                .all(|r| r.start <= r.end && (r.end as usize) <= items.len()),
            "lexicon candidate range is invalid"
        );
        assert!(
            items.iter().all(|b| (b.text as usize) < text_count),
            "lexicon candidate index is invalid"
        );
    }
    assert!(
        m.word_at
            .iter()
            .all(|&i| i == u32::MAX || (i as usize) < m.nodes.len()),
        "lexicon reading index is invalid"
    );
}

fn push_strings(buf: &mut Vec<u8>, texts: &[String]) -> (u32, u32, u32, u32) {
    let mut offs = Vec::with_capacity(texts.len() + 1);
    let mut bytes = Vec::new();
    for text in texts {
        assert!(
            bytes.len() <= u32::MAX as usize,
            "lexicon text table overflow"
        );
        offs.push(bytes.len() as u32);
        bytes.extend_from_slice(text.as_bytes());
    }
    assert!(
        bytes.len() <= u32::MAX as usize,
        "lexicon text table overflow"
    );
    offs.push(bytes.len() as u32);
    let (off, len) = push_pods(buf, &offs);
    let (bytes_off, bytes_len) = push_bytes(buf, &bytes);
    (off, len, bytes_off, bytes_len)
}

fn push_bytes(buf: &mut Vec<u8>, bytes: &[u8]) -> (u32, u32) {
    align8(buf);
    let off = u32::try_from(buf.len()).expect("lexicon image larger than 4 GiB");
    buf.extend_from_slice(bytes);
    let len = u32::try_from(bytes.len()).expect("lexicon section larger than 4 GiB");
    (off, len)
}

fn push_pods<T: Copy>(buf: &mut Vec<u8>, items: &[T]) -> (u32, u32) {
    align8(buf);
    let off = u32::try_from(buf.len()).expect("lexicon image larger than 4 GiB");
    if !items.is_empty() {
        let raw =
            unsafe { std::slice::from_raw_parts(items.as_ptr() as *const u8, size_of_val(items)) };
        buf.extend_from_slice(raw);
    }
    let len = u32::try_from(items.len()).expect("lexicon section larger than 4 GiB");
    (off, len)
}

fn align8(buf: &mut Vec<u8>) {
    while !buf.len().is_multiple_of(8) {
        buf.push(0);
    }
}

fn put_u32(buf: &mut [u8], at: usize, value: u32) {
    buf[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_f64(buf: &mut [u8], at: usize, value: f64) {
    buf[at..at + 8].copy_from_slice(&value.to_le_bytes());
}

fn put_pair(buf: &mut [u8], at: usize, off: u32, len: u32) {
    put_u32(buf, at, off);
    put_u32(buf, at + 4, len);
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

fn byte_slice(bytes: &[u8], off: u32, len: u32) -> &[u8] {
    let start = off as usize;
    let end = start
        .checked_add(len as usize)
        .expect("lexicon section overflow");
    assert!(end <= bytes.len(), "lexicon image is truncated");
    &bytes[start..end]
}

fn pod_slice<T>(bytes: &[u8], off: u32, len: u32) -> &[T] {
    let len = len as usize;
    if len == 0 {
        return &[];
    }
    let off = off as usize;
    let nbytes = len
        .checked_mul(size_of::<T>())
        .expect("lexicon section overflow");
    assert!(
        off.is_multiple_of(align_of::<T>()),
        "lexicon image is misaligned"
    );
    assert!(
        off.checked_add(nbytes)
            .is_some_and(|end| end <= bytes.len()),
        "lexicon image is truncated"
    );
    unsafe { std::slice::from_raw_parts(bytes.as_ptr().add(off) as *const T, len) }
}

#[cfg(test)]
mod validation_tests {
    use super::*;

    fn fixture() -> Vec<u8> {
        encode(&Tables {
            texts: &["你好".into()],
            syls: &["ni".into()],
            nodes: &[NodeRec {
                parent: u32::MAX,
                edge_lo: 0,
                word_lo: 0,
                word_n: 0,
                syl: u16::MAX,
                depth: 0,
                edge_n: 0,
                _pad: 0,
            }],
            words: &[],
            edges: &[],
            auto: &[[u32::MAX; 26]],
            auto_term: &[u16::MAX],
            exact: &[],
            exact_at: &[RangeRec { start: 0, end: 0 }],
            extend: &[],
            extend_at: &[RangeRec { start: 0, end: 0 }],
            word_at: &[u32::MAX],
            ln_corpus: 1.0,
        })
    }

    fn rejected(bytes: &[u8]) {
        // Guarantee the alignment required by the mapped image interface.
        let mut aligned = vec![0u64; bytes.len().div_ceil(8)];
        let buffer = unsafe {
            std::slice::from_raw_parts_mut(aligned.as_mut_ptr().cast::<u8>(), aligned.len() * 8)
        };
        buffer[..bytes.len()].copy_from_slice(bytes);
        assert!(std::panic::catch_unwind(|| open(&buffer[..bytes.len()])).is_err());
    }

    #[test]
    fn rejects_corrupt_header_sections_and_strings() {
        let bytes = fixture();
        open(&bytes);
        for len in [0, 4, 127] {
            rejected(&bytes[..len]);
        }
        for (at, value) in [(4, 2), (OFF_TEXT, u32::MAX), (OFF_NODES + 4, u32::MAX)] {
            let mut corrupt = bytes.clone();
            put_u32(&mut corrupt, at, value);
            rejected(&corrupt);
        }
        let offset = u32_at(&bytes, OFF_TEXT_BYTES) as usize;
        let mut corrupt = bytes.clone();
        corrupt[offset] = 0xff;
        rejected(&corrupt);
        let offset = u32_at(&bytes, OFF_TEXT) as usize;
        let mut corrupt = bytes.clone();
        put_u32(&mut corrupt, offset, 1);
        rejected(&corrupt);
    }

    #[test]
    fn rejects_invalid_indexes_and_ranges() {
        let bytes = fixture();
        let mut corrupt = bytes.clone();
        let offset = u32_at(&bytes, OFF_AUTO) as usize;
        put_u32(&mut corrupt, offset, 1);
        rejected(&corrupt);
        let mut corrupt = bytes.clone();
        let offset = u32_at(&bytes, OFF_EXACT_AT) as usize;
        put_u32(&mut corrupt, offset + 4, 1);
        rejected(&corrupt);
        let mut corrupt = bytes.clone();
        let offset = u32_at(&bytes, OFF_WORD_AT) as usize;
        put_u32(&mut corrupt, offset, 20);
        rejected(&corrupt);
    }
}
