//! The Earth globe shader shared by `satellite_orbit` and `moon_landing` (SPEC-002
//! SAT-A8, SAT-A9): a Natural Earth land map, per-pixel shading with a soft terminator,
//! ocean glint, city lights and a limb glow, drawn into a half-block [`Pixels`] buffer.
//! It lives here, not in either scene, because scenes never import each other (GEN-R3).

use std::f64::consts::{PI, TAU};

use sciart_core::ease;
use sciart_core::geom::v3;
use sciart_core::ink::{self, ramp};
use sciart_core::prelude::*;
use sciart_core::rng::{cell_key, hash01};

/// ECI (any unit) → view axes: x right, y up, z toward the viewer.
pub(crate) fn to_view(p: [f64; 3], elev: f64) -> [f64; 3] {
    let (sb, cb) = elev.sin_cos_det();
    // Edge-on the equator runs across the screen, north is up and ECI +y points away.
    let (vy, vz) = (p[2], -p[1]);
    [p[0], vy * cb - vz * sb, vy * sb + vz * cb]
}

/// View axes → ECI: the inverse of [`to_view`].
fn from_view(v: [f64; 3], elev: f64) -> [f64; 3] {
    let (sb, cb) = elev.sin_cos_det();
    let (vy, vz) = (v[1] * cb + v[2] * sb, -v[1] * sb + v[2] * cb);
    [v[0], -vz, vy]
}

/// Natural Earth 1:110m land (public domain) on a 128 × 64 equirectangular grid, north row
/// first, from 180° W: `.` ocean, `#` land, `d` desert (coarse boxes over the great hot
/// and cold deserts), `*` ice (Antarctica, Greenland, the Arctic and Antarctic pack), `o`
/// a large city's lights.
pub(crate) const MAP: [&str; 64] = [
    "********************************************************************************************************************************",
    "********************************************************************************************************************************",
    "********************************************************************************************************************************",
    "********************************************************************************************************************************",
    ".....................##..........###....*****************............#..............................##..........................",
    "................................##..........*************...........................#..........########.........................",
    "...................##.#####.###.#####*......************..........................##....######################....###...........",
    ".....#################..#..#..###.#..***......*********...............#######.........#.##.#################################.###",
    "####.############################.....****...******......**.........########.#.#################################################",
    "...#..##########################......***.....***..................####.########################################################",
    ".....######.##################......#**.........*.................####o.###############################################...###...",
    "........##......###############......#####....................#.....##...#########################################......##......",
    "..................#################.#######..................###...#...######o##################################.......###......",
    "...................################.########..................#o#o##o##############################################.............",
    "...................#####################...##..................#o##o######o########################################.............",
    "....................o################o#####....................####o##o######.###..##########################o###.#.............",
    "....................#dddd##########o###......................#.####.#.###o....###.#########dddddddddddddd###o###..##............",
    "....................#dddd#o#####o####o.......................#o#....o#.###o#######.########ddddddddddddddo####..................",
    "....................o###############o........................###....#...#########..########ddddddddddddddo..#o...#..............",
    ".....................oddddd##########.........................######........o#####oddddddd############o###o..#.#oo..............",
    "......................ddodd##o###o#..........................o#######..#....###o##ddddddddo################o..#.................",
    "........................ddd##oo#..#.........................dddddddddddddddoddddd.dddddddd#o#########o##o##.....................",
    ".........................dd##......o#......................ddddddddddddddddd.ddddd.ddddddd##o#o############.....................",
    "..........................###.............................ddddddddddddddddddd.ddoddd...o#######oo#######o#.o....................",
    "...........................##...#...#.....................ddddddddddddddddddd.ddddddd....######..####o..o.......................",
    "...........................#o#.##...#.o.#.................dddddddddddddddddddd.ddddd.....o#o##....####..........................",
    "...............................###........................dddddddddddddddddodd.ddd........##.......####....o....................",
    ".................................#................o.......######################...........oo......o###.........................",
    "..................................#..#.#o#.................##################o####.........#.........o..........................",
    "....................................#######.................#####o################..........#......#........#...................",
    "....................................#o########.....................##############.................#.o....#......................",
    "....................................##########.....................#############...................#o..###..##..................",
    "...................................#############...................##########o#.....................#..##.##...#................",
    "...................................################.................#o########......................##.....#....####............",
    "....................................###############o.................#########.......................o##.........###............",
    "....................................###############..................#########............................#.........#...........",
    "....................................o#############...................#########...#............................###.#.............",
    ".....................................#########o###..................##########...#..........................####..##............",
    ".......................................d##########..................ddddd##o#...##.........................#########............",
    ".......................................d#########....................dddd###....#........................ddddddddddd#...........",
    ".......................................d#######oo....................dddd####...#.......................#ddddddddddd##..........",
    ".......................................d#######......................ddddo##............................#ddddddddddd##..........",
    ".......................................#######........................#####..............................ddddddddddd###.........",
    "......................................o######.........................####...............................o###...######..........",
    "......................................#####o.....................................................................####o..........",
    "......................................######......................................................................#o#.........#.",
    "......................................####.........................................................................#..........#.",
    "......................................###...................................................................................#...",
    ".....................................###........................................................................................",
    ".....................................###................................................#.......................................",
    ".....................................###..#.....................................................................................",
    ".......................................##.......................................................................................",
    "................................................................................................................................",
    "................................................................................................................................",
    "................................................................................................................................",
    ".........................................*.........................................*................*...*.......................",
    "........................................*...................................*..**********...***************************.........",
    "********************************************************************************************************************************",
    "********************************************************************************************************************************",
    "********************************************************************************************************************************",
    "********************************************************************************************************************************",
    "********************************************************************************************************************************",
    "********************************************************************************************************************************",
    "********************************************************************************************************************************",
];
pub(crate) const MAP_W: usize = 128;
pub(crate) const MAP_H: usize = 64;

/// What covers the ground at a map cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ground {
    Ocean,
    Land,
    Desert,
    Ice,
    City,
}

/// The ground at longitude `lon` and latitude `lat` (rad); longitude wraps (SAT-A8).
pub(crate) fn ground(lon: f64, lat: f64) -> Ground {
    let u = (lon / TAU + 0.5).rem_euclid(1.0);
    let v = (0.5 - lat / PI).clamp(0.0, 1.0);
    let col = ((u * MAP_W as f64) as usize).min(MAP_W - 1);
    let row = ((v * MAP_H as f64) as usize).min(MAP_H - 1);
    match MAP[row].as_bytes().get(col) {
        Some(b'#') => Ground::Land,
        Some(b'd') => Ground::Desert,
        Some(b'*') => Ground::Ice,
        Some(b'o') => Ground::City,
        _ => Ground::Ocean,
    }
}

/// How much of the ground around `lon`/`lat` (rad) is desert, in [0, 1]: the map's
/// desert cells blurred over about two cells, so the coarse boxes fade into the land
/// around them instead of ending on a hard edge (SAT-A8).
pub(crate) fn desert_share(lon: f64, lat: f64) -> f64 {
    let u = (lon / TAU + 0.5) * MAP_W as f64 - 0.5;
    let v = (0.5 - lat / PI) * MAP_H as f64 - 0.5;
    let desert = |col: i64, row: i64| {
        let row = row.clamp(0, MAP_H as i64 - 1) as usize;
        let col = col.rem_euclid(MAP_W as i64) as usize;
        f64::from(u8::from(MAP[row].as_bytes()[col] == b'd'))
    };
    let bilinear = |u: f64, v: f64| {
        let (c, r) = (u.floor(), v.floor());
        let (fu, fv) = (u - c, v - r);
        let (c, r) = (c as i64, r as i64);
        let top = desert(c, r) * (1.0 - fu) + desert(c + 1, r) * fu;
        let bottom = desert(c, r + 1) * (1.0 - fu) + desert(c + 1, r + 1) * fu;
        top * (1.0 - fv) + bottom * fv
    };
    0.25 * (bilinear(u - 0.5, v - 0.5)
        + bilinear(u + 0.5, v - 0.5)
        + bilinear(u - 0.5, v + 0.5)
        + bilinear(u + 0.5, v + 0.5))
}

/// Half-width (in `cos` of the Sun angle) of the twilight band across the terminator.
const TWILIGHT: f64 = 0.12;

/// Colour of the globe at unit view normal `n`, whose surface point is at `lon`/`lat`,
/// under the unit view Sun `sun` (SAT-A9). False colour in the scene's palette: oceans in
/// Ice, land in Aurora, deserts in Blackbody sand, city lights in mid Ember, ice white.
///
/// A pixel holds one ramp, so land and desert mix by dithering: the pixel is desert when
/// `detail ·` [`desert_share`] beats its stable threshold `dither` ∈ [0, 1). `detail`
/// ∈ [0, 1] says how much the globe's size can show: city lights need more than half.
pub(crate) fn surface_ink(
    n: [f64; 3],
    (lon, lat): (f64, f64),
    sun: [f64; 3],
    detail: f64,
    dither: f64,
) -> Ink {
    let mu = v3::dot(n, sun);
    // 0 on the night side, 1 in full day, smooth across the terminator.
    let day = ease::smoothstep((mu + TWILIGHT) / (2.0 * TWILIGHT));
    let light = day * (0.35 + 0.65 * mu.max(0.0).sqrt());
    let g = ground(lon, lat);
    if day < 0.02 {
        return match g {
            // A mid Ember, dimmer still on a smaller globe, where one pixel is a larger area.
            Ground::City if detail > 0.5 => ramp(Ramp::Ember, 0.3 + 0.3 * detail),
            Ground::Ocean => ramp(Ramp::Ice, 0.04),
            _ => ramp(Ramp::Aurora, 0.05),
        };
    }
    match g {
        Ground::Ocean => {
            // Sun glint: the mirror direction of the Sun seen along the view axis.
            let glint = (2.0 * mu * n[2] - sun[2]).max(0.0).powi(24);
            ramp(Ramp::Ice, 0.06 + 0.34 * light + 0.45 * glint * day)
        }
        Ground::Ice => ramp(Ramp::Ice, 0.12 + 0.86 * light),
        // Sand: the tan stretch of the Blackbody ramp, paler than the land around it, in
        // full day only (dimmed, the ramp would redden it toward the terminator).
        _ if detail * ease::smoothstep(2.0 * light - 0.6) * desert_share(lon, lat) > dither => {
            Ink::new(Role::Blackbody, ink::level(255.0 * (0.14 + 0.26 * light)))
        }
        _ => ramp(Ramp::Aurora, 0.06 + 0.40 * light),
    }
}

/// Pixels of atmosphere drawn outside the limb.
const HALO_PX: f64 = 2.2;

/// A globe drawn into a [`Pixels`] buffer: centre and radius in pixels, the viewer's
/// elevation above its equator, how far it has turned, and the view-space Sun.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Globe {
    pub cx: f64,
    pub cy: f64,
    pub r: f64,
    pub elev: f64,
    /// Rotation angle (rad): longitude `λ` faces inertial longitude `λ + spin`.
    pub spin: f64,
    /// The Sun in the globe's own north-up view frame.
    pub sun: [f64; 3],
    /// Drawn half a turn round, south up (`sun` turns with it).
    pub south_up: bool,
}

impl Globe {
    /// Shade every pixel of the disc and its limb glow; pixels outside are untouched.
    pub(crate) fn paint(&self, px: &mut Pixels) {
        if !(self.r.is_finite() && self.r > 0.0) {
            return;
        }
        let reach = self.r + HALO_PX + 1.0;
        let size = px.pixel_size();
        let x0 = (self.cx - reach).floor().max(0.0) as i32;
        let x1 = (self.cx + reach).ceil().min(f64::from(size.width())) as i32;
        let y0 = (self.cy - reach).floor().max(0.0) as i32;
        let y1 = (self.cy + reach).ceil().min(f64::from(size.height())) as i32;
        for y in y0..y1 {
            let ny = (f64::from(y) + 0.5 - self.cy) / self.r;
            for x in x0..x1 {
                let nx = (f64::from(x) + 0.5 - self.cx) / self.r;
                // Screen-anchored, so the dither does not sparkle as the Earth turns.
                if let Some(ink) = self.shade(nx, ny, hash01(cell_key(x, y))) {
                    px.set(x, y, ink);
                }
            }
        }
    }

    /// The globe's own frame for screen offset `(nx, ny)`: turned half a turn when it is
    /// drawn south up.
    fn frame(&self, nx: f64, ny: f64) -> (f64, f64) {
        if self.south_up { (-nx, -ny) } else { (nx, ny) }
    }

    /// Longitude and latitude (rad) under normalised disc offset `(nx, ny)` (ny down), or
    /// `None` off the disc. Only tests need to ask.
    #[cfg(test)]
    pub(crate) fn lon_lat(&self, nx: f64, ny: f64) -> Option<(f64, f64)> {
        let (nx, ny) = self.frame(nx, ny);
        let d2 = nx * nx + ny * ny;
        (d2 < 1.0).then(|| self.lon_lat_of([nx, -ny, (1.0 - d2).sqrt()]))
    }

    fn lon_lat_of(&self, n: [f64; 3]) -> (f64, f64) {
        let body = from_view(n, self.elev);
        let lat = body[2].clamp(-1.0, 1.0).asin();
        (body[1].atan2(body[0]) - self.spin, lat)
    }

    /// Ink at normalised disc offset `(nx, ny)` (ny down), or `None` off the globe;
    /// `dither` ∈ [0, 1) is the pixel's threshold for mixing land and desert.
    pub(crate) fn shade(&self, nx: f64, ny: f64, dither: f64) -> Option<Ink> {
        let (nx, ny) = self.frame(nx, ny);
        let d2 = nx * nx + ny * ny;
        if d2 >= 1.0 {
            // The limb glow, the air seen edge-on: brightest at the lit limb's edge and
            // fading out over HALO_PX, one band with no gap to the ground.
            let out = (d2.sqrt() - 1.0) * self.r / HALO_PX;
            let d = d2.sqrt();
            let facing = (nx * self.sun[0] - ny * self.sun[1]) / d;
            let glow = (1.0 - out) * (0.25 + 0.75 * facing).max(0.0);
            return (glow > 0.08).then(|| ramp(Ramp::Ice, 0.14 + 0.62 * glow));
        }
        let n = [nx, -ny, (1.0 - d2).sqrt()];
        // A small globe has too few pixels for a desert's dithered edge or a city: one
        // such pixel would dominate the disc, so both fade out below ~10 px radius.
        let detail = ease::smoothstep((self.r - 6.0) / 10.0);
        Some(surface_ink(n, self.lon_lat_of(n), self.sun, detail, dither))
    }
}
