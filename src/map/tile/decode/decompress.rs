//! gzip detection + transparent decompression for tile bodies.
//! Some tile servers (and `mapscii.me` historically) gzip their MVT
//! payloads without setting `Content-Encoding`, so we sniff the magic
//! bytes ourselves rather than relying on the HTTP layer.

use std::io::Read;

pub(super) fn maybe_decompress(buffer: &[u8]) -> Vec<u8> {
    if buffer.len() >= 2 && buffer[0] == 0x1f && buffer[1] == 0x8b {
        let mut decoder = flate2::read::GzDecoder::new(buffer);
        let mut out = Vec::new();
        decoder.read_to_end(&mut out).unwrap_or(0);
        out
    } else {
        buffer.to_vec()
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use flate2::Compression;
    use flate2::write::GzEncoder;

    use super::*;

    #[test]
    fn passes_through_uncompressed_bytes_unchanged() {
        let data = b"not gzipped data";
        assert_eq!(maybe_decompress(data), data.to_vec());
    }

    #[test]
    fn decompresses_gzipped_bytes() {
        let original = b"hello, world".repeat(8);
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&original).unwrap();
        let gzipped = encoder.finish().unwrap();
        // Sniff guard
        assert_eq!(&gzipped[..2], &[0x1f, 0x8b]);
        assert_eq!(maybe_decompress(&gzipped), original);
    }

    #[test]
    fn empty_input_passes_through() {
        assert_eq!(maybe_decompress(&[]), Vec::<u8>::new());
    }

    /// Buffer that *starts* with the gzip magic bytes but is otherwise
    /// not a valid stream — `GzDecoder` returns an error mid-read, and
    /// our `unwrap_or(0)` swallows it; the caller gets whatever (if
    /// any) bytes the decoder managed to emit before the error. Must
    /// not panic.
    #[test]
    fn corrupt_gzip_does_not_panic() {
        let _ = maybe_decompress(&[0x1f, 0x8b, 0xff, 0xff, 0xff]);
    }
}
