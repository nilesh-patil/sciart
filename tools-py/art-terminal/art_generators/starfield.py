import random
import time


def generate(width, height):
    """Enhanced starfield with nebulae, galaxies, star clusters, and varied stellar types."""
    # Initialize stars on first call
    if not hasattr(generate, "stars"):
        generate.stars = []
        # Create stars at different depths with colors and types
        for _ in range(int(width * height * 0.12)):
            star_type = random.choices(
                ["white", "blue", "yellow", "red", "variable"],
                weights=[60, 15, 15, 8, 2],
            )[0]
            generate.stars.append(
                {
                    "x": random.uniform(0, width),
                    "y": random.uniform(0, height),
                    "depth": random.choice([1, 2, 3, 4]),
                    "brightness": random.choice([0, 1, 2, 3]),
                    "twinkle_phase": random.random() * 6.28,
                    "type": star_type,
                    "variable_period": random.uniform(2, 8)
                    if star_type == "variable"
                    else 0,
                }
            )
        generate.shooting_stars = []

        # Add nebulae (gas clouds)
        generate.nebulae = []
        for _ in range(3):
            generate.nebulae.append(
                {
                    "x": random.uniform(width * 0.2, width * 0.8),
                    "y": random.uniform(height * 0.2, height * 0.8),
                    "size": random.randint(8, 15),
                    "drift_x": random.uniform(-0.1, 0.1),
                    "drift_y": random.uniform(-0.1, 0.1),
                }
            )

        # Add distant galaxies
        generate.galaxies = []
        for _ in range(5):
            generate.galaxies.append(
                {
                    "x": random.uniform(0, width),
                    "y": random.uniform(0, height),
                    "type": random.choice(["spiral", "elliptical"]),
                    "orientation": random.uniform(0, 3.14),
                    "size": random.randint(2, 4),
                }
            )

        # Add star clusters
        generate.clusters = []
        for _ in range(4):
            generate.clusters.append(
                {
                    "x": random.uniform(width * 0.2, width * 0.8),
                    "y": random.uniform(height * 0.2, height * 0.8),
                    "stars": [
                        (random.gauss(0, 3), random.gauss(0, 2)) for _ in range(15)
                    ],
                }
            )

    grid = [[" " for _ in range(width)] for _ in range(height)]
    t = time.time()

    # Draw nebulae as diffuse clouds
    import math

    for nebula in generate.nebulae:
        # Slow drift
        nebula["x"] += nebula["drift_x"]
        nebula["y"] += nebula["drift_y"]
        # Wrap around
        nebula["x"] = nebula["x"] % width
        nebula["y"] = nebula["y"] % height

        nx, ny = int(nebula["x"]), int(nebula["y"])
        size = nebula["size"]

        for dy in range(-size, size + 1):
            for dx in range(-size, size + 1):
                dist = math.sqrt(dx**2 + dy**2)
                if dist < size:
                    y, x = ny + dy, nx + dx
                    if 0 <= y < height and 0 <= x < width:
                        # Density falloff from center
                        density = 1 - (dist / size)
                        if random.random() < density * 0.3:
                            grid[y][x] = random.choice(["░", "▒", "≈", "~", "∼"])

    # Draw distant galaxies
    for galaxy in generate.galaxies:
        gx, gy = int(galaxy["x"]), int(galaxy["y"])
        if galaxy["type"] == "spiral":
            # Small spiral shape
            for arm_angle in [0, 2.09, 4.19]:  # 3 spiral arms
                for r in range(1, galaxy["size"]):
                    angle = arm_angle + r * 0.5 + galaxy["orientation"]
                    x = gx + int(r * math.cos(angle))
                    y = gy + int(r * math.sin(angle) * 0.5)  # Flattened
                    if 0 <= y < height and 0 <= x < width and grid[y][x] == " ":
                        grid[y][x] = "∴" if r == 1 else "·"
        else:  # elliptical
            for dy in range(-galaxy["size"] // 2, galaxy["size"] // 2 + 1):
                for dx in range(-galaxy["size"], galaxy["size"] + 1):
                    if (
                        dx**2 / galaxy["size"] ** 2 + dy**2 / (galaxy["size"] / 2) ** 2
                        < 0.5
                    ):
                        y, x = gy + dy, gx + dx
                        if 0 <= y < height and 0 <= x < width and grid[y][x] == " ":
                            grid[y][x] = random.choice(["·", "∙"])

    # Draw star clusters
    for cluster in generate.clusters:
        cx, cy = int(cluster["x"]), int(cluster["y"])
        for star_dx, star_dy in cluster["stars"]:
            x, y = cx + int(star_dx), cy + int(star_dy)
            if 0 <= y < height and 0 <= x < width and grid[y][x] == " ":
                grid[y][x] = random.choice(["·", "*", "+"])

    # Star characters by type and brightness
    star_chars = {
        "white": [".", "·", "*", "★"],
        "blue": ["·", "∙", "✦", "✧"],
        "yellow": [".", "◦", "◉", "⊙"],
        "red": ["·", "•", "◉", "◉"],
        "variable": [".", "·", "*", "★"],
    }

    for star in generate.stars:
        # Parallax scrolling - deeper stars move slower
        scroll_speed = star["depth"] * 2
        x = (star["x"] + t * scroll_speed) % width
        y = star["y"]

        # Twinkling effect
        twinkle = (1 + abs(time.time() * 2 + star["twinkle_phase"]) % 2 - 1) / 2

        # Variable star brightness changes over time
        brightness = star["brightness"]
        if star["type"] == "variable":
            var_bright = (1 + math.sin(t / star["variable_period"])) / 2
            brightness = int(brightness * var_bright)
            brightness = max(0, min(3, brightness))

        # Only show star if twinkling allows (or if very bright)
        if twinkle > 0.3 or brightness >= 3:
            ix, iy = int(x), int(y)
            if 0 <= iy < height and 0 <= ix < width:
                # Twinkle affects brightness
                final_brightness = min(3, brightness + (1 if twinkle > 0.8 else 0))
                char = star_chars[star["type"]][final_brightness]
                if grid[iy][ix] in [" ", "░"]:  # Don't overwrite nebula cores
                    grid[iy][ix] = char

    # Spawn shooting stars randomly
    if random.random() < 0.02 and len(generate.shooting_stars) < 3:
        generate.shooting_stars.append(
            {
                "x": random.uniform(0, width),
                "y": 0,
                "vx": random.uniform(-3, 3),
                "vy": random.uniform(2, 5),
                "trail_length": random.randint(5, 12),
                "age": 0,
            }
        )

    # Update and draw shooting stars
    stars_to_remove = []
    for meteor in generate.shooting_stars:
        meteor["x"] += meteor["vx"]
        meteor["y"] += meteor["vy"]
        meteor["age"] += 1

        # Remove if off screen or too old
        if (
            meteor["y"] >= height
            or meteor["x"] < 0
            or meteor["x"] >= width
            or meteor["age"] > 50
        ):
            stars_to_remove.append(meteor)
        else:
            # Draw meteor trail
            for i in range(meteor["trail_length"]):
                trail_x = int(meteor["x"] - i * meteor["vx"] * 0.2)
                trail_y = int(meteor["y"] - i * meteor["vy"] * 0.2)
                if 0 <= trail_y < height and 0 <= trail_x < width:
                    if i == 0:
                        grid[trail_y][trail_x] = "*"
                    elif i < meteor["trail_length"] // 2:
                        grid[trail_y][trail_x] = "·"
                    else:
                        grid[trail_y][trail_x] = "."

    for meteor in stars_to_remove:
        generate.shooting_stars.remove(meteor)

    return "\n".join(["".join(row) for row in grid])
