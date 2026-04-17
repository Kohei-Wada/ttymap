use crate::geo::LonLat;

#[derive(Clone, Copy)]
pub struct RenderRequest {
    pub center: LonLat,
    pub zoom: f64,
    pub width: usize,
    pub height: usize,
}
