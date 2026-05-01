//! Argument and stepping helpers shared by the scene review examples (`frames`, `preview`).

use sciart_core::prelude::*;
use sciart_generators::Entry;

/// `WxH` in cells, both at least 1.
pub fn size(v: &str) -> Result<Size, String> {
    let dim = |s: &str| s.parse::<u16>().ok().filter(|&n| n > 0);
    v.split_once('x')
        .and_then(|(w, h)| dim(w).zip(dim(h)))
        .map(|(w, h)| Size::new(w, h))
        .ok_or(format!("bad --size {v}"))
}

/// Comma-separated simulated seconds, finite and non-negative, sorted.
pub fn times(v: &str) -> Result<Vec<f64>, String> {
    let mut times: Vec<f64> = v
        .split(',')
        .map(|s| {
            s.trim()
                .parse::<f64>()
                .ok()
                .filter(|t| t.is_finite() && *t >= 0.0)
        })
        .collect::<Option<_>>()
        .ok_or(format!("bad --t {v}"))?;
    times.sort_by(f64::total_cmp);
    Ok(times)
}

/// A registered scene by name or alias; the error lists every scene.
pub fn scene(name: &str) -> Result<&'static Entry, String> {
    sciart_generators::find(name).ok_or_else(|| {
        let names: Vec<_> = sciart_generators::registry()
            .iter()
            .map(|e| e.meta.name)
            .collect();
        format!("unknown scene {name:?}; known: {}", names.join(", "))
    })
}

/// Step `stage` at the fixed `SIM_DT`, exactly as the app does, until `*steps` reaches
/// simulated time `t`.
pub fn step_to(stage: &mut Stage, steps: &mut u64, t: f64) {
    let target = (t / SIM_DT).round() as u64;
    while *steps < target {
        stage.step(SIM_DT);
        *steps += 1;
    }
}
