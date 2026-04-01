import random
import math
import time


def generate(width, height):
    """Black hole with accretion disk and relativistic jets."""
    grid = [[" " for _ in range(width)] for _ in range(height)]

    bh_x, bh_y = width // 2, height // 2
    event_horizon = 4
    isco = 6
    ergosphere = 7
    t = time.time()

    # Initialize particles for accretion disk
    if not hasattr(generate, "particles"):
        generate.particles = []
        for _ in range(500):  # Reduced from 800
            angle = random.uniform(0, 2 * math.pi)
            distance = random.uniform(isco, event_horizon + 20)
            speed = 1.5 / math.sqrt(distance)
            z = random.uniform(-2, 2)
            temp_factor = 1.0 / (distance / isco)
            brightness = min(3, int(temp_factor * 2))
            generate.particles.append(
                {
                    "angle": angle,
                    "distance": distance,
                    "speed": speed,
                    "z": z,
                    "brightness": brightness,
                }
            )

    # Update particle positions with frame dragging
    for particle in generate.particles:
        frame_drag = 0.05 * (event_horizon / particle["distance"]) ** 2
        particle["angle"] += particle["speed"] + frame_drag

        if random.random() < 0.001:
            particle["distance"] *= 0.99
            if particle["distance"] < isco + 1:
                particle["distance"] = event_horizon + 20
                particle["angle"] = random.uniform(0, 2 * math.pi)
                temp_factor = 1.0 / (particle["distance"] / isco)
                particle["brightness"] = min(3, int(temp_factor * 2))

    # Draw ergosphere boundary (sparse)
    for angle in range(0, 360, 15):  # Reduced from 3
        rad = math.radians(angle)
        for r in range(ergosphere, ergosphere + 2):
            x = bh_x + int(r * math.cos(rad) * 2.5)
            y = bh_y + int(r * math.sin(rad))
            if 0 <= y < height and 0 <= x < width:
                if grid[y][x] == " ":
                    grid[y][x] = "∘"

    # Draw accretion disk particles
    chars = ["·", ":", "∴", "※"]
    for p in generate.particles:
        if abs(p["z"]) < 1.5:
            x = bh_x + int(p["distance"] * math.cos(p["angle"]) * 2.5)
            y = bh_y + int(p["distance"] * math.sin(p["angle"]))

            if 0 <= y < height and 0 <= x < width:
                brightness = max(0, p["brightness"] - int(abs(p["z"])))
                grid[y][x] = chars[min(brightness, 3)]

    # Draw event horizon
    for dy in range(-event_horizon, event_horizon + 1):
        for dx in range(-int(event_horizon * 2.5), int(event_horizon * 2.5) + 1):
            dist = math.sqrt((dx / 2.5) ** 2 + dy**2)
            if dist <= event_horizon:
                y, x = bh_y + dy, bh_x + dx
                if 0 <= y < height and 0 <= x < width:
                    if dist < event_horizon * 0.4:
                        grid[y][x] = "●"
                    elif dist < event_horizon * 0.7:
                        grid[y][x] = "▓"
                    elif dist < event_horizon * 0.9:
                        grid[y][x] = "▒"
                    else:
                        grid[y][x] = "░"

    # Draw relativistic jets shooting from poles
    jet_length = min(height // 2 - 5, 20)
    jet_chars = ["│", "┆", "╎", "╏"]

    for direction in [-1, 1]:
        for dist in range(event_horizon + 2, jet_length):
            jet_width = max(1, int(3 - dist * 0.1))
            pulse = math.sin(t * 3 + dist * 0.3) * 0.5 + 0.5

            for offset in range(-jet_width, jet_width + 1):
                y = bh_y + direction * dist
                x = bh_x + int(offset * 2.5)

                if 0 <= y < height and 0 <= x < width:
                    intensity = max(0, min(3, int((1 - dist / jet_length) * 3 * pulse)))
                    if intensity > 0:
                        char = jet_chars[3 - intensity]
                        if grid[y][x] == " ":
                            grid[y][x] = char

    # Add jet particles
    if not hasattr(generate, "jet_particles"):
        generate.jet_particles = []

    if random.random() < 0.3:
        for direction in [-1, 1]:
            generate.jet_particles.append(
                {
                    "x": bh_x,
                    "y": bh_y + direction * (event_horizon + 3),
                    "vy": direction * random.uniform(0.5, 1.2),
                    "vx": random.uniform(-0.3, 0.3),
                    "age": 0,
                }
            )

    particles_to_remove = []
    for jp in generate.jet_particles:
        jp["x"] += jp["vx"]
        jp["y"] += jp["vy"]
        jp["age"] += 1

        if (
            jp["age"] > 40
            or jp["y"] < 0
            or jp["y"] >= height
            or jp["x"] < 0
            or jp["x"] >= width
        ):
            particles_to_remove.append(jp)
        else:
            jx, jy = int(jp["x"]), int(jp["y"])
            if 0 <= jy < height and 0 <= jx < width and grid[jy][jx] == " ":
                grid[jy][jx] = "·"

    for jp in particles_to_remove:
        generate.jet_particles.remove(jp)

    return "\n".join(["".join(row) for row in grid])
