import random


def generate(width, height):
    """Generates a visualization of Brownian motion.

    Shows a large particle being jostled by smaller invisible particles,
    following a random walk pattern.
    """
    # Initialize particle position on first call
    if not hasattr(generate, "particle_x"):
        generate.particle_x = width // 2
        generate.particle_y = height // 2
        generate.trail = []

    # Random walk step
    dx = random.choice([-1, 0, 1])
    dy = random.choice([-1, 0, 1])

    generate.particle_x = max(0, min(width - 1, generate.particle_x + dx))
    generate.particle_y = max(0, min(height - 1, generate.particle_y + dy))

    # Add to trail (keep last 100 positions)
    generate.trail.append((generate.particle_x, generate.particle_y))
    if len(generate.trail) > 100:
        generate.trail.pop(0)

    grid = [[" " for _ in range(width)] for _ in range(height)]

    # Draw trail with fading effect
    trail_chars = [".", ":", "*", "O"]
    trail_len = len(generate.trail)
    for idx, (x, y) in enumerate(generate.trail):
        if 0 <= y < height and 0 <= x < width:
            # Older positions get fainter characters
            char_idx = min(
                int((idx / trail_len) * len(trail_chars)), len(trail_chars) - 1
            )
            grid[y][x] = trail_chars[char_idx]

    # Draw current particle position
    if 0 <= generate.particle_y < height and 0 <= generate.particle_x < width:
        grid[generate.particle_y][generate.particle_x] = "@"

    return "\n".join(["".join(row) for row in grid])


if __name__ == "__main__":
    import os
    import time

    try:
        width, height = os.get_terminal_size()
    except OSError:
        width, height = 80, 24

    while True:
        output = generate(width, height)
        os.system("clear" if os.name == "posix" else "cls")
        print(output)
        time.sleep(0.1)
