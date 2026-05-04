//! `TileKey` — the universal `(z, x, y)` slippy-map tile identifier.
//!
//! Lives at the top of the tile subsystem because every layer below
//! (cache, fetch backends, decoder) speaks in these keys. Keeping the
//! type out of `cache.rs` avoids the awkwardness of fetch backends
//! reaching back into the cache module just for the address type.

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TileKey {
    pub z: u32,
    pub x: i32,
    pub y: i32,
}

impl TileKey {
    pub fn new(z: u32, x: i32, y: i32) -> Self {
        Self { z, x, y }
    }
}

impl std::fmt::Display for TileKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}/{}", self.z, self.x, self.y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_uses_zxy_form() {
        assert_eq!(TileKey::new(5, 17, 10).to_string(), "5/17/10");
    }
}
