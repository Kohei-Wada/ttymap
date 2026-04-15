use rstar::{RTree, RTreeObject, AABB};

#[derive(Debug, Clone)]
struct LabelEntry {
    min_x: f64,
    min_y: f64,
    max_x: f64,
    max_y: f64,
}

impl RTreeObject for LabelEntry {
    type Envelope = AABB<[f64; 2]>;

    fn envelope(&self) -> Self::Envelope {
        AABB::from_corners([self.min_x, self.min_y], [self.max_x, self.max_y])
    }
}

pub struct LabelBuffer {
    tree: RTree<LabelEntry>,
    default_margin: f64,
}

impl LabelBuffer {
    pub fn new() -> Self {
        LabelBuffer {
            tree: RTree::new(),
            default_margin: 5.0,
        }
    }

    pub fn clear(&mut self) {
        self.tree = RTree::new();
    }

    pub fn write_if_possible(&mut self, text: &str, x: f64, y: f64, margin: Option<f64>) -> bool {
        if self.has_space(text, x, y, margin) {
            let entry = Self::calculate_area(text, x, y, margin.unwrap_or(self.default_margin));
            self.tree.insert(entry);
            true
        } else {
            false
        }
    }

    fn has_space(&self, text: &str, x: f64, y: f64, margin: Option<f64>) -> bool {
        let area = Self::calculate_area(text, x, y, margin.unwrap_or(self.default_margin));
        let envelope = area.envelope();
        self.tree.locate_in_envelope_intersecting(&envelope).next().is_none()
    }

    fn calculate_area(text: &str, x: f64, y: f64, margin: f64) -> LabelEntry {
        LabelEntry {
            min_x: x - margin,
            min_y: y - margin / 2.0,
            max_x: x + margin + text.len() as f64,
            max_y: y + margin / 2.0,
        }
    }
}

impl Default for LabelBuffer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_label_buffer() {
        let buf = LabelBuffer::new();
        assert_eq!(buf.default_margin, 5.0);
    }

    #[test]
    fn test_first_label_always_fits() {
        let mut buf = LabelBuffer::new();
        let result = buf.write_if_possible("Hello", 0.0, 0.0, None);
        assert!(result);
    }

    #[test]
    fn test_overlapping_labels_rejected() {
        let mut buf = LabelBuffer::new();
        let first = buf.write_if_possible("Hello", 10.0, 10.0, None);
        let second = buf.write_if_possible("Hello", 10.0, 10.0, None);
        assert!(first);
        assert!(!second);
    }

    #[test]
    fn test_distant_labels_both_fit() {
        let mut buf = LabelBuffer::new();
        let first = buf.write_if_possible("Hi", 0.0, 0.0, None);
        let second = buf.write_if_possible("Hi", 1000.0, 1000.0, None);
        assert!(first);
        assert!(second);
    }

    #[test]
    fn test_clear_allows_reuse() {
        let mut buf = LabelBuffer::new();
        buf.write_if_possible("Hello", 10.0, 10.0, None);
        buf.clear();
        let result = buf.write_if_possible("Hello", 10.0, 10.0, None);
        assert!(result);
    }
}
