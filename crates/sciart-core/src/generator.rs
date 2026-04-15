use crate::{Canvas, Ctx, Size};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Category {
    Astronomy,
    SpaceExploration,
    /// Physics scenes; also the pre-M10 chemistry scenes, which keep this label.
    PhysicsChemistry,
    Basic,
    /// M10 (CORE-R47).
    Biology,
    /// M10 (CORE-R47).
    Chemistry,
}

impl Category {
    pub const fn label(self) -> &'static str {
        match self {
            Category::Astronomy => "astronomy",
            Category::SpaceExploration => "space-exploration",
            Category::PhysicsChemistry => "physics-chemistry",
            Category::Basic => "basic",
            Category::Biology => "biology",
            Category::Chemistry => "chemistry",
        }
    }
}

#[derive(Debug)]
pub struct Meta {
    pub name: &'static str,
    pub category: Category,
    pub description: &'static str,
}

/// A scene (CORE-R14). Call protocol (CORE-R15), guaranteed by the runtime:
/// `resize` before the first `update`/`render` and on every size change;
/// `render` receives a cleared canvas of the last resized size.
///
/// Contract (GEN-R4..R10): never panic for any size `>= 1x1`, rebuild size-dependent
/// state in `resize`, use only `ctx.rng` and `dt`/`ctx.time()`.
pub trait Generator: Send {
    fn meta(&self) -> &'static Meta;
    fn resize(&mut self, size: Size, ctx: &mut Ctx);
    fn update(&mut self, dt: f64, ctx: &mut Ctx);
    fn render(&self, canvas: &mut Canvas);
}
