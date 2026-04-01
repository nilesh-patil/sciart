import random

# Pre-defined constellations (coordinates are normalized)
CONSTELLATIONS = {
    "Orion": {
        "stars": [
            (0.1, 0.1),
            (0.2, 0.15),
            (0.3, 0.2),  # Belt
            (0.05, 0.3),
            (0.35, 0.4),  # Shoulders
            (0.08, 0.0),
            (0.32, 0.05),  # Feet
        ],
        "lines": [(0, 1), (1, 2), (0, 3), (2, 4), (0, 5), (2, 6)],
    },
    "Ursa Major": {
        "stars": [
            (0.1, 0.8),
            (0.2, 0.7),
            (0.35, 0.65),
            (0.5, 0.75),  # Body
            (0.65, 0.8),
            (0.8, 0.75),
            (0.9, 0.6),  # Tail
        ],
        "lines": [(0, 1), (1, 2), (2, 3), (3, 4), (4, 5), (5, 6)],
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
                grid[y1][x1] = "."
        if x1 == x2 and y1 == y2:
            break
        e2 = 2 * err
        if e2 > -dy:
            err -= dy
            x1 += sx
        if e2 < dx:
            err += dx
            y1 += sy


def generate(width, height):
    """Generates a 2D representation of a constellation map."""
    grid = [[" " for _ in range(width)] for _ in range(height)]

    # Choose a random constellation
    if not hasattr(generate, "constellation_name") or random.random() < 0.01:
        generate.constellation_name = random.choice(list(CONSTELLATIONS.keys()))

    constellation = CONSTELLATIONS[generate.constellation_name]

    # Draw stars
    star_coords = []
    for x_norm, y_norm in constellation["stars"]:
        x = int(x_norm * width)
        y = int(y_norm * height)
        if 0 <= y < height and 0 <= x < width:
            grid[y][x] = "*"
            star_coords.append((x, y))

    # Draw lines
    for start_idx, end_idx in constellation["lines"]:
        if start_idx < len(star_coords) and end_idx < len(star_coords):
            x1, y1 = star_coords[start_idx]
            x2, y2 = star_coords[end_idx]
            draw_line(grid, x1, y1, x2, y2)

    # Draw constellation name
    name = generate.constellation_name
    name_x = width // 2 - len(name) // 2
    name_y = height - 2
    if 0 <= name_y < height:
        for i, char in enumerate(name):
            if 0 <= name_x + i < width:
                grid[name_y][name_x + i] = char

    return "\n".join(["".join(row) for row in grid])


if __name__ == "__main__":
    import time
    import os

    try:
        width, height = os.get_terminal_size()
    except OSError:
        width, height = 80, 24  # Default size

    while True:
        output = generate(width, height)
        os.system("clear" if os.name == "posix" else "cls")
        print(output)
        time.sleep(0.1)
