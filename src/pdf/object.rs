//! Serialization primitives for PDF indirect objects.

use flate2::{Compression, write::ZlibEncoder};
use std::io::Write;

/// Zlib-compress `data` at the highest level. `/FlateDecode` expects the zlib
/// wrapper, not a raw deflate stream.
pub fn flate(data: &[u8]) -> Vec<u8> {
    let mut e = ZlibEncoder::new(Vec::new(), Compression::best());
    e.write_all(data).expect("writing to a Vec cannot fail");
    e.finish().expect("finishing into a Vec cannot fail")
}

/// An indirect object with no stream data, so it can be packed into an
/// object stream rather than written at the top level of the file.
pub struct CompressedObj {
    pub id: u32,
    /// The complete object body, e.g. `<</Type/Catalog/Pages 2 0 R>>`.
    pub body: String,
}

/// Serialize a top-level stream object. `dict_body` is spliced between `<<`
/// and the generated `/Length`, e.g. `"/Type/ObjStm/N 4/First 20"`.
pub fn stream_object(id: u32, dict_body: &str, data: &[u8]) -> Vec<u8> {
    let head = format!("{id} 0 obj\n<<{dict_body}/Length {}>>\nstream\n", data.len());
    let mut out = Vec::with_capacity(head.len() + data.len() + 20);
    out.extend_from_slice(head.as_bytes());
    out.extend_from_slice(data);
    out.extend_from_slice(b"\nendstream\nendobj\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn flate_round_trips() {
        let src = b"BT /F0 8.5 Tf (hello) ' ET".repeat(40);
        let packed = flate(&src);
        assert!(packed.len() < src.len(), "compression should shrink repetitive input");
        let mut d = flate2::read::ZlibDecoder::new(&packed[..]);
        let mut back = Vec::new();
        d.read_to_end(&mut back).unwrap();
        assert_eq!(back, src);
    }

    #[test]
    fn flate_is_deterministic() {
        assert_eq!(flate(b"the same bytes"), flate(b"the same bytes"));
    }

    #[test]
    fn stream_object_has_length_and_delimiters() {
        let out = stream_object(7, "/Filter/FlateDecode", b"abcd");
        let s = String::from_utf8_lossy(&out);
        assert!(s.starts_with("7 0 obj\n<</Filter/FlateDecode/Length 4>>\nstream\n"));
        assert!(s.ends_with("\nendstream\nendobj\n"));
        assert!(s.contains("abcd"));
    }

    #[test]
    fn stream_object_length_counts_raw_bytes_not_chars() {
        let data = vec![0x00u8, 0x80, 0xFF, 0x0A];
        let out = stream_object(1, "/Type/XRef", &data);
        assert!(String::from_utf8_lossy(&out).contains("/Length 4>>"));
    }
}
