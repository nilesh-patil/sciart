use crate::{Canvas, Ctx, Size};

/// Where a scene belongs (CORE-R77, ADR-0013). Sky scenes are sorted by setting (Voyages,
/// then Earth & Sky, then Sun & Planets, then Deep Cosmos); the rest by which of physics (P),
/// chemistry (C) and biology (B) its model needs. Variant order is display order only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Category {
    /// Deep space: galaxies, clusters, black holes, the early universe.
    DeepCosmos,
    /// The Sun, and planets, moons and comets (including exoplanets).
    SunPlanets,
    /// The sky as seen from Earth's surface or atmosphere.
    EarthSky,
    /// Human spacecraft and missions.
    Voyages,
    /// P: a physical law (forces, flow, fields, waves, statistical or quantum mechanics,
    /// deterministic chaos).
    Physics,
    /// C: chemical kinetics, molecular structure or a phase change (incl. ordering).
    Chemistry,
    /// B: an organism, tissue, cell or artificial life.
    Biology,
    /// P×C.
    PhysicalChemistry,
    /// P×B.
    Biophysics,
    /// C×B.
    Biochemistry,
    /// P×C×B.
    LivingMatter,
}

impl Category {
    /// Every category, in display order.
    pub const ALL: [Category; 11] = [
        Category::DeepCosmos,
        Category::SunPlanets,
        Category::EarthSky,
        Category::Voyages,
        Category::Physics,
        Category::Chemistry,
        Category::Biology,
        Category::PhysicalChemistry,
        Category::Biophysics,
        Category::Biochemistry,
        Category::LivingMatter,
    ];

    /// The label in kebab case (`&` dropped), printed by `sciart list` (CLI-R4), so the
    /// machine id and the name people see never drift apart.
    pub const fn id(self) -> &'static str {
        match self {
            Category::DeepCosmos => "deep-cosmos",
            Category::SunPlanets => "sun-planets",
            Category::EarthSky => "earth-sky",
            Category::Voyages => "voyages",
            Category::Physics => "forces-waves",
            Category::Chemistry => "reactions",
            Category::Biology => "living-systems",
            Category::PhysicalChemistry => "states-of-matter",
            Category::Biophysics => "physics-of-life",
            Category::Biochemistry => "molecules-of-life",
            Category::LivingMatter => "life-from-matter",
        }
    }

    /// Display name, as written in SPEC-002's scene headings (GEN-R15).
    pub const fn label(self) -> &'static str {
        match self {
            Category::DeepCosmos => "Deep Cosmos",
            Category::SunPlanets => "Sun & Planets",
            Category::EarthSky => "Earth & Sky",
            Category::Voyages => "Voyages",
            Category::Physics => "Forces & Waves",
            Category::Chemistry => "Reactions",
            Category::Biology => "Living Systems",
            Category::PhysicalChemistry => "States of Matter",
            Category::Biophysics => "Physics of Life",
            Category::Biochemistry => "Molecules of Life",
            Category::LivingMatter => "Life from Matter",
        }
    }
}

#[derive(Debug)]
pub struct Meta {
    pub name: &'static str,
    pub category: Category,
    pub description: &'static str,
    /// `Phenomenon · mechanism`, drawn by `Stage` (CORE-R71; content rules GEN-R12).
    pub title: &'static str,
    /// Every caption [`Generator::caption`] can return; empty for a single-act scene.
    pub captions: &'static [&'static str],
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

    /// Called by `Stage::render` exactly once right before every `render` (CORE-R101):
    /// the place to rasterise simulation state into the scene's buffers once per drawn
    /// frame rather than once per step (the runtime takes about two 60 Hz steps per
    /// 30 FPS frame; GEN-R20). Must not advance the simulation or draw from `ctx.rng`,
    /// and must not allocate (GEN-R11). Default: nothing.
    fn prepare(&mut self) {}

    fn render(&self, canvas: &mut Canvas);

    /// The live act's caption, which replaces the title's mechanism; an entry of
    /// `meta().captions` (CORE-R71).
    fn caption(&self) -> Option<&'static str> {
        None
    }
}
