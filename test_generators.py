#!/usr/bin/env python3
"""Test script to verify all generators can be imported and have generate() function."""

import sys
import os

# Add the art-terminal directory to Python path
sys.path.insert(0, os.path.join(os.path.dirname(__file__), "tools-py", "art-terminal"))

from art_generators import (
    black_hole,
    brownian_motion,
    chemical_reaction,
    constellation_map,
    cosmic_microwave_background,
    crystal_growth,
    doppler_effect,
    double_pendulum,
    exoplanet_transit,
    galaxy_collision,
    gravitational_lensing,
    mars_rover,
    molecular_dynamics,
    moon_landing,
    moon_phases,
    particle_accelerator,
    pulsar,
    radioactive_decay,
    rocket_launch,
    satellite_orbit,
    solar_system,
    spacetime_grid,
    starfield,
    stellar_nursery,
    voyager_journey,
    wave_interference,
)

generators = [
    ("black_hole", black_hole),
    ("brownian_motion", brownian_motion),
    ("chemical_reaction", chemical_reaction),
    ("constellation_map", constellation_map),
    ("cosmic_microwave_background", cosmic_microwave_background),
    ("crystal_growth", crystal_growth),
    ("doppler_effect", doppler_effect),
    ("double_pendulum", double_pendulum),
    ("exoplanet_transit", exoplanet_transit),
    ("galaxy_collision", galaxy_collision),
    ("gravitational_lensing", gravitational_lensing),
    ("mars_rover", mars_rover),
    ("molecular_dynamics", molecular_dynamics),
    ("moon_landing", moon_landing),
    ("moon_phases", moon_phases),
    ("particle_accelerator", particle_accelerator),
    ("pulsar", pulsar),
    ("radioactive_decay", radioactive_decay),
    ("rocket_launch", rocket_launch),
    ("satellite_orbit", satellite_orbit),
    ("solar_system", solar_system),
    ("spacetime_grid", spacetime_grid),
    ("starfield", starfield),
    ("stellar_nursery", stellar_nursery),
    ("voyager_journey", voyager_journey),
    ("wave_interference", wave_interference),
]

print("Testing all generators...")
print("=" * 60)

passed = 0
failed = 0
errors = []

for name, module in generators:
    try:
        if not hasattr(module, "generate"):
            failed += 1
            errors.append(f"❌ {name}: missing generate() function")
            continue

        # Try calling generate with small dimensions
        result = module.generate(40, 20)

        if not isinstance(result, str):
            failed += 1
            errors.append(f"❌ {name}: generate() didn't return a string")
            continue

        passed += 1
        print(f"✅ {name}")

    except Exception as e:
        failed += 1
        errors.append(f"❌ {name}: {type(e).__name__}: {e}")

print("=" * 60)
print(f"\nResults: {passed} passed, {failed} failed out of {len(generators)} total")

if errors:
    print("\nErrors:")
    for error in errors:
        print(f"  {error}")
    sys.exit(1)
else:
    print("\n🎉 All generators working correctly!")
    sys.exit(0)
