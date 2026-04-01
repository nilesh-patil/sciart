import random
import math
import time


def generate(width, height):
    """Generates a vibrant nebula with swirling gas, dust lanes, and forming stars."""
    # Initialize state
    if not hasattr(generate, "time_offset"):
        generate.time_offset = 0
        generate.proto_stars = []
        generate.pillars = []  # Pillars of creation-like structures

        # Create some dust pillars
        for _ in range(3):
            generate.pillars.append(
                {
                    "x": random.randint(width // 4, 3 * width // 4),
                    "y": random.randint(0, height),
                    "width": random.randint(3, 8),
                    "height": random.randint(10, 20),
                }
            )

    generate.time_offset += 0.05

    grid = [[" " for _ in range(width)] for _ in range(height)]

    # Multi-layer nebula with different densities
    for i in range(height):
        for j in range(width):
            # Swirling pattern using time-based sine waves
            t = generate.time_offset
            wave1 = math.sin((j * 0.1 + i * 0.05 + t * 2)) * 0.5 + 0.5
            wave2 = math.cos((j * 0.08 - i * 0.06 + t * 1.5)) * 0.5 + 0.5
            wave3 = math.sin((j + i) * 0.04 + t) * 0.5 + 0.5

            # Combine waves
            density = wave1 * 0.4 + wave2 * 0.3 + wave3 * 0.3

            # Add radial gradient from center
            cx, cy = width // 2, height // 2
            dist_from_center = math.sqrt(
                ((j - cx) / width) ** 2 + ((i - cy) / height) ** 2
            )
            density *= 1 - dist_from_center * 0.5

            # Map to characters
            if density > 0.7:
                grid[i][j] = "#"  # Dense gas
            elif density > 0.5:
                grid[i][j] = "+"  # Medium density
            elif density > 0.3:
                grid[i][j] = ":"  # Thin gas
            elif density > 0.15:
                grid[i][j] = "."  # Very thin
            # else remains " "

    # Draw dust pillars
    for pillar in generate.pillars:
        px, py = pillar["x"], pillar["y"]
        pw, ph = pillar["width"], pillar["height"]

        for dy in range(ph):
            # Pillar gets narrower toward top
            width_at_y = max(1, int(pw * (1 - dy / ph * 0.6)))
            for dx in range(-width_at_y // 2, width_at_y // 2 + 1):
                y, x = py - dy, px + dx
                if 0 <= y < height and 0 <= x < width:
                    grid[y][x] = "█" if dy < ph // 2 else "▓"

    # Spawn and update proto-stars
    if random.random() < 0.05 and len(generate.proto_stars) < 20:
        generate.proto_stars.append(
            {
                "x": random.randint(0, width - 1),
                "y": random.randint(0, height - 1),
                "age": 0,
                "max_age": random.randint(50, 150),
            }
        )

    stars_to_remove = []
    for star in generate.proto_stars:
        star["age"] += 1

        if star["age"] > star["max_age"]:
            stars_to_remove.append(star)
        else:
            # Star brightness increases then fades
            progress = star["age"] / star["max_age"]
            if progress < 0.3:
                brightness = progress / 0.3
            else:
                brightness = 1 - (progress - 0.3) / 0.7

            x, y = star["x"], star["y"]
            if 0 <= y < height and 0 <= x < width:
                if brightness > 0.7:
                    grid[y][x] = "★"
                elif brightness > 0.4:
                    grid[y][x] = "*"
                elif brightness > 0.2:
                    grid[y][x] = "+"

                # Star glow
                if brightness > 0.5:
                    for dx, dy in [(-1, 0), (1, 0), (0, -1), (0, 1)]:
                        gx, gy = x + dx, y + dy
                        if (
                            0 <= gy < height
                            and 0 <= gx < width
                            and grid[gy][gx] in [" ", ".", ":"]
                        ):
                            grid[gy][gx] = "·"

    for star in stars_to_remove:
        generate.proto_stars.remove(star)

    return "\n".join(["".join(row) for row in grid])


if __name__ == "__main__":
    import os

    try:
        width, height = os.get_terminal_size()
    except OSError:
        width, height = 80, 24

    while True:
        output = generate(width, height)
        os.system("clear" if os.name == "posix" else "cls")
        print(output)
        time.sleep(0.1)
