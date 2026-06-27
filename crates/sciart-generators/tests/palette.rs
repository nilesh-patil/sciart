//! GEN-R23 (ADR-0018): every scene inks only with the roles that existed at M11. No drawn
//! cell (glyph foreground or half-block background) uses `Role::Ramp(_)`,
//! `Role::Blackbody` or `Role::Phase`; checked on seed-42 Unicode frames at 80×24 and
//! 200×50 after 0, 1, 5 and 20 s of simulated time.

use std::collections::BTreeMap;

use sciart_core::prelude::*;
use sciart_generators::{Entry, registry};

const SIZES: [(u16, u16); 2] = [(80, 24), (200, 50)];

/// Scenes added at M13 whose M11-idiom restyle (GEN-R24) has not landed yet: skipped by
/// the palette check. An allowlist that may only shrink: a listed scene that already
/// passes fails `gen_r23_pending_only_shrinks` until its entry is deleted. Alphabetical.
const PENDING: &[&str] = &[
    "catalytic_surface",
    "chemical_garden",
    "cosmic_web",
    "dicty_aggregation",
    "firefly_sync",
    "heart_fibrillation",
    "hp_protein_folding",
    "jwst_unfolding",
    "kelvin_helmholtz",
    "lenia",
    "lichtenberg_discharge",
    "meteor_shower",
    "molecular_motors",
    "motility_clustering",
    "noctilucent_clouds",
    "orbital_rendezvous",
    "quantum_double_slit",
    "radical_chain",
    "rainbow",
    "relativistic_jet",
    "saturn_rings",
    "seashell_pigmentation",
    "solar_eruption",
    "spinodal_decomposition",
    "spiral_galaxy",
    "star_trails",
    "total_solar_eclipse",
];
const TIMES: [u32; 4] = [0, 1, 5, 20];

fn is_m11_role(role: Role) -> bool {
    !matches!(role, Role::Ramp(_) | Role::Blackbody | Role::Phase)
}

/// Check every entry on all cores (scenes are independent: 76 scenes × 20 s at
/// 200×50 would otherwise dominate a debug `rs-test`), keeping the non-empty results.
fn par_offenders(entries: &[&'static Entry]) -> Vec<(String, BTreeMap<String, String>)> {
    let threads = std::thread::available_parallelism().map_or(4, usize::from);
    let next = std::sync::atomic::AtomicUsize::new(0);
    let mut offenders: Vec<(String, BTreeMap<String, String>)> = std::thread::scope(|s| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                s.spawn(|| {
                    let mut out = Vec::new();
                    loop {
                        let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(entry) = entries.get(i) else { break };
                        let roles = offending_roles(entry);
                        if !roles.is_empty() {
                            out.push((entry.meta.name.to_owned(), roles));
                        }
                    }
                    out
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|w| w.join().expect("palette worker panicked"))
            .collect()
    });
    offenders.sort();
    offenders
}

/// The non-M11 roles a scene uses, with the first frame each shows up in.
fn offending_roles(entry: &Entry) -> BTreeMap<String, String> {
    let mut found = BTreeMap::new();
    for (w, h) in SIZES {
        let mut stage = Stage::new((entry.build)(), 42, Size::new(w, h), GlyphMode::Unicode);
        let mut steps = 0_u32;
        for t in TIMES {
            while steps < t * 60 {
                stage.step(SIM_DT);
                steps += 1;
            }
            for cell in stage.render().cells() {
                for ink in [cell.ink, cell.bg] {
                    if !is_m11_role(ink.role) {
                        found
                            .entry(format!("{:?}", ink.role))
                            .or_insert_with(|| format!("{w}x{h} t={t}s"));
                    }
                }
            }
        }
    }
    found
}

#[test]
fn gen_r23_scenes_use_only_m11_roles() {
    let entries: Vec<&'static Entry> = registry()
        .iter()
        .filter(|e| !PENDING.contains(&e.meta.name))
        .collect();
    let offenders = par_offenders(&entries);
    let report: Vec<String> = offenders
        .iter()
        .map(|(name, roles)| {
            let roles: Vec<String> = roles
                .iter()
                .map(|(role, first)| format!("{role} (first at {first})"))
                .collect();
            format!("  {name}: {}", roles.join(", "))
        })
        .collect();
    assert!(
        offenders.is_empty(),
        "GEN-R23: {} of {} checked scenes ink with roles M11 did not have \
         ({} restyles PENDING):\n{}",
        offenders.len(),
        entries.len(),
        PENDING.len(),
        report.join("\n")
    );
}

#[test]
fn gen_r23_classifier_matches_the_m11_role_list() {
    use sciart_core::ink::Ramp;
    let m11 = [
        Role::Background,
        Role::Dim,
        Role::Star,
        Role::Sun,
        Role::Planet,
        Role::Particle,
        Role::Wave,
        Role::Hot,
        Role::Cold,
        Role::Structure,
        Role::Accent,
        Role::Text,
        Role::Seq,
        Role::Div,
    ];
    assert!(m11.into_iter().all(is_m11_role));
    for role in [Role::Phase, Role::Blackbody, Role::Ramp(Ramp::Ember)] {
        assert!(!is_m11_role(role), "{role:?}");
    }
}

#[test]
fn gen_r23_pending_only_shrinks() {
    let names: Vec<&str> = registry().iter().map(|e| e.meta.name).collect();
    for p in PENDING {
        assert!(names.contains(p), "PENDING names no registered scene: {p}");
    }
    assert!(
        PENDING.windows(2).all(|w| w[0] < w[1]),
        "PENDING must be alphabetical without duplicates"
    );
    let entries: Vec<&'static Entry> = registry()
        .iter()
        .filter(|e| PENDING.contains(&e.meta.name))
        .collect();
    let still: Vec<String> = par_offenders(&entries).into_iter().map(|o| o.0).collect();
    let done: Vec<&str> = PENDING
        .iter()
        .copied()
        .filter(|p| !still.iter().any(|s| s == p))
        .collect();
    assert!(
        done.is_empty(),
        "GEN-R23: these PENDING scenes now use only M11 roles; delete them from PENDING: {done:?}"
    );
}
