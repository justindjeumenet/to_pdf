//! PDF 1.5 object streams and cross-reference streams.
//!
//! Packing every non-stream object into one `/Type/ObjStm` and replacing the
//! classic xref table with a `/Type/XRef` stream is the bulk of the structural
//! size saving: a classic table costs exactly 20 uncompressed bytes per object.

use super::object::{CompressedObj, flate};

/// Pack `objs` into one object stream. Returns the dictionary body (without
/// `/Length`, which [`super::object::stream_object`] adds) and the compressed
/// payload. The caller must record each object's index, which is its position
/// in `objs`.
pub fn object_stream(objs: &[CompressedObj]) -> (String, Vec<u8>) {
    let mut header = String::new();
    let mut bodies = String::new();
    for o in objs {
        header.push_str(&format!("{} {} ", o.id, bodies.len()));
        bodies.push_str(&o.body);
        bodies.push('\n');
    }
    let first = header.len();
    let mut raw = header;
    raw.push_str(&bodies);
    let data = flate(raw.as_bytes());
    let dict = format!(
        "/Type/ObjStm/N {}/First {first}/Filter/FlateDecode",
        objs.len()
    );
    (dict, data)
}

/// One cross-reference entry, in the `/W [1 4 2]` shape.
#[derive(Copy, Clone)]
pub enum Entry {
    /// Type 0: the head of the free list.
    Free,
    /// Type 1: an object written at the top level, at this byte offset.
    InFile(u64),
    /// Type 2: an object living inside object stream `stm` at index `idx`.
    InStream { stm: u32, idx: u32 },
}

/// Build the compressed `/W [1 4 2]` cross-reference payload: one type byte,
/// a big-endian u32, and a big-endian u16 per entry.
pub fn xref_data(entries: &[Entry]) -> Vec<u8> {
    let mut raw = Vec::with_capacity(entries.len() * 7);
    for e in entries {
        let (t, f2, f3): (u8, u32, u16) = match *e {
            Entry::Free => (0, 0, 65535),
            Entry::InFile(off) => (1, off as u32, 0),
            Entry::InStream { stm, idx } => (2, stm, idx as u16),
        };
        raw.push(t);
        raw.extend_from_slice(&f2.to_be_bytes());
        raw.extend_from_slice(&f3.to_be_bytes());
    }
    flate(&raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pdf::object::CompressedObj;
    use std::io::Read;

    fn inflate(b: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        flate2::read::ZlibDecoder::new(b)
            .read_to_end(&mut out)
            .unwrap();
        out
    }

    #[test]
    fn object_stream_header_indexes_each_body() {
        let objs = vec![
            CompressedObj {
                id: 1,
                body: "<</Type/Catalog>>".into(),
            },
            CompressedObj {
                id: 2,
                body: "<</Type/Pages>>".into(),
            },
        ];
        let (dict, data) = object_stream(&objs);
        assert!(dict.contains("/Type/ObjStm"));
        assert!(dict.contains("/N 2"));
        assert!(dict.contains("/Filter/FlateDecode"));

        let raw = String::from_utf8(inflate(&data)).unwrap();
        let first: usize = dict
            .split("/First ")
            .nth(1)
            .unwrap()
            .split('/')
            .next()
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert_eq!(&raw[..first], "1 0 2 18 ");
        assert!(raw[first..].starts_with("<</Type/Catalog>>"));
    }

    #[test]
    fn object_stream_is_smaller_than_the_bodies_for_repetitive_input() {
        let objs: Vec<CompressedObj> = (4..204)
            .map(|i| CompressedObj {
                id: i,
                body: format!("<</Type/Page/Parent 2 0 R/Contents {} 0 R>>", i + 200),
            })
            .collect();
        let raw_len: usize = objs.iter().map(|o| o.body.len() + 1).sum();
        let (_, data) = object_stream(&objs);
        assert!(
            data.len() < raw_len / 4,
            "200 page dicts must compress hard"
        );
    }

    #[test]
    fn xref_entries_are_seven_bytes_each_before_compression() {
        let entries = vec![
            Entry::Free,
            Entry::InStream { stm: 9, idx: 0 },
            Entry::InFile(1234),
        ];
        let raw = inflate(&xref_data(&entries));
        assert_eq!(raw.len(), 21);
        assert_eq!(&raw[0..7], &[0, 0, 0, 0, 0, 0xFF, 0xFF]);
        assert_eq!(&raw[7..14], &[2, 0, 0, 0, 9, 0, 0]);
        assert_eq!(&raw[14..21], &[1, 0, 0, 0x04, 0xD2, 0, 0]);
    }

    #[test]
    fn xref_beats_a_classic_table_on_size() {
        let entries: Vec<Entry> = (0..1000).map(|i| Entry::InFile(i * 97)).collect();
        assert!(xref_data(&entries).len() < 20 * entries.len() / 4);
    }
}
