//! The scene catalogue: names, titles and categories (LIB-R1).

use std::fmt;

use sciart_core::Meta;

/// One registered scene: its name and what `sciart list` shows about it.
///
/// Get one from [`scenes`] or [`find`]; pass its [`name`](SceneInfo::name) to
/// [`Stage::new`](crate::Stage::new).
#[derive(Clone, Copy)]
pub struct SceneInfo {
    meta: &'static Meta,
}

impl SceneInfo {
    pub(crate) fn of(meta: &'static Meta) -> Self {
        Self { meta }
    }

    /// The scene's name, e.g. `aurora`: what `sciart run` takes. Names are stable; a
    /// renamed scene keeps its old name as an alias that [`find`] resolves.
    pub fn name(&self) -> &'static str {
        self.meta.name
    }

    /// The science title drawn at the bottom left, `Phenomenon · mechanism`.
    pub fn title(&self) -> &'static str {
        self.meta.title
    }

    /// A one-line description.
    pub fn description(&self) -> &'static str {
        self.meta.description
    }

    /// Where the scene belongs.
    pub fn category(&self) -> Category {
        Category::of(self.meta.category)
    }

    /// The captions of a scene told in acts, in order; empty for a single-act scene. A
    /// live caption replaces the mechanism half of the title.
    pub fn captions(&self) -> &'static [&'static str] {
        self.meta.captions
    }
}

impl PartialEq for SceneInfo {
    fn eq(&self, other: &Self) -> bool {
        self.meta.name == other.meta.name
    }
}

impl Eq for SceneInfo {}

impl std::hash::Hash for SceneInfo {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.meta.name.hash(state);
    }
}

impl fmt::Debug for SceneInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("SceneInfo").field(&self.meta.name).finish()
    }
}

/// Every scene, sorted by name. Retired names are not listed.
pub fn scenes() -> impl ExactSizeIterator<Item = SceneInfo> {
    sciart_generators::registry()
        .iter()
        .map(|entry| SceneInfo::of(entry.meta))
}

/// The scene called `name`, or the scene a retired name now points to.
pub fn find(name: &str) -> Option<SceneInfo> {
    sciart_generators::find(name).map(|entry| SceneInfo::of(entry.meta))
}

/// Where a scene belongs: sky scenes by setting, the rest by which of physics, chemistry
/// and biology their model needs. The variant order is the display order.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Category {
    /// Galaxies, clusters, black holes, the early universe.
    DeepCosmos,
    /// The Sun, planets, moons and comets, including exoplanets.
    SunPlanets,
    /// The sky seen from Earth's surface or atmosphere.
    EarthSky,
    /// Spacecraft and missions.
    Voyages,
    /// A physical law: forces, flow, fields, waves, statistical or quantum mechanics.
    ForcesWaves,
    /// Chemical kinetics, molecular structure or a phase change.
    Reactions,
    /// An organism, tissue, cell or artificial life.
    LivingSystems,
    /// Physics and chemistry.
    StatesOfMatter,
    /// Physics and biology.
    PhysicsOfLife,
    /// Chemistry and biology.
    MoleculesOfLife,
    /// Physics, chemistry and biology.
    LifeFromMatter,
}

impl Category {
    const ALL: [Category; 11] = [
        Category::DeepCosmos,
        Category::SunPlanets,
        Category::EarthSky,
        Category::Voyages,
        Category::ForcesWaves,
        Category::Reactions,
        Category::LivingSystems,
        Category::StatesOfMatter,
        Category::PhysicsOfLife,
        Category::MoleculesOfLife,
        Category::LifeFromMatter,
    ];

    /// Every category, in display order.
    pub fn all() -> &'static [Category] {
        &Self::ALL
    }

    /// The kebab-case id `sciart list` prints and `--category` takes, e.g. `deep-cosmos`.
    pub fn id(self) -> &'static str {
        self.core().id()
    }

    /// The display name, e.g. `Deep Cosmos`.
    pub fn label(self) -> &'static str {
        self.core().label()
    }

    pub(crate) fn of(category: sciart_core::Category) -> Self {
        use sciart_core::Category as C;
        match category {
            C::DeepCosmos => Category::DeepCosmos,
            C::SunPlanets => Category::SunPlanets,
            C::EarthSky => Category::EarthSky,
            C::Voyages => Category::Voyages,
            C::Physics => Category::ForcesWaves,
            C::Chemistry => Category::Reactions,
            C::Biology => Category::LivingSystems,
            C::PhysicalChemistry => Category::StatesOfMatter,
            C::Biophysics => Category::PhysicsOfLife,
            C::Biochemistry => Category::MoleculesOfLife,
            C::LivingMatter => Category::LifeFromMatter,
        }
    }

    fn core(self) -> sciart_core::Category {
        use sciart_core::Category as C;
        match self {
            Category::DeepCosmos => C::DeepCosmos,
            Category::SunPlanets => C::SunPlanets,
            Category::EarthSky => C::EarthSky,
            Category::Voyages => C::Voyages,
            Category::ForcesWaves => C::Physics,
            Category::Reactions => C::Chemistry,
            Category::LivingSystems => C::Biology,
            Category::StatesOfMatter => C::PhysicalChemistry,
            Category::PhysicsOfLife => C::Biophysics,
            Category::MoleculesOfLife => C::Biochemistry,
            Category::LifeFromMatter => C::LivingMatter,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lib_r1_categories_mirror_the_core_in_display_order() {
        let ids: Vec<_> = Category::all().iter().map(|c| c.id()).collect();
        let core: Vec<_> = sciart_core::Category::ALL.iter().map(|c| c.id()).collect();
        assert_eq!(ids, core);
        for &c in sciart_core::Category::ALL.iter() {
            assert_eq!(Category::of(c).core(), c);
        }
    }

    #[test]
    fn lib_r1_scenes_list_the_registry_by_name() {
        let names: Vec<_> = scenes().map(|s| s.name()).collect();
        let registry: Vec<_> = sciart_generators::registry()
            .iter()
            .map(|e| e.meta.name)
            .collect();
        assert_eq!(names, registry);
        assert_eq!(scenes().len(), registry.len());
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(names, sorted);
    }

    #[test]
    fn lib_r1_find_resolves_names_and_retired_aliases() {
        assert_eq!(find("aurora").map(|s| s.name()), Some("aurora"));
        for &(old, new) in sciart_generators::ALIASES {
            assert_eq!(find(old).map(|s| s.name()), Some(new));
        }
        assert!(find("no_such_scene").is_none());
        let aurora = find("aurora").unwrap();
        assert!(!aurora.title().is_empty() && !aurora.description().is_empty());
    }
}
