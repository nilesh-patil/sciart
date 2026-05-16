# ADR-0015: Phase, blackbody and named ramp roles (amends ADR-0004, ADR-0011)

- **Status**: Accepted
- **Date**: 2026-09-27

## Context
ADR-0011 gave fields one sequential and one diverging colormap per theme. Under a given theme,
every `Seq` field therefore has the same colours: BZ, Gray–Scott, the solar surface and a nebula
all look inferno. Three M12 needs don't fit `Seq` or `Div` at all:

- **Cyclic quantities** such as quantum phase or oscillator phase. A linear ramp puts a hard seam
  at 2π and gives 0 and 2π opposite colours.
- **Temperature as colour.** A star, accretion disc, ember or plume has a physical colour, the
  Planck locus. Hand-picked `Hot` levels lose it, and RGB in a scene would break ADR-0004.
- **Per-scene identity.** Field scenes need different, still perceptual ramps under one theme, so
  that a chemical wave, an emission nebula and an aurora don't share a palette.

## Decision
Three additions to `sciart_core::Role`. All are semantic (a meaning plus a `level`), never RGB,
and each keeps `Ink` at two bytes.

- **`Role::Phase`**: a cyclic field. `level` is the phase, one full turn over 256 levels
  (`ink::phase(radians)`, `ink::phase_value(ink)`), so 255 sits next to 0. Themes map it to a
  closed hue loop with no seam and nearly constant lightness. Magnitude is not in the ink, so a
  scene shows `|ψ|` by coverage (dot density, a threshold) and the hue carries `arg ψ`.
- **`Role::Blackbody`**: `level` is temperature on a log scale from 1000 K (level 0) to 30 000 K
  (level 255) (`ink::blackbody(kelvin)`, `ink::blackbody_kelvin(ink)`). The log scale gives the
  red-to-white part, where the colour changes fastest, about half the levels. Themes map it to
  the Planck locus in sRGB (CIE 1931 2° observer, normalised to the brightest channel). Lightness
  rises from a dark red (L* ≈ 12) to white near 6500 K and then stays blue-white (L* ≥ 75), so
  hotter never reads as dimmer on the red side.
- **`Role::Ramp(Ramp)`**: a sequential field in a named ramp (`ink::ramp(ramp, v)`, `v ∈ [0, 1]`,
  read back with `ink::seq_value`). Ten ramps (six at M11; M12 added `Starlight`, `Sulfur`, `Cobalt` and `Eosin`), one hue family each, all with a near-black low end
  (L* ≈ 4) and lightness increasing evenly to a pale top (L* ≥ 93):

  | Ramp | Hue path | Meant for |
  |------|----------|-----------|
  | `Ember` | oxblood → scarlet → orange → pale gold | glowing matter, fire, plumes, heat of reaction, growth age |
  | `Ice` | navy → teal-blue → cyan → white | cold gas, frost and snow, Cherenkov and young-star blue |
  | `Ferroin` | wine → crimson → violet → azure | BZ ferroin/ferriin (red reduced, blue oxidised), redox fronts |
  | `HAlpha` | crimson → rose → pale pink | emission nebulae (Hα 656 nm), prominences, the Hubble-palette red |
  | `Aurora` | deep teal → emerald → lime-white | auroral O I 557.7 nm, fluorescence (GFP-like), bioluminescence |
  | `Plasma` | indigo → violet → lavender-white | gas discharges, lightning, synchrotron glow, pulsar beams |
 | `Starlight` | near-black → neutral grey → pale warm white | unresolved starlight (a globular cluster's glow, the Milky Way); added in M12 |
 | `Sulfur` | near-black olive → chrome yellow → pale lemon | the *Physarum* plasmodium, pollen and sulfur yellows; added in M12 so a yellow organism need not borrow Ember's orange |
  | `Cobalt` | ultramarine → cobalt → silver-blue | cobalt glass and pigment, a cool mineral precipitate (`liesegang_rings`); a blue that is neither Ice's cyan nor Ferroin's azure top; added in M12 |
  | `Eosin` | hematoxylin purple → magenta → pale pink | an H&E stain, for patterns that read as tissue (`gray_scott`); added in M12 |

  Each ramp is a 256-level table built once from OKLCh: lightness linear in level, hue linear
  between the two ends, chroma lowered only where sRGB cannot hold it.
  Named "real" palettes are false colour; SPEC-002 says so in the scenes that use them.

Theme mapping (RND-R10..R12): `astro` and `rainbow` share the Blackbody locus and the named ramps,
because those are a scene's physical or art-directed colour, not the theme's. `rainbow` gives
Phase a full-saturation hue wheel, and `astro` a softer isoluminant loop. `mono` keeps its RND-R6
floors: Blackbody and every ramp use the `Seq` grey ramp (linear in L* from the near-black
floor), and Phase is a triangle in L* (dark at 0, bright at half a turn), which is continuous
across the wrap.

`Dots` picks a cell's ink by intensity (CORE-R41). A Phase level is an angle, not an intensity,
so every Phase dot ties and the last one set wins.

## Consequences
+ Field scenes get their own identity and physical colour without leaving the semantic model.
  Themes still switch correctly, and snapshots (glyphs only) are unaffected.
+ `Ramp` is a closed enum mapped in one place, so a new ramp is one enum arm and one OKLCh
  spec (two hues and a chroma); the RND-R12 tests cover it through `Ramp::ALL`.
− Mono can't separate ramps (it has one channel), and its Phase is a brightness triangle, so
  opposite phases 0 and π read as dark and bright, not as two hues.
− Blackbody lightness is not monotone above the white point, 6500 K (white is brighter than
  blue-white). RND-R1's monotone property holds for Blackbody only up to the white point.
− Phase carries no magnitude; domain colouring needs coverage for `|ψ|`.

## Alternatives considered
- More `Seq`-like roles per theme (`Seq2`, `Seq3`): anonymous, and they don't say what a
  scene means.
- A `Role::Spectrum` for wavelength: needed for dispersion only (the M13 rainbow scene). It isn't
  a monotone ramp, so it is deferred to that scene.
- Encoding magnitude and phase together in the level, for example 5 + 3 bits: coarse in both,
  and too clever for the call sites.
- Ramps computed per call in OKLab: costs cubes and a matrix per painted cell. The tables are
  built once on first use (768 bytes each) and read by level.
