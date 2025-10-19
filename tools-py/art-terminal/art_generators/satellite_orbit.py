import math
import time


def generate(screen_width, screen_height):
    """Generates a view of Earth from space with orbiting satellites and the ISS.

    Shows Earth in the center with satellites in different orbital paths and trails.
    """
    grid = [[" " for _ in range(screen_width)] for _ in range(screen_height)]

    earth_radius = min(screen_width, screen_height) // 4
    earth_x, earth_y = screen_width // 2, screen_height // 2

    # Draw Earth with continents - reduced density
    for i in range(screen_height):
        for j in range(screen_width):
            distance = math.sqrt(((j - earth_x) / 2.5) ** 2 + (i - earth_y) ** 2)
            if distance <= earth_radius:
                # Create continent-like patterns
                noise_val = ((i * 7 + j * 13) % 17) / 17.0
                if noise_val < 0.25:  # Reduced from 0.3
                    grid[i][j] = "▓"  # Land masses
                elif noise_val < 0.3:  # Reduced from 0.4
                    grid[i][j] = " "  # Reduced coasts
                else:
                    # Ocean - much sparser
                    if noise_val < 0.5:
                        grid[i][j] = "~"
                    elif noise_val < 0.6:
                        grid[i][j] = "≈"
                    else:
                        grid[i][j] = " "  # More empty ocean

    # Define satellites with different orbital parameters - NO LABELS
    satellites = [
        {
            "distance": earth_radius + 5,
            "speed": 0.02,
            "trail": True,
        },
        {
            "distance": earth_radius + 8,
            "speed": 0.018,
            "trail": True,
        },
        {
            "distance": earth_radius + 12,
            "speed": 0.012,
            "trail": True,
        },
        {
            "distance": earth_radius + 4,
            "speed": 0.025,
            "trail": True,
        },
    ]

    # Draw orbital paths (faint) - much sparser
    for sat in satellites:
        for angle in range(0, 360, 15):  # Increased from 8 for less density
            rad = math.radians(angle)
            x = earth_x + int(sat["distance"] * math.cos(rad) * 2.5)
            y = earth_y + int(sat["distance"] * math.sin(rad))
            if 0 <= y < screen_height and 0 <= x < screen_width and grid[y][x] == " ":
                grid[y][x] = "∘"

    # Draw satellites with trails - all same character
    for sat in satellites:
        angle = time.time() * sat["speed"]
        x = earth_x + int(sat["distance"] * math.cos(angle) * 2.5)
        y = earth_y + int(sat["distance"] * math.sin(angle))

        # Draw trail - sparser
        if sat["trail"]:
            for trail_idx in range(1, 8):
                trail_angle = angle - (trail_idx * 0.08)
                tx = earth_x + int(sat["distance"] * math.cos(trail_angle) * 2.5)
                ty = earth_y + int(sat["distance"] * math.sin(trail_angle))
                if (
                    0 <= ty < screen_height
                    and 0 <= tx < screen_width
                    and grid[ty][tx] == " "
                ):
                    grid[ty][tx] = "·"

        # Draw satellite - all the same character, no labels
        if 0 <= y < screen_height and 0 <= x < screen_width:
            grid[y][x] = "◆"

    # Add atmosphere glow around Earth - much sparser
    for angle in range(0, 360, 10):  # Increased from 5
        rad = math.radians(angle)
        glow_dist = earth_radius + 1
        x = earth_x + int(glow_dist * math.cos(rad) * 2.5)
        y = earth_y + int(glow_dist * math.sin(rad))
        if 0 <= y < screen_height and 0 <= x < screen_width and grid[y][x] == " ":
            grid[y][x] = "░"

    return "\n".join(["".join(row) for row in grid])


if __name__ == "__main__":
    import os

    try:
        term_width, term_height = os.get_terminal_size()
    except OSError:
        term_width, term_height = 80, 24

    while True:
        output = generate(term_width, term_height)
        os.system("clear" if os.name == "posix" else "cls")
        print(output)
        time.sleep(0.1)
