//! Real mouse movement via enigo (cargo feature `mouse`, KA-R4).
//! On macOS the terminal needs Accessibility permission; failures surface as
//! `mouse:err(n)` in the HUD rather than stopping the app.

use enigo::{Coordinate, Enigo, Mouse, Settings};

use crate::Mover;

pub struct EnigoMover(Enigo);

impl std::fmt::Debug for EnigoMover {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("EnigoMover")
    }
}

impl EnigoMover {
    pub fn new() -> Result<Self, String> {
        Enigo::new(&Settings::default())
            .map(Self)
            .map_err(|e| e.to_string())
    }
}

impl Mover for EnigoMover {
    fn screen_size(&mut self) -> Result<(i32, i32), String> {
        self.0.main_display().map_err(|e| e.to_string())
    }

    fn move_to(&mut self, x: i32, y: i32) -> Result<(), String> {
        self.0
            .move_mouse(x, y, Coordinate::Abs)
            .map_err(|e| e.to_string())
    }
}
