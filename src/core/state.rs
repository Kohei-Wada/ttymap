use log::debug;

use super::config::Config;
use super::input::Action;
use super::snapshot::RenderRequest;
use crate::geo::{self, LonLat};

pub struct Core {
    config: Config,
    center: LonLat,
    zoom: f64,
    min_zoom: f64,
    width: usize,
    height: usize,
    running: bool,
}

impl Core {
    pub fn new(config: Config, width: usize, height: usize) -> Self {
        let min_zoom = Self::calculate_min_zoom(width);
        let zoom = config.initial_zoom.unwrap_or(min_zoom);
        let center = LonLat {
            lon: config.initial_lon,
            lat: config.initial_lat,
        };

        Core {
            config,
            center,
            zoom,
            min_zoom,
            width,
            height,
            running: true,
        }
    }

    pub fn is_running(&self) -> bool {
        self.running
    }
    pub fn width(&self) -> usize {
        self.width
    }
    pub fn height(&self) -> usize {
        self.height
    }
    pub fn stop(&mut self) {
        self.running = false;
    }
    pub fn zoom_step(&self) -> f64 {
        self.config.zoom_step
    }

    /// Process a map action. Returns true if redraw needed.
    pub fn process_action(&mut self, action: &Action) -> bool {
        let step = 8.0 / 2.0_f64.powf(self.zoom);
        let zoom_step = self.config.zoom_step;
        let max_zoom = self.config.max_zoom;

        match action {
            Action::None | Action::SearchOpen | Action::HelpToggle | Action::WikiToggle => false,
            Action::Quit => {
                debug!("action: Quit");
                self.running = false;
                false
            }
            Action::PanLeft => self.pan(step, -1.0, 0.0),
            Action::PanRight => self.pan(step, 1.0, 0.0),
            Action::PanUp => self.pan(step, 0.0, 0.75),
            Action::PanDown => self.pan(step, 0.0, -0.75),
            Action::PanLeftFast => self.pan(step, -10.0, 0.0),
            Action::PanRightFast => self.pan(step, 10.0, 0.0),
            Action::PanUpHalf => self.pan(step, 0.0, 7.5),
            Action::PanDownHalf => self.pan(step, 0.0, -7.5),
            Action::ZoomIn => {
                self.zoom = (self.zoom + zoom_step).min(max_zoom);
                true
            }
            Action::ZoomOut => {
                self.zoom = (self.zoom - zoom_step).max(self.min_zoom);
                true
            }
            Action::ZoomToWorld => {
                self.zoom = self.min_zoom;
                true
            }
            Action::ResetPosition => {
                self.center = LonLat {
                    lon: self.config.initial_lon,
                    lat: self.config.initial_lat,
                };
                self.zoom = self.config.initial_zoom.unwrap_or(self.min_zoom);
                true
            }
            Action::Redraw => true,
        }
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        let (w, h) = crate::render::canvas_size(cols, rows);
        self.width = w;
        self.height = h;
        self.min_zoom = Self::calculate_min_zoom(self.width);
        self.zoom = self.zoom.clamp(self.min_zoom, self.config.max_zoom);
    }

    pub fn render_request(&self) -> RenderRequest {
        RenderRequest {
            center: self.center,
            zoom: self.zoom,
            width: self.width,
            height: self.height,
        }
    }

    pub fn status_bar(&self) -> String {
        format!(
            " {:.3}, {:.3}  zoom: {:.1}",
            self.center.lat, self.center.lon, self.zoom
        )
    }

    fn pan(&mut self, step: f64, dlon: f64, dlat: f64) -> bool {
        self.center.lon += step * dlon;
        self.center.lat += step * dlat;
        self.center = geo::normalize(self.center);
        true
    }

    /// Pan the map by terminal cell offsets (for mouse drag).
    /// dx/dy are in terminal cells (not pixels).
    pub fn pan_by_cells(&mut self, dx: i16, dy: i16) {
        // Each cell = 2 braille pixels wide, 4 braille pixels tall.
        // Convert cell offset to pixel offset, then to degrees.
        let tile_size = geo::tile_size_at_zoom(self.zoom);
        let z = geo::base_zoom(self.zoom);
        let n = (1u64 << z) as f64;

        // Degrees per pixel
        let lon_per_px = 360.0 / (n * tile_size);
        let lat_per_px = 360.0 / (n * tile_size) * self.center.lat.to_radians().cos();

        self.center.lon -= dx as f64 * 2.0 * lon_per_px;
        self.center.lat += dy as f64 * 4.0 * lat_per_px;
        self.center = geo::normalize(self.center);
    }

    /// Zoom in/out by a delta amount.
    pub fn zoom_by(&mut self, delta: f64) {
        self.zoom = (self.zoom + delta).clamp(self.min_zoom, self.config.max_zoom);
    }

    /// Zoom towards a screen position (in terminal cells relative to center).
    /// Keeps the point under the cursor fixed on screen.
    pub fn zoom_towards(&mut self, dx_cells: f64, dy_cells: f64, delta: f64) {
        let old_zoom = self.zoom;
        self.zoom_by(delta);
        let new_zoom = self.zoom;
        if (new_zoom - old_zoom).abs() < 1e-10 {
            return;
        }

        let ratio = 1.0 - 2.0_f64.powf(old_zoom - new_zoom);
        // pan_by_cells subtracts dx (drag convention), so negate for "move towards"
        self.pan_by_cells(-(dx_cells * ratio) as i16, -(dy_cells * ratio) as i16);
    }

    /// Move the map center to the given location.
    pub fn jump_to(&mut self, location: LonLat) {
        self.center = geo::normalize(location);
    }

    fn calculate_min_zoom(width: usize) -> f64 {
        4.0 - (4096.0 / width as f64).ln() / 2.0_f64.ln()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_core() -> Core {
        Core::new(Config::default(), 160, 92)
    }

    #[test]
    fn test_quit() {
        let mut core = default_core();
        assert!(core.is_running());
        core.process_action(&Action::Quit);
        assert!(!core.is_running());
    }

    #[test]
    fn test_pan() {
        let mut core = default_core();
        let before = core.center.lon;
        core.process_action(&Action::PanRight);
        assert!(core.center.lon > before);
    }

    #[test]
    fn test_zoom_in_out() {
        let mut core = default_core();
        for _ in 0..5 {
            core.process_action(&Action::ZoomIn);
        }
        let after_in = core.zoom;
        core.process_action(&Action::ZoomOut);
        assert!(core.zoom < after_in);
    }

    #[test]
    fn test_resize() {
        let mut core = default_core();
        core.resize(120, 40);
        let (expected_w, expected_h) = crate::render::canvas_size(120, 40);
        assert_eq!(core.width(), expected_w);
        assert_eq!(core.height(), expected_h);
    }

    #[test]
    fn test_reset_position() {
        let mut core = default_core();
        core.process_action(&Action::PanRight);
        let moved = core.center.lon;
        core.process_action(&Action::ResetPosition);
        assert_ne!(core.center.lon, moved);
        assert_eq!(core.center.lon, core.config.initial_lon);
    }
}
