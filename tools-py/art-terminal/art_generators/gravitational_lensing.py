import random
import math
import time


def generate(width, height):
    """Gravitational lensing with Einstein rings, crosses, and arcs."""
    grid = [[" " for _ in range(width)] for _ in range(height)]

    lens_center_x, lens_center_y = width // 2, height // 2

    # Multiple lensing masses (galaxy cluster)
    if not hasattr(generate, "lenses"):
        generate.lenses = [
            {"x": lens_center_x, "y": lens_center_y, "mass": 100, "einstein_r": 12},
            {
                "x": lens_center_x + 15,
                "y": lens_center_y - 8,
                "mass": 40,
                "einstein_r": 6,
            },
            {
                "x": lens_center_x - 12,
                "y": lens_center_y + 10,
                "mass": 35,
                "einstein_r": 5,
            },
        ]

    # Generate background sources
    if not hasattr(generate, "sources"):
        generate.sources = []

        # Perfectly aligned quasar for primary Einstein ring
        generate.sources.append(
            {
                "x": lens_center_x,
                "y": lens_center_y,
                "type": "quasar",
                "brightness": 5,
                "size": 1,
            }
        )

        # Quasar for Einstein Cross (4-image configuration)
        generate.sources.append(
            {
                "x": lens_center_x + 3,
                "y": lens_center_y + 2,
                "type": "quasar",
                "brightness": 4,
                "size": 1,
            }
        )

        # Extended background galaxies
        for _ in range(15):
            generate.sources.append(
                {
                    "x": random.uniform(width * 0.2, width * 0.8),
                    "y": random.uniform(height * 0.2, height * 0.8),
                    "type": "galaxy",
                    "brightness": random.randint(2, 3),
                    "size": random.randint(2, 4),
                    "orientation": random.uniform(0, math.pi),
                }
            )

        # Point sources (distant stars/quasars) - reduced
        for _ in range(50):  # Reduced from 100
            generate.sources.append(
                {
                    "x": random.uniform(0, width),
                    "y": random.uniform(0, height),
                    "type": "star",
                    "brightness": random.randint(1, 2),
                    "size": 1,
                }
            )

    t = time.time()

    # Process lensing for each source
    for source_idx, source in enumerate(generate.sources):
        if source["type"] in ["quasar", "galaxy"]:
            sx, sy = source["x"], source["y"]

            for lens in generate.lenses:
                dx = sx - lens["x"]
                dy = sy - lens["y"]
                dist = math.sqrt(dx**2 + dy**2)

                if dist < 0.5:
                    continue

                # Primary lens creates ring for aligned source
                if dist < 3 and source_idx == 0:
                    # Draw Einstein ring
                    for angle in range(0, 360, 3):  # Reduced from 2
                        rad = math.radians(angle)
                        ring_r = lens["einstein_r"] * (
                            1.0 + 0.05 * math.sin(t + angle / 10)
                        )
                        ring_x = lens["x"] + int(ring_r * math.cos(rad) * 2.5)
                        ring_y = lens["y"] + int(ring_r * math.sin(rad))
                        if 0 <= ring_y < height and 0 <= ring_x < width:
                            if grid[ring_y][ring_x] == " ":
                                grid[ring_y][ring_x] = "○"

                # Einstein Cross configuration (4 images)
                elif dist < 8 and source_idx == 1:
                    angle = math.atan2(dy, dx)
                    # Create 4 images at cardinal directions
                    for img_angle in [0, math.pi / 2, math.pi, 3 * math.pi / 2]:
                        img_r = lens["einstein_r"] * 0.8
                        img_x = lens["x"] + int(
                            img_r * math.cos(angle + img_angle) * 2.5
                        )
                        img_y = lens["y"] + int(img_r * math.sin(angle + img_angle))
                        if 0 <= img_y < height and 0 <= img_x < width:
                            grid[img_y][img_x] = "✦"

                # Gravitational arcs from extended galaxies
                elif source["type"] == "galaxy" and dist < lens["einstein_r"] * 2:
                    angle = math.atan2(dy, dx)
                    # Create arc
                    arc_length = 60
                    for arc_angle in range(-arc_length // 2, arc_length // 2, 5):  # Reduced density
                        arc_rad = math.radians(arc_angle)
                        arc_r = lens["einstein_r"] * (1.0 + random.uniform(-0.2, 0.2))
                        arc_x = lens["x"] + int(arc_r * math.cos(angle + arc_rad) * 2.5)
                        arc_y = lens["y"] + int(arc_r * math.sin(angle + arc_rad))
                        if 0 <= arc_y < height and 0 <= arc_x < width:
                            if grid[arc_y][arc_x] == " ":
                                grid[arc_y][arc_x] = random.choice(["◠", "◡"])

    # Draw lensing galaxy cluster
    for lens in generate.lenses:
        lx, ly = int(lens["x"]), int(lens["y"])
        size = int(lens["mass"] / 20)

        for dy in range(-size, size + 1):
            for dx in range(-int(size * 2.5), int(size * 2.5) + 1):
                dist = math.sqrt((dx / 2.5) ** 2 + dy**2)
                if dist <= size:
                    y, x = ly + dy, lx + dx
                    if 0 <= y < height and 0 <= x < width:
                        if dist < size * 0.4:
                            grid[y][x] = "⊙"
                        elif dist < size * 0.7:
                            grid[y][x] = "◉"
                        elif dist < size * 0.9:
                            grid[y][x] = "○"

    # Draw minimal background stars
    for source in generate.sources:
        if source["type"] == "star":
            sx, sy = int(source["x"]), int(source["y"])
            min_dist_to_lens = min(
                math.sqrt(
                    (source["x"] - lens["x"]) ** 2 + (source["y"] - lens["y"]) ** 2
                )
                for lens in generate.lenses
            )
            if min_dist_to_lens > 25:
                if 0 <= sy < height and 0 <= sx < width:
                    if grid[sy][sx] == " ":
                        grid[sy][sx] = "·" if source["brightness"] == 1 else "✦"

    return "\n".join(["".join(row) for row in grid])
