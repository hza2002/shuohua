use ratatui::layout::Rect;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitTarget {
    Module(usize),
    Source(usize),
    Field(usize),
}

#[derive(Debug, Default, Clone)]
pub struct HitRegions {
    pub modules: Vec<(Rect, usize)>,
    pub sources: Vec<(Rect, usize)>,
    pub fields: Vec<(Rect, usize)>,
    /// The selected-field detail pane; wheel here scrolls its text rather than
    /// moving the field selection.
    pub detail: Option<Rect>,
}

impl HitRegions {
    pub fn detail_contains(&self, x: u16, y: u16) -> bool {
        self.detail.is_some_and(|r| contains(r, x, y))
    }
}

fn contains(r: Rect, x: u16, y: u16) -> bool {
    x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height
}

impl HitRegions {
    pub fn clear(&mut self) {
        self.modules.clear();
        self.sources.clear();
        self.fields.clear();
        self.detail = None;
    }

    pub fn hit(&self, x: u16, y: u16) -> Option<HitTarget> {
        for (r, i) in &self.fields {
            if contains(*r, x, y) {
                return Some(HitTarget::Field(*i));
            }
        }
        for (r, i) in &self.sources {
            if contains(*r, x, y) {
                return Some(HitTarget::Source(*i));
            }
        }
        for (r, i) in &self.modules {
            if contains(*r, x, y) {
                return Some(HitTarget::Module(*i));
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;

    #[test]
    fn hit_finds_field_row_by_point() {
        let mut h = HitRegions::default();
        h.fields.push((Rect::new(24, 5, 50, 1), 0));
        h.fields.push((Rect::new(24, 6, 50, 1), 1));
        assert_eq!(h.hit(30, 6), Some(HitTarget::Field(1)));
        assert_eq!(h.hit(0, 0), None);
    }

    #[test]
    fn hit_prefers_fields_over_sources() {
        let mut h = HitRegions::default();
        h.sources.push((Rect::new(24, 1, 60, 1), 0));
        h.fields.push((Rect::new(24, 1, 60, 1), 3)); // overlaps source
        assert_eq!(h.hit(30, 1), Some(HitTarget::Field(3)));
    }
}
