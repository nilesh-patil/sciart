import random
import time
import math

# Enhanced constellations with star magnitudes
CONSTELLATIONS = {
    "Orion": {
        "stars": [
            (0.5, 0.25, 0.0),  # Betelgeuse
            (0.65, 0.7, 0.3),  # Rigel
            (0.45, 0.5, 1.8),  # Alnitak
            (0.5, 0.52, 2.0),  # Alnilam
            (0.55, 0.5, 1.7),  # Mintaka
            (0.42, 0.35, 2.2),  # Bellatrix
            (0.48, 0.72, 2.8),  # Saiph
            (0.58, 0.35, 3.0),
            (0.62, 0.72, 3.0),
        ],
        "lines": [
            (0, 5),
            (5, 2),
            (2, 3),
            (3, 4),
            (4, 7),
            (7, 1),
            (1, 8),
            (8, 4),
            (2, 6),
            (4, 1),
        ],
        "deep_sky": [(0.5, 0.62, "nebula")],  # Orion Nebula
    },
    "Ursa Major": {
        "stars": [
            (0.2, 0.3, 1.8),
            (0.3, 0.25, 2.3),
            (0.42, 0.35, 2.4),
            (0.52, 0.3, 2.4),
            (0.62, 0.35, 1.8),
            (0.72, 0.32, 2.2),
            (0.82, 0.38, 1.9),
        ],
        "lines": [(0, 1), (1, 2), (2, 3), (3, 4), (4, 5), (5, 6), (3, 0)],
        "deep_sky": [(0.65, 0.55, "galaxy"), (0.68, 0.6, "galaxy")],
    },
    "Cassiopeia": {
        "stars": [
            (0.15, 0.5, 2.2),
            (0.3, 0.35, 2.3),
            (0.5, 0.45, 2.5),
            (0.7, 0.32, 2.7),
            (0.85, 0.5, 3.4),
        ],
        "lines": [(0, 1), (1, 2), (2, 3), (3, 4)],
        "deep_sky": [(0.45, 0.6, "cluster")],
    },
    "Scorpius": {
        "stars": [
            (0.5, 0.4, 0.9),
            (0.45, 0.35, 2.6),
            (0.52, 0.3, 2.3),
            (0.58, 0.38, 2.9),
            (0.63, 0.48, 2.3),
            (0.68, 0.58, 1.6),
            (0.72, 0.68, 2.7),
            (0.75, 0.75, 3.0),
        ],
        "lines": [(0, 1), (1, 2), (2, 3), (3, 4), (4, 5), (5, 6), (6, 7)],
        "deep_sky": [(0.6, 0.5, "cluster")],
    },
    "Cygnus": {
        "stars": [
            (0.5, 0.2, 1.3),
            (0.5, 0.5, 2.2),
            (0.5, 0.75, 1.3),
            (0.25, 0.5, 2.5),
            (0.75, 0.5, 2.9),
        ],
        "lines": [(0, 1), (1, 2), (1, 3), (1, 4)],
        "deep_sky": [(0.6, 0.45, "nebula")],
    },
    "Leo": {
        "stars": [
            (0.4, 0.5, 1.4),
            (0.5, 0.45, 2.6),
            (0.6, 0.4, 2.0),
            (0.65, 0.3, 2.1),
            (0.7, 0.35, 3.4),
            (0.35, 0.35, 2.3),
            (0.3, 0.45, 3.5),
        ],
        "lines": [(0, 1), (1, 2), (2, 3), (3, 4), (2, 5), (5, 6), (6, 0)],
        "deep_sky": [(0.55, 0.55, "galaxy"), (0.58, 0.58, "galaxy")],
    },
    "Andromeda": {
        "stars": [
            (0.35, 0.4, 2.1),
            (0.5, 0.35, 2.0),
            (0.65, 0.45, 2.3),
            (0.48, 0.25, 3.3),
        ],
        "lines": [(0, 1), (1, 2), (1, 3)],
        "deep_sky": [(0.52, 0.3, "galaxy")],
    },
    "Aquila": {
        "stars": [
            (0.5, 0.5, 0.8),
            (0.45, 0.42, 2.7),
            (0.55, 0.42, 2.7),
            (0.42, 0.6, 3.2),
            (0.58, 0.6, 3.2),
        ],
        "lines": [(1, 0), (0, 2), (0, 3), (0, 4)],
        "deep_sky": [],
    },
    "Lyra": {
        "stars": [
            (0.5, 0.3, 0.0),
            (0.42, 0.5, 3.2),
            (0.58, 0.5, 3.2),
            (0.48, 0.6, 4.3),
            (0.52, 0.6, 4.3),
        ],
        "lines": [(0, 1), (0, 2), (1, 3), (2, 4), (3, 4)],
        "deep_sky": [(0.55, 0.45, "nebula")],
    },
    "Taurus": {
        "stars": [
            (0.4, 0.55, 0.9),
            (0.5, 0.4, 2.9),
            (0.35, 0.35, 3.0),
            (0.3, 0.45, 3.4),
            (0.55, 0.6, 1.7),
        ],
        "lines": [(0, 1), (0, 2), (0, 3), (1, 4)],
        "deep_sky": [
            (0.28, 0.38, "cluster"),
            (0.42, 0.52, "nebula"),
        ],
    },
    "Gemini": {
        "stars": [
            (0.35, 0.3, 1.6),
            (0.45, 0.25, 1.9),
            (0.32, 0.55, 3.0),
            (0.42, 0.6, 2.9),
            (0.38, 0.7, 3.5),
            (0.48, 0.68, 3.3),
        ],
        "lines": [(0, 1), (0, 2), (2, 4), (1, 3), (3, 5)],
        "deep_sky": [(0.4, 0.45, "cluster")],
    },
    "Pegasus": {
        "stars": [
            (0.25, 0.25, 2.4),
            (0.7, 0.25, 2.8),
            (0.7, 0.7, 2.4),
            (0.25, 0.7, 2.5),
        ],
        "lines": [(0, 1), (1, 2), (2, 3), (3, 0)],
        "deep_sky": [(0.48, 0.48, "cluster")],
    },
}


def draw_line(grid, x1, y1, x2, y2):
    """Draws a line on the grid using Bresenham's line algorithm."""
    dx = abs(x2 - x1)
    dy = abs(y2 - y1)
    sx = 1 if x1 < x2 else -1
    sy = 1 if y1 < y2 else -1
    err = dx - dy

    while True:
        if 0 <= y1 < len(grid) and 0 <= x1 < len(grid[0]):
            if grid[y1][x1] == " ":
                grid[y1][x1] = "─" if abs(dx) > abs(dy) else "│"
        if x1 == x2 and y1 == y2:
            break
        e2 = 2 * err
        if e2 > -dy:
            err -= dy
            x1 += sx
        if e2 < dx:
            err += dx
            y1 += sy


def generate(grid_width, grid_height):
    """Generates constellation map with stars and deep-sky objects."""
    grid = [[" " for _ in range(grid_width)] for _ in range(grid_height)]

    # Initialize background stars with varied brightness
    if not hasattr(generate, "background_stars"):
        generate.background_stars = []
        # Drastically reduced background stars
        for _ in range(int(grid_width * grid_height * 0.001)):  # Reduced from 0.03
            generate.background_stars.append(
                {
                    "x": random.randint(0, grid_width - 1),
                    "y": random.randint(0, grid_height - 1),
                    "brightness": random.choice([0, 0, 0, 1]),  # Mostly faint
                    "twinkle_phase": random.random() * 6.28,
                }
            )

    t = time.time()

    # Draw background stars with twinkling
    bg_chars = ["·", "∘"]
    for star in generate.background_stars:
        x, y = star["x"], star["y"]
        if 0 <= y < grid_height and 0 <= x < grid_width:
            twinkle = 0.7 + 0.3 * math.sin(t * 2 + star["twinkle_phase"])
            if random.random() < twinkle:
                if grid[y][x] == " ":
                    grid[y][x] = bg_chars[min(star["brightness"], 1)]

    # Choose constellation (change less frequently)
    if not hasattr(generate, "constellation_name") or random.random() < 0.005:
        generate.constellation_name = random.choice(list(CONSTELLATIONS.keys()))

    constellation = CONSTELLATIONS[generate.constellation_name]

    # Draw constellation lines first
    star_coords = []
    for star_data in constellation["stars"]:
        x_norm, y_norm = star_data[0], star_data[1]
        x = int(x_norm * grid_width)
        y = int(y_norm * grid_height)
        star_coords.append((x, y))

    for start_idx, end_idx in constellation["lines"]:
        if start_idx < len(star_coords) and end_idx < len(star_coords):
            x1, y1 = star_coords[start_idx]
            x2, y2 = star_coords[end_idx]
            draw_line(grid, x1, y1, x2, y2)

    # Draw constellation stars with magnitude-based size
    for idx, star_data in enumerate(constellation["stars"]):
        x_norm, y_norm, magnitude = star_data
        x, y = star_coords[idx]

        if 0 <= y < grid_height and 0 <= x < grid_width:
            # Brightness based on magnitude (lower = brighter)
            if magnitude < 1.5:
                grid[y][x] = "✦"
                # Add glow around bright stars
                for dy, dx in [(-1, 0), (1, 0), (0, -1), (0, 1)]:
                    ny, nx = y + dy, x + dx
                    if (
                        0 <= ny < grid_height
                        and 0 <= nx < grid_width
                        and grid[ny][nx] in [" ", "·", "∘"]
                    ):
                        grid[ny][nx] = "∘"
            elif magnitude < 2.5:
                grid[y][x] = "○"
            else:
                grid[y][x] = "∘"

    # Draw deep-sky objects
    for dso in constellation["deep_sky"]:
        dso_x_norm, dso_y_norm, dso_type = dso
        dso_x = int(dso_x_norm * grid_width)
        dso_y = int(dso_y_norm * grid_height)

        if dso_type == "nebula":
            # Draw nebula as diffuse cloud
            for dy in range(-2, 3):
                for dx in range(-4, 5):
                    dist = math.sqrt((dx / 2) ** 2 + dy**2)
                    if dist < 2.5:
                        ny, nx = dso_y + dy, dso_x + dx
                        if 0 <= ny < grid_height and 0 <= nx < grid_width:
                            if grid[ny][nx] in [" ", "·", "∘"]:
                                grid[ny][nx] = random.choice(["░", "▒"])
        elif dso_type == "galaxy":
            # Draw galaxy as elongated ellipse
            for dy in range(-1, 2):
                for dx in range(-3, 4):
                    if abs(dy) + abs(dx) / 2 < 2:
                        ny, nx = dso_y + dy, dso_x + dx
                        if 0 <= ny < grid_height and 0 <= nx < grid_width:
                            if grid[ny][nx] in [" ", "·", "∘"]:
                                grid[ny][nx] = "∴" if abs(dy) == 0 else "·"
        elif dso_type == "cluster":
            # Draw cluster as tight group of stars
            for _ in range(6):  # Reduced from 8
                dy = random.randint(-1, 1)
                dx = random.randint(-2, 2)
                ny, nx = dso_y + dy, dso_x + dx
                if 0 <= ny < grid_height and 0 <= nx < grid_width:
                    if grid[ny][nx] in [" ", "·", "∘"]:
                        grid[ny][nx] = random.choice(["·", "∘"])

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
