import random
import time
import math


def generate(width, height):
    """Minimal starfield with sparse stars, depth variation, and occasional shooting star.

    Pure black background with <0.5% star density.
    """
    # Initialize stars on first call
    if not hasattr(generate, "stars"):
        generate.stars = []
        # Very sparse star distribution: 0.3% density
        for _ in range(int(width * height * 0.003)):
            generate.stars.append(
                {
                    "x": random.uniform(0, width),
                    "y": random.uniform(0, height),
                    "depth": random.choice([1, 2, 3]),  # For parallax
                    "brightness": random.choice([0, 0, 0, 1, 1, 2]),  # Mostly dim
                    "twinkle_phase": random.random() * 6.28,
                }
            )
        generate.shooting_stars = []

    grid = [[" " for _ in range(width)] for _ in range(height)]
    t = time.time()

    # Star characters by brightness (only approved chars)
    star_chars = ["·", ".", "˙"]  # Sparse, minimal

    for star in generate.stars:
        # Parallax scrolling - deeper stars move slower
        scroll_speed = star["depth"] * 0.5
        x = (star["x"] + t * scroll_speed) % width
        y = star["y"]

        # Subtle twinkling
        twinkle = 0.7 + 0.3 * abs(math.sin(t * 2 + star["twinkle_phase"]))

        # Only show star if twinkling allows
        if random.random() < twinkle:
            ix, iy = int(x), int(y)
            if 0 <= iy < height and 0 <= ix < width:
                char = star_chars[min(star["brightness"], 2)]
                grid[iy][ix] = char

    # Spawn shooting stars rarely
    if random.random() < 0.01 and len(generate.shooting_stars) < 2:
        generate.shooting_stars.append(
            {
                "x": random.uniform(0, width),
                "y": 0,
                "vx": random.uniform(-2, 2),
                "vy": random.uniform(3, 6),
                "trail_length": random.randint(4, 8),
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
            or meteor["age"] > 40
        ):
            stars_to_remove.append(meteor)
        else:
            # Draw meteor trail (sparse)
            for i in range(meteor["trail_length"]):
                trail_x = int(meteor["x"] - i * meteor["vx"] * 0.3)
                trail_y = int(meteor["y"] - i * meteor["vy"] * 0.3)
                if 0 <= trail_y < height and 0 <= trail_x < width:
                    if grid[trail_y][trail_x] == " ":
                        if i == 0:
                            grid[trail_y][trail_x] = "∘"
                        elif i < meteor["trail_length"] // 2:
                            grid[trail_y][trail_x] = "·"
                        else:
                            grid[trail_y][trail_x] = "˙"

    for meteor in stars_to_remove:
        generate.shooting_stars.remove(meteor)

    return "\n".join(["".join(row) for row in grid])
