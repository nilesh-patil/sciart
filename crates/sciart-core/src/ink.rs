/// What a cell *means*; themes turn roles into colors (ADR-0004, CORE-R8).
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    Background,
    Dim,
    Star,
    Sun,
    Planet,
    Particle,
    Wave,
    Hot,
    Cold,
    Structure,
    Accent,
    Text,
}

/// Semantic style of a cell: a role plus an intensity `0..=255`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Ink {
    pub role: Role,
    pub level: u8,
}

impl Ink {
    pub const BLANK: Ink = Ink::new(Role::Background, 0);

    pub const fn new(role: Role, level: u8) -> Self {
        Self { role, level }
    }
}

impl Default for Ink {
    fn default() -> Self {
        Self::BLANK
    }
}
