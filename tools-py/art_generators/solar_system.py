import math
import time


def generate(width, height):
    """Generates an enhanced 2D representation of the solar system with orbital paths."""
    grid = [[" " for _ in range(width)] for _ in range(height)]

    sun_x, sun_y = width // 2, height // 2

    # Enhanced planets with all 8 planets plus Pluto
    planets = [
        {"name": "Mercury", "char": "☿", "distance": 6, "speed": 0.04, "size": 1},
        {"name": "Venus", "char": "♀", "distance": 9, "speed": 0.025, "size": 1},
        {"name": "Earth", "char": "⊕", "distance": 12, "speed": 0.02, "size": 1},
        {"name": "Mars", "char": "♂", "distance": 15, "speed": 0.015, "size": 1},
        {"name": "Jupiter", "char": "♃", "distance": 20, "speed": 0.008, "size": 2},
        {"name": "Saturn", "char": "♄", "distance": 25, "speed": 0.006, "size": 2},
        {"name": "Uranus", "char": "⛢", "distance": 29, "speed": 0.004, "size": 1},
        {"name": "Neptune", "char": "♆", "distance": 32, "speed": 0.003, "size": 1},
    ]

    # Draw orbital paths first (so planets appear on top)
    for planet in planets:
        for angle in range(0, 360, 5):
            rad = math.radians(angle)
            x = sun_x + int(planet["distance"] * math.cos(rad) * 2.5)
            y = sun_y + int(planet["distance"] * math.sin(rad))
            if 0 <= y < height and 0 <= x < width:
                grid[y][x] = "·"

    # Draw asteroid belt between Mars and Jupiter
    for _ in range(100):
        angle = math.radians((_ * 3.6 + time.time() * 5) % 360)
        distance = 17 + (_ % 3) * 0.5
        x = sun_x + int(distance * math.cos(angle) * 2.5)
        y = sun_y + int(distance * math.sin(angle))
        if 0 <= y < height and 0 <= x < width:
            grid[y][x] = "."

    # Draw the Sun with corona
    for dy in range(-2, 3):
        for dx in range(-5, 6):
            dist = math.sqrt((dx / 2.5) ** 2 + dy**2)
            if dist <= 2:
                y, x = sun_y + dy, sun_x + dx
                if 0 <= y < height and 0 <= x < width:
                    if dist < 1:
                        grid[y][x] = "@"
                    elif dist < 1.5:
                        grid[y][x] = "O"
                    else:
                        grid[y][x] = "o"

    # Draw planets with motion trails
    for planet in planets:
        angle = time.time() * planet["speed"]
        x = sun_x + int(planet["distance"] * math.cos(angle) * 2.5)
        y = sun_y + int(planet["distance"] * math.sin(angle))

        # Draw planet
        if 0 <= y < height and 0 <= x < width:
            grid[y][x] = planet["char"]

        # Draw trail (last few positions)
        for trail in range(1, 4):
            trail_angle = angle - (trail * 0.1)
            tx = sun_x + int(planet["distance"] * math.cos(trail_angle) * 2.5)
            ty = sun_y + int(planet["distance"] * math.sin(trail_angle))
            if 0 <= ty < height and 0 <= tx < width and grid[ty][tx] == "·":
                grid[ty][tx] = ":" if trail == 1 else "."

    return "\n".join(["".join(row) for row in grid])
