/// Terminal dimensions in cells. Both axes are always `>= 1` (CORE-R1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Size {
    width: u16,
    height: u16,
}

impl Size {
    pub const fn new(width: u16, height: u16) -> Self {
        Self {
            width: if width == 0 { 1 } else { width },
            height: if height == 0 { 1 } else { height },
        }
    }

    pub const fn width(self) -> u16 {
        self.width
    }

    pub const fn height(self) -> u16 {
        self.height
    }

    pub const fn area(self) -> usize {
        self.width as usize * self.height as usize
    }
}
