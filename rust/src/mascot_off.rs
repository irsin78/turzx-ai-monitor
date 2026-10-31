//! Build without mascot art (see build.rs): the same interface, nothing drawn.

use tiny_skia::Pixmap;

pub const ENABLED: bool = false;

/// Where a mascot may roam: x range of its feet and the ground line.
#[derive(Clone, Copy)]
pub struct Area {
    pub x0: f32,
    pub x1: f32,
    pub ground: f32,
}

pub struct Mascots;

impl Mascots {
    pub fn new(_areas: &[Area; 3]) -> Self {
        Mascots
    }
    pub fn update(&mut self, _dt: f32, _areas: &[Area; 3]) {}
    pub fn draw(&self, _pm: &mut Pixmap, _areas: &[Area; 3]) {}
    /// No mascots: the status bubbles point at the middle of the empty strips.
    pub fn heads(&self, areas: &[Area; 3]) -> [(f32, f32); 3] {
        [0, 1, 2].map(|i| ((areas[i].x0 + areas[i].x1) / 2.0, areas[i].ground - 40.0))
    }
}
