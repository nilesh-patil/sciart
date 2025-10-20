import random
import math


def generate(width, height):
    """Generates an enhanced black hole with accretion disk and gravitational effects."""
    grid = [[" " for _ in range(width)] for _ in range(height)]

    bh_x, bh_y = width // 2, height // 2
    event_horizon = 4
    ergosphere = 7

    # Initialize particles for accretion disk
    if not hasattr(generate, "particles"):
        generate.particles = []
        for _ in range(800):
            angle = random.uniform(0, 2 * math.pi)
            distance = random.uniform(event_horizon + 2, event_horizon + 20)
            # Keplerian velocity: v ∝ 1/√r
            speed = 1.5 / math.sqrt(distance)
            # Random vertical displacement for 3D effect
            z = random.uniform(-2, 2)
            brightness = random.choice([0, 1, 2, 3])
            generate.particles.append(
                {
                    "angle": angle,
                    "distance": distance,
                    "speed": speed,
                    "z": z,
                    "brightness": brightness,
                }
            )

    # Update particle positions
    for particle in generate.particles:
        particle["angle"] += particle["speed"]
        # Slowly spiral inward
        if random.random() < 0.001:
            particle["distance"] *= 0.99
            # Reset if too close to event horizon
            if particle["distance"] < event_horizon + 2:
                particle["distance"] = event_horizon + 20
                particle["angle"] = random.uniform(0, 2 * math.pi)

    # Draw photon sphere glow
    for angle in range(0, 360, 3):
        rad = math.radians(angle)
        for r in range(ergosphere, ergosphere + 3):
            x = bh_x + int(r * math.cos(rad) * 2.5)
            y = bh_y + int(r * math.sin(rad))
            if 0 <= y < height and 0 <= x < width:
                grid[y][x] = "." if r > ergosphere + 1 else ":"

    # Draw accretion disk particles
    chars = [".", ":", "*", "#"]
    for p in generate.particles:
        # Apply 3D perspective (particles with higher z appear dimmer)
        if abs(p["z"]) < 1.5:  # Only show particles near the viewing plane
            x = bh_x + int(p["distance"] * math.cos(p["angle"]) * 2.5)
            y = bh_y + int(p["distance"] * math.sin(p["angle"]))

            if 0 <= y < height and 0 <= x < width:
                # Brightness affected by height (z) and intrinsic brightness
                brightness = max(0, p["brightness"] - int(abs(p["z"])))
                grid[y][x] = chars[min(brightness, 3)]

    # Draw event horizon (the black hole itself)
    for dy in range(-event_horizon, event_horizon + 1):
        for dx in range(-int(event_horizon * 2.5), int(event_horizon * 2.5) + 1):
            dist = math.sqrt((dx / 2.5) ** 2 + dy**2)
            if dist <= event_horizon:
                y, x = bh_y + dy, bh_x + dx
                if 0 <= y < height and 0 <= x < width:
                    grid[y][x] = "@"

    return "\n".join(["".join(row) for row in grid])
