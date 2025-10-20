import random
import time


def generate(width, height):
    """Generates a dynamic starfield with parallax scrolling and twinkling."""
    # Initialize stars on first call
    if not hasattr(generate, "stars"):
        generate.stars = []
        # Create stars at different depths for parallax effect
        for _ in range(int(width * height * 0.12)):
            generate.stars.append(
                {
                    "x": random.uniform(0, width),
                    "y": random.uniform(0, height),
                    "depth": random.choice([1, 2, 3, 4]),  # depth layer
                    "brightness": random.choice([0, 1, 2, 3]),  # brightness level
                    "twinkle_phase": random.random() * 6.28,
                }
            )

    grid = [[" " for _ in range(width)] for _ in range(height)]

    # Star characters by brightness
    star_chars = [".", "·", "*", "★"]

    t = time.time()

    for star in generate.stars:
        # Parallax scrolling - deeper stars move slower
        scroll_speed = star["depth"] * 2
        x = (star["x"] + t * scroll_speed) % width
        y = star["y"]

        # Twinkling effect using sine wave
        twinkle = (1 + abs(time.time() * 2 + star["twinkle_phase"]) % 2 - 1) / 2

        # Only show star if twinkling allows (or if very bright)
        if twinkle > 0.3 or star["brightness"] >= 3:
            ix, iy = int(x), int(y)
            if 0 <= iy < height and 0 <= ix < width:
                # Twinkle affects brightness
                brightness = min(3, star["brightness"] + (1 if twinkle > 0.8 else 0))
                grid[iy][ix] = star_chars[brightness]

    return "\n".join(["".join(row) for row in grid])
