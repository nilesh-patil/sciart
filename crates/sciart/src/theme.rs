//! Colour themes (LIB-R1): the palettes `t` cycles through in the binary.

/// How a scene's semantic inks become colours.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Theme {
    /// Saturated semantic colours on black; the default.
    #[default]
    Astro,
    /// Hue by position.
    Rainbow,
    /// Greys only.
    Mono,
    /// Auroral emission-line colours on a polar night.
    Aurora,
    /// The Okabe–Ito colour-blind-safe palette.
    Colorblind,
}

impl Theme {
    const ALL: [Theme; 5] = [
        Theme::Astro,
        Theme::Rainbow,
        Theme::Mono,
        Theme::Aurora,
        Theme::Colorblind,
    ];

    /// Every theme, in the order the binary's `t` key cycles them.
    pub fn all() -> &'static [Theme] {
        &Self::ALL
    }

    /// The name `--theme` takes, e.g. `astro`.
    pub fn name(self) -> &'static str {
        self.render().name()
    }

    /// The theme with this name, ignoring ASCII case.
    pub fn from_name(name: &str) -> Option<Theme> {
        Self::ALL
            .into_iter()
            .find(|t| t.name().eq_ignore_ascii_case(name))
    }

    pub(crate) fn render(self) -> sciart_render::Theme {
        match self {
            Theme::Astro => sciart_render::Theme::Astro,
            Theme::Rainbow => sciart_render::Theme::Rainbow,
            Theme::Mono => sciart_render::Theme::Mono,
            Theme::Aurora => sciart_render::Theme::Aurora,
            Theme::Colorblind => sciart_render::Theme::Colorblind,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lib_r1_themes_mirror_the_renderer_in_cycle_order() {
        let names: Vec<_> = Theme::all().iter().map(|t| t.name()).collect();
        let render: Vec<_> = sciart_render::Theme::ALL.iter().map(|t| t.name()).collect();
        assert_eq!(names, render);
        assert_eq!(Theme::default(), Theme::Astro);
        assert_eq!(Theme::from_name("AURORA"), Some(Theme::Aurora));
        assert_eq!(Theme::from_name("sepia"), None);
    }
}
