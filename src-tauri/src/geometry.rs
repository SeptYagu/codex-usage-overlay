//! Shared physical-pixel geometry and multi-monitor work-area selection.

/// Axis-aligned rectangle in integer physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhysicalRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl PhysicalRect {
    /// Right edge in physical pixels (`i64` avoids overflow on virtual desktop bounds).
    pub fn right(&self) -> i64 {
        self.x as i64 + self.width as i64
    }

    pub fn bottom(&self) -> i64 {
        self.y as i64 + self.height as i64
    }

    pub fn contains_point(&self, x: f64, y: f64) -> bool {
        let left = self.x as f64;
        let top = self.y as f64;
        x >= left && x < left + self.width as f64 && y >= top && y < top + self.height as f64
    }

    /// Overlapping area in square physical pixels; zero when touching or separated.
    pub fn intersection_area(&self, other: &PhysicalRect) -> i64 {
        let width = (self.right().min(other.right()) - (self.x as i64).max(other.x as i64)).max(0);
        let height = (self.bottom().min(other.bottom()) - (self.y as i64).max(other.y as i64)).max(0);
        width * height
    }
}

/// Axis-aligned rectangle in floating-point physical pixels (for DPI-scaled projections).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PhysicalRectF {
    pub left: f64,
    pub top: f64,
    pub width: f64,
    pub height: f64,
}

impl PhysicalRectF {
    pub fn new(left: f64, top: f64, width: f64, height: f64) -> Self {
        Self { left, top, width, height }
    }

    pub fn right(&self) -> f64 {
        self.left + self.width
    }

    pub fn bottom(&self) -> f64 {
        self.top + self.height
    }

    /// Overlapping area with `other`; zero when they only touch or miss entirely.
    pub fn intersection_area(&self, other: &PhysicalRectF) -> f64 {
        let width = (self.right().min(other.right()) - self.left.max(other.left)).max(0.0);
        let height = (self.bottom().min(other.bottom()) - self.top.max(other.top)).max(0.0);
        width * height
    }
}

/// Monitor work area in physical pixels, decoupled from `tauri::Monitor`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MonitorWorkArea {
    pub left: f64,
    pub top: f64,
    pub width: f64,
    pub height: f64,
    pub scale_factor: f64,
}

impl MonitorWorkArea {
    pub fn rect(&self) -> PhysicalRectF {
        PhysicalRectF::new(self.left, self.top, self.width, self.height)
    }

    pub fn has_usable_scale(&self) -> bool {
        self.scale_factor.is_finite() && self.scale_factor > 0.0
    }
}

pub fn monitor_bounds(monitor: &tauri::Monitor) -> (PhysicalRect, f64) {
    let area = monitor.work_area();
    (
        PhysicalRect {
            x: area.position.x,
            y: area.position.y,
            width: area.size.width,
            height: area.size.height,
        },
        monitor.scale_factor(),
    )
}

pub fn monitor_work_area(monitor: &tauri::Monitor) -> MonitorWorkArea {
    let work = monitor.work_area();
    MonitorWorkArea {
        left: work.position.x as f64,
        top: work.position.y as f64,
        width: work.size.width as f64,
        height: work.size.height as f64,
        scale_factor: monitor.scale_factor(),
    }
}

/// Picks the host monitor for a released window by intersecting it with every work
/// area, preferring `fallback_index` on exact ties and smallest `(x, y)` otherwise.
pub fn select_monitor_by_overlap(
    win: PhysicalRect,
    monitors: &[(PhysicalRect, f64)],
    fallback_index: usize,
) -> Option<(PhysicalRect, f64)> {
    if monitors.is_empty() {
        return None;
    }
    let host = if fallback_index < monitors.len() { fallback_index } else { 0 };
    let mut best_area = 0i64;
    for (work, _) in monitors {
        best_area = best_area.max(win.intersection_area(work));
    }
    if best_area == 0 {
        return monitors.get(host).copied();
    }
    let tied: Vec<usize> = monitors
        .iter()
        .enumerate()
        .filter(|(_, (work, _))| win.intersection_area(work) == best_area)
        .map(|(index, _)| index)
        .collect();
    let index = if tied.contains(&fallback_index) {
        fallback_index
    } else {
        *tied
            .iter()
            .min_by_key(|&&index| (monitors[index].0.x, monitors[index].0.y))
            .unwrap_or(&tied[0])
    };
    Some(monitors[index])
}

/// Picks the work area that contains `point`, falling back to `fallback` and then
/// the first monitor when the point lies outside every display.
pub fn select_work_area(
    monitors: &[(PhysicalRect, f64)],
    fallback: usize,
    point: Option<(f64, f64)>,
) -> Option<(PhysicalRect, f64)> {
    if let Some((x, y)) = point {
        if let Some(found) = monitors
            .iter()
            .find(|(rect, _)| rect.contains_point(x, y))
        {
            return Some(*found);
        }
    }
    monitors
        .get(fallback)
        .or_else(|| monitors.first())
        .copied()
}

/// Picks the index of the usable-scale work area overlapping `project(work)` by the
/// largest non-zero area.
pub fn select_best_work_area_index(
    work_areas: &[MonitorWorkArea],
    project: impl Fn(&MonitorWorkArea) -> PhysicalRectF,
) -> Option<usize> {
    let mut best: Option<(usize, f64)> = None;
    for (index, work) in work_areas.iter().enumerate() {
        if !work.has_usable_scale() {
            continue;
        }
        let area = project(work).intersection_area(&work.rect());
        let is_better = best.is_none_or(|(_, best_area)| area > best_area);
        if area > 0.0 && is_better {
            best = Some((index, area));
        }
    }
    best.map(|(index, _)| index)
}
