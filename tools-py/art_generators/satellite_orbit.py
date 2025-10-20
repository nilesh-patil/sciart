import math
import time


def generate(width, height):
    """Generates a view of Earth from space with orbiting satellites.

    Shows Earth in the center with satellites in different orbital paths.
    """
    grid = [[" " for _ in range(width)] for _ in range(height)]

    earth_radius = min(width, height) // 4
    earth_x, earth_y = width // 2, height // 2

    # Draw Earth
    for i in range(height):
        for j in range(width):
            distance = math.sqrt(((j - earth_x) / 2.5) ** 2 + (i - earth_y) ** 2)
            if distance <= earth_radius:
                # Simple pattern for Earth's surface
                if (i + j) % 3 == 0:
                    grid[i][j] = "~"  # Water
                else:
                    grid[i][j] = "#"  # Land

    # Define satellites with different orbital parameters
    satellites = [
        {"distance": earth_radius + 5, "speed": 0.02, "char": "S"},
        {"distance": earth_radius + 10, "speed": 0.015, "char": "s"},
        {"distance": earth_radius + 15, "speed": 0.01, "char": "s"},
    ]

    # Draw satellites
    for sat in satellites:
        angle = time.time() * sat["speed"]
        x = earth_x + int(sat["distance"] * math.cos(angle) * 2.5)
        y = earth_y + int(sat["distance"] * math.sin(angle))

        if 0 <= y < height and 0 <= x < width:
            grid[y][x] = sat["char"]

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
