//! One-line status overlay (SPEC-003 RND-R4, RND-R9, spec F4).
//!
//! The full line is
//! `<name> | <fps> fps | <theme> | seed <n> | <keep-awake> | <flags…> | <key hints>`,
//! where `<name>` is the scene's science title (RND-R9), or its id without one.
//! When it is wider than the terminal it is *fitted*, not clipped: lower-priority fields are
//! dropped or shortened one step at a time (see [`LADDER`]), then the name is shortened, so
//! the state flags (`LOW-POWER`, `PAUSED`, `<k> quarantined`) are the last thing to go.

use std::fmt::{self, Write};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::Widget;
use sciart_core::title::{Form, Title, ascii_of};

use crate::Theme;

const SEP: &str = " | ";
const KEY_HINTS: &str = "n p space r h t ? q";
/// Elision marker. The HUD is ASCII-only (RND-R4) and does not know the glyph mode, so `…`
/// is not an option; `~` costs a single column where `...` would cost three.
const ELIDE: char = '~';

#[derive(Debug, Clone)]
pub struct Hud<'a> {
    /// The scene id: the name when there is no title, and its last-resort short form.
    pub generator: &'a str,
    /// The live science title (RT-R12); `None` when titles are off.
    pub title: Option<Title>,
    pub fps: f64,
    pub theme: Theme,
    pub seed: u64,
    /// Keep-awake status string (KA-R5).
    pub keep_awake: &'a str,
    pub low_power: bool,
    pub paused: bool,
    pub quarantined: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeepAwake {
    /// `caffeinate:on`
    Full,
    /// `ka:on` — the backend name is detail; the state is what matters.
    Short,
    Hidden,
}

/// Which optional fields to show. The name and the flags are not in here: the flags are always
/// shown, and the name is fitted separately once the ladder is exhausted.
#[derive(Debug, Clone, Copy)]
struct Plan {
    hints: bool,
    seed: bool,
    theme: bool,
    keep_awake: KeepAwake,
    fps: bool,
}

/// Degradation order, most complete first (RND-R4). Each rung removes the least useful field
/// that is still shown: key hints are also in the docs; a 20-digit seed is long and only matters
/// for reproducing a run; the theme is visible on screen anyway; the keep-awake backend name is
/// detail; fps is diagnostics. The keep-awake state goes last because keeping the machine awake
/// is the app's second job.
const LADDER: [Plan; 7] = {
    let full = Plan {
        hints: true,
        seed: true,
        theme: true,
        keep_awake: KeepAwake::Full,
        fps: true,
    };
    let no_hints = Plan {
        hints: false,
        ..full
    };
    let no_seed = Plan {
        seed: false,
        ..no_hints
    };
    let no_theme = Plan {
        theme: false,
        ..no_seed
    };
    let ka_short = Plan {
        keep_awake: KeepAwake::Short,
        ..no_theme
    };
    let no_fps = Plan {
        fps: false,
        ..ka_short
    };
    let no_ka = Plan {
        keep_awake: KeepAwake::Hidden,
        ..no_fps
    };
    [full, no_hints, no_seed, no_theme, ka_short, no_fps, no_ka]
};

/// What the name slot shows.
#[derive(Debug, Clone, Copy)]
enum Name {
    /// A form of the title, written in ASCII (RND-R9).
    Title(Title, Form),
    Id,
    /// The id cut to at most this many columns (≥ 2), ending in [`ELIDE`] when cut.
    Elided(usize),
    Hidden,
}

impl Hud<'_> {
    /// The complete, unfitted line (what [`Hud::fit`] returns when the width allows it).
    pub fn line(&self) -> String {
        self.fit(usize::MAX)
    }

    /// The HUD text fitted to `width` columns (RND-R4, RND-R9). Never longer than `width`;
    /// deterministic; every flag is present whenever `width` is at least the width of the flags
    /// alone.
    ///
    /// Widths are counted in `char`s: every input is ASCII (registry names, KA-R5 statuses,
    /// titles through `ascii_of`), and ratatui clips anything wider regardless. Measuring runs
    /// through a counting sink, so the only allocation is the returned `String`.
    pub fn fit(&self, width: usize) -> String {
        let mut out = String::new();
        self.fit_into(width, &mut out);
        out
    }

    /// [`Hud::fit`] into `out` (replacing its contents), so the app can reuse one buffer and
    /// the HUD allocates nothing per frame (RT-R14).
    pub fn fit_into(&self, width: usize, out: &mut String) {
        out.clear();
        let name = self.title.map_or(Name::Id, |t| Name::Title(t, Form::Full));
        for plan in LADDER {
            if self.measure(plan, name) <= width {
                return self.write_into(out, plan, name);
            }
        }

        // Only the name and the flags are left; shorten the name before touching any flag:
        // first to shorter whole names (RND-R9), then by eliding the id.
        let last = LADDER[LADDER.len() - 1];
        if let Some(title) = self.title {
            for name in [Name::Title(title, Form::Phenomenon), Name::Id] {
                if self.measure(last, name) <= width {
                    return self.write_into(out, last, name);
                }
            }
        }
        let flags_len = self.measure(last, Name::Hidden);
        let name = if flags_len == 0 {
            Name::Elided(width)
        } else {
            match width.checked_sub(flags_len + SEP.len()) {
                Some(budget) if budget >= 2 => Name::Elided(budget),
                _ => Name::Hidden,
            }
        };
        self.write_into(out, last, name);
        // Even the flags alone may not fit: last resort, cut the text itself.
        elide_in_place(out, width);
    }

    fn measure(&self, plan: Plan, name: Name) -> usize {
        let mut counter = Counter(0);
        // Writing to `Counter` cannot fail; an `Err` would only come from a `Display` impl.
        let _ = self.write(&mut counter, plan, name);
        counter.0
    }

    fn write_into(&self, out: &mut String, plan: Plan, name: Name) {
        // Writing to a `String` cannot fail.
        let _ = self.write(out, plan, name);
    }

    /// The single source of truth for the layout: `measure` and `write_into` both run it,
    /// so the measured width is by construction the rendered width.
    fn write(&self, out: &mut impl Write, plan: Plan, name: Name) -> fmt::Result {
        let mut f = Fields { out, first: true };
        match name {
            // `?` only for chars outside the title charset, which GEN-R12 keeps out.
            Name::Title(title, form) => {
                f.chars(title.chars(form).map(|c| ascii_of(c).unwrap_or('?')))?;
            }
            Name::Id => f.field(format_args!("{}", self.generator))?,
            Name::Elided(budget) => f.elided(self.generator, budget)?,
            Name::Hidden => {}
        }
        if plan.fps {
            f.field(format_args!("{:.0} fps", self.fps))?;
        }
        if plan.theme {
            f.field(format_args!("{}", self.theme.name()))?;
        }
        if plan.seed {
            f.field(format_args!("seed {}", self.seed))?;
        }
        match plan.keep_awake {
            KeepAwake::Full => f.field(format_args!("{}", self.keep_awake))?,
            KeepAwake::Short => match self.keep_awake.split_once(':') {
                // `caffeinate:on` -> `ka:on`; statuses without a backend (`off`) stay as is.
                Some((_, state)) => f.field(format_args!("ka:{state}"))?,
                None => f.field(format_args!("{}", self.keep_awake))?,
            },
            KeepAwake::Hidden => {}
        }
        if self.low_power {
            f.field(format_args!("LOW-POWER"))?;
        }
        if self.paused {
            f.field(format_args!("PAUSED"))?;
        }
        if self.quarantined > 0 {
            f.field(format_args!("{} quarantined", self.quarantined))?;
        }
        if plan.hints {
            f.field(format_args!("{KEY_HINTS}"))?;
        }
        Ok(())
    }
}

/// Writes fields joined by [`SEP`].
struct Fields<'w, W: Write> {
    out: &'w mut W,
    first: bool,
}

impl<W: Write> Fields<'_, W> {
    fn sep(&mut self) -> fmt::Result {
        if !std::mem::take(&mut self.first) {
            self.out.write_str(SEP)?;
        }
        Ok(())
    }

    fn field(&mut self, args: fmt::Arguments<'_>) -> fmt::Result {
        self.sep()?;
        self.out.write_fmt(args)
    }

    fn chars(&mut self, mut chars: impl Iterator<Item = char>) -> fmt::Result {
        self.sep()?;
        chars.try_for_each(|c| self.out.write_char(c))
    }

    /// `text` cut to `budget` columns, the last one being [`ELIDE`] when anything was cut.
    fn elided(&mut self, text: &str, budget: usize) -> fmt::Result {
        if budget == 0 {
            return Ok(());
        }
        self.sep()?;
        if text.chars().count() <= budget {
            return self.out.write_str(text);
        }
        if budget == 1 {
            // No room for a marker; one real character says more than a lone `~`.
            return self.out.write_str(prefix(text, 1));
        }
        self.out.write_str(prefix(text, budget - 1))?;
        self.out.write_char(ELIDE)
    }
}

/// The first `n` chars of `s`.
fn prefix(s: &str, n: usize) -> &str {
    s.char_indices().nth(n).map_or(s, |(i, _)| &s[..i])
}

/// Cut `s` to at most `width` chars, marking the cut with [`ELIDE`] when there is room.
fn elide_in_place(s: &mut String, width: usize) {
    if s.chars().count() <= width {
        return;
    }
    if width <= 1 {
        let keep = prefix(s, width).len();
        s.truncate(keep);
        return;
    }
    let keep = prefix(s, width - 1).len();
    s.truncate(keep);
    s.push(ELIDE);
}

/// A `fmt::Write` sink that only counts chars, so fitting can try layouts without allocating.
struct Counter(usize);

impl Write for Counter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.0 += s.chars().count();
        Ok(())
    }
}

impl Hud<'_> {
    /// Draw the HUD on the last row of `area` (RND-R4, RND-R9), fitting the text into
    /// `scratch`: with a reused `scratch` it allocates nothing (RT-R14).
    pub fn render_into(&self, area: Rect, buf: &mut Buffer, scratch: &mut String) {
        if area.height == 0 || area.width == 0 {
            return;
        }
        let (x, y) = (area.x, area.bottom() - 1);
        // One reversed lead cell, so the name starts in column 1 like the canvas title's
        // text (CORE-R74) and does not shift when the HUD is toggled (RND-R9).
        buf[(x, y)]
            .set_symbol(" ")
            .set_style(Style::new().reversed());
        let width = area.width - 1;
        self.fit_into(usize::from(width), scratch);
        if scratch.is_empty() {
            return;
        }
        // What `Line::from(text).reversed()` draws, without building a `Line`: the whole
        // row reversed, the text from its left edge.
        let row = Rect::new(x + 1, y, width, 1).intersection(buf.area);
        buf.set_style(row, Style::new().reversed());
        buf.set_stringn(
            row.x,
            row.y,
            scratch.as_str(),
            usize::from(row.width),
            Style::new(),
        );
    }
}

impl Widget for &Hud<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        self.render_into(area, buf, &mut String::new());
    }
}

#[cfg(test)]
mod tests {
    use ratatui::style::Modifier;

    use super::*;

    const BIG_SEED: u64 = 16_376_649_230_169_585_180; // 20 digits, like a random default seed

    /// Untitled (`--no-title`): the name slot is the id, as before M11.
    fn hud() -> Hud<'static> {
        Hud {
            generator: "starfield",
            title: None,
            fps: 29.6,
            theme: Theme::Astro,
            seed: 42,
            keep_awake: "caffeinate:on",
            low_power: false,
            paused: false,
            quarantined: 0,
        }
    }

    /// The case that broke: a long scene name, paused, random seed, 80 columns.
    fn cmb() -> Hud<'static> {
        Hud {
            generator: "cosmic_microwave_background",
            seed: BIG_SEED,
            paused: true,
            ..hud()
        }
    }

    /// `cmb()` with its science title.
    fn titled_cmb() -> Hud<'static> {
        Hud {
            title: Some(Title::new("Oldest light · early sound waves", None)),
            ..cmb()
        }
    }

    #[test]
    fn rnd_r9_name_slot_is_the_ascii_title() {
        let h = titled_cmb();
        assert_eq!(
            h.line(),
            "Oldest light - early sound waves | 30 fps | astro | seed 16376649230169585180 \
             | caffeinate:on | PAUSED | n p space r h t ? q"
        );
        let h = Hud {
            title: Some(Title::new("Strange attractor · chaos", Some("Rössler"))),
            ..hud()
        };
        assert!(h.line().starts_with("Strange attractor - Rossler | "));
        assert!(h.line().is_ascii());
    }

    #[test]
    fn rnd_r9_fit_steps_title_then_phenomenon_then_id() {
        // The RND-R4 ladder runs with the full title; then the name slot shortens.
        let h = titled_cmb();
        let expected: &[(usize, &str)] = &[
            (
                80,
                "Oldest light - early sound waves | 30 fps | astro | caffeinate:on | PAUSED",
            ),
            (41, "Oldest light - early sound waves | PAUSED"),
            // Phenomenon only; the id (36 columns with the flag) is longer, so it is skipped.
            (40, "Oldest light | PAUSED"),
            (21, "Oldest light | PAUSED"),
            // Then the id, elided.
            (20, "cosmic_mic~ | PAUSED"),
            (11, "c~ | PAUSED"),
            (10, "PAUSED"),
        ];
        for &(width, want) in expected {
            assert_eq!(h.fit(width), want, "width {width}");
        }

        // An id shorter than the phenomenon is used whole before it is elided.
        let h = Hud {
            generator: "pulsar",
            title: Some(Title::new("Neutron star beacon · spin", None)),
            ..cmb()
        };
        let expected: &[(usize, &str)] = &[
            (35, "Neutron star beacon - spin | PAUSED"),
            (34, "Neutron star beacon | PAUSED"),
            (28, "Neutron star beacon | PAUSED"),
            (27, "pulsar | PAUSED"),
            (15, "pulsar | PAUSED"),
            (14, "puls~ | PAUSED"),
        ];
        for &(width, want) in expected {
            assert_eq!(h.fit(width), want, "width {width}");
        }
    }

    #[test]
    fn rnd_r9_without_a_title_the_slot_is_the_id() {
        assert!(
            cmb()
                .line()
                .starts_with("cosmic_microwave_background | 30 fps")
        );
        assert_eq!(cmb().fit(35), "cosmic_microwave_backgrou~ | PAUSED");
    }

    #[test]
    fn rnd_r4_line_format() {
        assert_eq!(
            hud().line(),
            "starfield | 30 fps | astro | seed 42 | caffeinate:on | n p space r h t ? q"
        );
        let h = Hud {
            low_power: true,
            paused: true,
            quarantined: 2,
            ..hud()
        };
        assert_eq!(
            h.line(),
            "starfield | 30 fps | astro | seed 42 | caffeinate:on | LOW-POWER | PAUSED \
             | 2 quarantined | n p space r h t ? q"
        );
        assert!(h.line().is_ascii());
    }

    #[test]
    fn rnd_r4_fit_degrades_in_priority_order() {
        let h = cmb();
        let full = "cosmic_microwave_background | 30 fps | astro | seed 16376649230169585180 \
                    | caffeinate:on | PAUSED | n p space r h t ? q";
        let no_hints = "cosmic_microwave_background | 30 fps | astro | seed 16376649230169585180 \
                        | caffeinate:on | PAUSED";
        let no_seed = "cosmic_microwave_background | 30 fps | astro | caffeinate:on | PAUSED";
        let no_theme = "cosmic_microwave_background | 30 fps | caffeinate:on | PAUSED";
        let ka_short = "cosmic_microwave_background | 30 fps | ka:on | PAUSED";
        let no_fps = "cosmic_microwave_background | ka:on | PAUSED";
        let no_ka = "cosmic_microwave_background | PAUSED";
        assert_eq!(full.len(), 119);
        let expected: &[(usize, &str)] = &[
            (200, full),
            (119, full),
            // 1. key hints go first
            (118, no_hints),
            (97, no_hints),
            // 2. then the seed (at 80 columns this is where PAUSED used to fall off)
            (96, no_seed),
            (80, no_seed),
            (69, no_seed),
            // 3. theme
            (68, no_theme),
            (61, no_theme),
            // 4. keep-awake detail: backend name -> `ka`
            (60, ka_short),
            (53, ka_short),
            // 5. fps
            (52, no_fps),
            (44, no_fps),
            // 6. keep-awake entirely
            (43, no_ka),
            (36, no_ka),
            // 7. elide the name (ASCII `~`), never the flag
            (35, "cosmic_microwave_backgrou~ | PAUSED"),
            (30, "cosmic_microwave_bac~ | PAUSED"),
            (11, "c~ | PAUSED"),
            // 8. no room for a meaningful name: flags alone
            (10, "PAUSED"),
            (6, "PAUSED"),
            // 9. below the flags' own width: cut them too (last resort)
            (5, "PAUS~"),
            (2, "P~"),
            (1, "P"),
            (0, ""),
        ];
        for &(width, want) in expected {
            assert_eq!(h.fit(width), want, "width {width}");
        }
    }

    #[test]
    fn rnd_r4_fit_all_flags_survive_before_the_name() {
        let h = Hud {
            low_power: true,
            quarantined: 12,
            ..cmb()
        };
        assert_eq!(
            h.fit(80),
            "cosmic_microwave_background | ka:on | LOW-POWER | PAUSED | 12 quarantined"
        );
        assert_eq!(
            h.fit(50),
            "cosmic_micr~ | LOW-POWER | PAUSED | 12 quarantined"
        );
        // The flags alone are 35 columns; a name needs 2 more plus the separator.
        assert_eq!(h.fit(40), "c~ | LOW-POWER | PAUSED | 12 quarantined");
        assert_eq!(h.fit(39), "LOW-POWER | PAUSED | 12 quarantined");
        assert_eq!(h.fit(35), "LOW-POWER | PAUSED | 12 quarantined");
        assert_eq!(h.fit(34), "LOW-POWER | PAUSED | 12 quarantin~");
        assert_eq!(h.fit(20), "LOW-POWER | PAUSED ~");
    }

    #[test]
    fn rnd_r4_fit_without_flags_elides_only_the_name() {
        let h = Hud {
            keep_awake: "off",
            ..hud()
        };
        assert_eq!(h.fit(45), "starfield | 30 fps | astro | seed 42 | off");
        assert_eq!(h.fit(40), "starfield | 30 fps | astro | off");
        // `off` has no backend to shorten, so it is kept as is.
        assert_eq!(h.fit(24), "starfield | 30 fps | off");
        assert_eq!(h.fit(20), "starfield | off");
        assert_eq!(h.fit(14), "starfield");
        assert_eq!(h.fit(9), "starfield");
        assert_eq!(h.fit(5), "star~");
        assert_eq!(h.fit(1), "s");
        assert_eq!(h.fit(0), "");
    }

    #[test]
    fn rnd_r4_line_is_the_unconstrained_fit() {
        let h = cmb();
        assert_eq!(h.line(), h.fit(usize::MAX));
        assert_eq!(h.line(), h.fit(h.line().len()));
        assert_ne!(h.line(), h.fit(h.line().len() - 1));
    }

    /// Exhaustive property over widths 0..=160, every flag combination and a spread of names,
    /// titles (longer and shorter than the id, captioned, accented), seeds, fps values and
    /// keep-awake statuses.
    #[test]
    fn rnd_r4_r9_fit_properties_over_many_widths() {
        let names = ["-", "x", "starfield", "cosmic_microwave_background"];
        let titles = [
            None,
            Some(Title::new("Oldest light · early sound waves", None)),
            Some(Title::new("Neutron star beacon · spin", None)),
            Some(Title::new("Strange attractor · chaos", Some("Rössler"))),
            Some(Title::new("Ab", None)),
        ];
        let statuses = ["off", "caffeinate:on", "mouse:err(3)", "mouse:unavailable"];
        for name in names {
            for title in titles {
                for keep_awake in statuses {
                    // Shortest and longest seed and fps texts: only their widths matter.
                    for seed in [0, u64::MAX] {
                        for fps in [0.0, 144.0] {
                            for bits in 0..8u8 {
                                check_fit(&Hud {
                                    generator: name,
                                    title,
                                    fps,
                                    seed,
                                    keep_awake,
                                    low_power: bits & 1 != 0,
                                    paused: bits & 2 != 0,
                                    quarantined: if bits & 4 != 0 { 3 } else { 0 },
                                    ..hud()
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    fn check_fit(h: &Hud<'_>) {
        let mut flags = Vec::new();
        if h.low_power {
            flags.push("LOW-POWER".to_string());
        }
        if h.paused {
            flags.push("PAUSED".to_string());
        }
        if h.quarantined > 0 {
            flags.push(format!("{} quarantined", h.quarantined));
        }
        let flags_min = flags.join(SEP).len();
        let full = h.line();
        let mut prev_len = 0;
        for width in 0..=160 {
            let got = h.fit(width);
            assert!(got.is_ascii(), "{got:?}");
            assert!(got.len() <= width, "width {width}: {got:?}");
            assert_eq!(got, h.fit(width), "deterministic");
            if width >= full.len() {
                assert_eq!(got, full, "width {width}");
            }
            if width >= flags_min {
                for flag in &flags {
                    assert!(
                        got.contains(flag.as_str()),
                        "width {width}: {flag} in {got:?}"
                    );
                }
            }
            if !flags.is_empty() && width >= flags_min + SEP.len() + 2 {
                let title_initial = h.title.map(|t| &t.phenomenon[..1]);
                assert!(
                    got.starts_with(&h.generator[..1])
                        || title_initial.is_some_and(|i| got.starts_with(i)),
                    "width {width}: {got:?}"
                );
            }
            assert_eq!(got.is_empty(), width == 0, "width {width}");
            assert!(!got.starts_with(' ') && !got.ends_with(' ') || got.ends_with(" ~"));
            // More room never shows less.
            assert!(got.len() >= prev_len, "width {width}: {got:?}");
            prev_len = got.len();
        }
    }

    #[test]
    fn rnd_r4_r9_renders_only_the_last_row_fitted_and_reversed_after_a_lead_cell() {
        let area = Rect::new(0, 0, 12, 3);
        let mut buf = Buffer::empty(area);
        for y in 0..3 {
            buf[(0, y)].set_char('#');
        }
        hud().render(area, &mut buf);

        assert_eq!(buf[(0, 0)].symbol(), "#");
        assert_eq!(buf[(0, 1)].symbol(), "#");
        let bottom: String = (0..12).map(|x| buf[(x, 2)].symbol().to_string()).collect();
        // Fitted, not clipped mid-field (was `starfield | ` before RND-R4 fitting), after a
        // lead cell (RND-R9).
        assert_eq!(bottom, " starfield  ");
        assert!(buf[(0, 2)].modifier.contains(Modifier::REVERSED));
    }

    #[test]
    fn rnd_r4_widget_fits_to_the_area_width() {
        let area = Rect::new(0, 0, 40, 2);
        let mut buf = Buffer::empty(area);
        cmb().render(area, &mut buf);
        let bottom: String = (0..40).map(|x| buf[(x, 1)].symbol().to_string()).collect();
        assert_eq!(bottom, " cosmic_microwave_background | PAUSED   ");
    }

    #[test]
    fn rnd_r4_zero_height_area_is_a_noop() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 5, 1));
        hud().render(Rect::new(0, 0, 5, 0), &mut buf);
        assert_eq!(buf[(0, 0)].symbol(), " ");
    }
}
