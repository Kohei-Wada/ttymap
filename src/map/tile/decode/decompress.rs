//! gzip detection + transparent decompression for tile bodies.
//! Some tile servers (and `mapscii.me` historically) gzip their MVT
//! payloads without setting `Content-Encoding`, so we sniff the magic
//! bytes ourselves rather than relying on the HTTP layer.

use std::io::Read;

pub(super) fn maybe_decompress(buffer: &[u8]) -> Vec<u8> {
    if buffer.len() >= 2 && buffer[0] == 0x1f && buffer[1] == 0x8b {
        let mut decoder = flate2::read::GzDecoder::new(buffer);
        let mut out = Vec::new();
        // `read_to_end` writes whatever bytes the decoder emits before
        // failing, so swallowing the `Err` would leak a half-inflated
        // prefix to downstream prost — and prost can sometimes parse
        // that prefix as a valid (empty / partial) `Tile`. Drop the
        // partial output on error so the failure surfaces as "empty
        // tile" rather than "partial tile" downstream.
        match decoder.read_to_end(&mut out) {
            Ok(_) => out,
            Err(_) => Vec::new(),
        }
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
    /// not a valid stream. `GzDecoder` errors mid-read; the function
    /// must drop whatever partial bytes the decoder emitted rather
    /// than leaking them to the protobuf parser (which can sometimes
    /// parse a truncated prefix as a valid empty / partial `Tile`).
    /// Empty output deterministically signals failure to `decode()`.
    #[test]
    fn corrupt_gzip_yields_empty_not_partial_data() {
        let result = maybe_decompress(&[0x1f, 0x8b, 0xff, 0xff, 0xff]);
        assert!(
            result.is_empty(),
            "corrupt gzip must not leak partial decompressed bytes (got {} bytes)",
            result.len()
        );
    }

    /// Truncated mid-deflate-block: the gzip header parses, GzDecoder
    /// emits some uncompressed prefix, then errors. Without the fix
    /// the partial prefix would leak to the caller.
    #[test]
    fn truncated_gzip_yields_empty_not_partial_data() {
        // Build a real gzip stream and chop off the trailing CRC + size
        // footer (last 8 bytes) — the body is still being inflated when
        // the truncation hits.
        let original = b"hello world ".repeat(64);
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&original).unwrap();
        let full = encoder.finish().unwrap();
        let truncated = &full[..full.len() - 8];
        assert_eq!(&truncated[..2], &[0x1f, 0x8b]);
        let result = maybe_decompress(truncated);
        assert!(
            result.is_empty(),
            "truncated gzip must drop the half-inflated prefix (got {} bytes)",
            result.len()
        );
    }
}
