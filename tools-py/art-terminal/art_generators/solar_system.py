import math
import time
import random


def generate(width, height):
    """Generates solar system with size-coded circles for planets."""
    grid = [[" " for _ in range(width)] for _ in range(height)]

    # Add title
    title = "SOLAR SYSTEM"
    subtitle = "Orbital mechanics of 8 planets"
    title_x = (width - len(title)) // 2
    subtitle_x = (width - len(subtitle)) // 2

    if height > 2:
        for i, char in enumerate(title):
            if 0 <= title_x + i < width:
                grid[0][title_x + i] = char
        for i, char in enumerate(subtitle):
            if 0 <= subtitle_x + i < width:
                grid[1][subtitle_x + i] = char

    sun_x, sun_y = width // 2, height // 2

    # Planets with size-based character representation
    planets = [
        {"distance": 6, "speed": 0.04, "char": "·"},  # Mercury (small)
        {"distance": 9, "speed": 0.025, "char": "·"},  # Venus (small)
        {"distance": 12, "speed": 0.02, "char": "○", "has_moon": True},  # Earth
        {"distance": 15, "speed": 0.015, "char": "·"},  # Mars (small)
        {"distance": 20, "speed": 0.008, "char": "◎", "has_moons": 4},  # Jupiter (large)
        {"distance": 25, "speed": 0.006, "char": "⊙", "has_rings": True, "has_moon": True},  # Saturn
        {"distance": 29, "speed": 0.004, "char": "○"},  # Uranus
        {"distance": 32, "speed": 0.003, "char": "·"},  # Neptune
    ]

    # Draw sparse orbital paths (only key points)
    for planet in planets:
        for angle in range(0, 360, 30):  # Reduced from every 5 degrees
            rad = math.radians(angle)
            x = sun_x + int(planet["distance"] * math.cos(rad) * 2.5)
            y = sun_y + int(planet["distance"] * math.sin(rad))
            if 0 <= y < height and 0 <= x < width:
                if grid[y][x] == " ":
                    grid[y][x] = "┄"

    # Draw asteroid belt (reduced density)
    for _ in range(25):  # Reduced from 50
        angle = math.radians((_ * 7.2 + time.time() * 5) % 360)
        distance = 17 + (_ % 3) * 0.5
        x = sun_x + int(distance * math.cos(angle) * 2.5)
        y = sun_y + int(distance * math.sin(angle))
        if 0 <= y < height and 0 <= x < width:
            if grid[y][x] == " ":
                grid[y][x] = "∘"

    # Draw the Sun
    t = time.time()
    for dy in range(-2, 3):
        for dx in range(-5, 6):
            dist = math.sqrt((dx / 2.5) ** 2 + dy**2)
            if dist <= 2.5:
                y, x = sun_y + dy, sun_x + dx
                if 0 <= y < height and 0 <= x < width:
                    if dist < 0.8:
                        grid[y][x] = "⊙"
                    elif dist < 1.5:
                        grid[y][x] = "●"
                    elif dist < 2:
                        grid[y][x] = "◉"

    # Solar prominences (occasional)
    if math.sin(t * 0.5) > 0.3:
        for prom_y in [sun_y - 3, sun_y + 3]:
            for prom_x in [sun_x - 2, sun_x, sun_x + 2]:
                if 0 <= prom_y < height and 0 <= prom_x < width:
                    if grid[prom_y][prom_x] == " ":
                        grid[prom_y][prom_x] = "~"

    # Draw planets
    for planet in planets:
        angle = time.time() * planet["speed"]
        x = sun_x + int(planet["distance"] * math.cos(angle) * 2.5)
        y = sun_y + int(planet["distance"] * math.sin(angle))

        # Draw Saturn's rings
        if planet.get("has_rings") and 0 <= y < height:
            ring_width = 6
            ring_height = 2
            for ry in range(-ring_height, ring_height + 1):
                for rx in range(-ring_width, ring_width + 1):
                    ring_dist = math.sqrt((rx / 2.5) ** 2 + (ry) ** 2)
                    if 2.5 < ring_dist < 3.5:
                        ring_y, ring_x = y + ry, x + rx
                        if 0 <= ring_y < height and 0 <= ring_x < width:
                            if grid[ring_y][ring_x] in [" ", "┄"]:
                                grid[ring_y][ring_x] = "≈"

        # Draw planet
        if 0 <= y < height and 0 <= x < width:
            grid[y][x] = planet["char"]

        # Draw Earth's moon
        if planet.get("has_moon"):
            moon_angle = time.time() * 0.5 + angle
            mx = x + int(2 * math.cos(moon_angle) * 2.5)
            my = y + int(2 * math.sin(moon_angle))
            if 0 <= my < height and 0 <= mx < width:
                grid[my][mx] = "∘"

        # Draw Jupiter's moons
        if planet.get("has_moons"):
            for moon_idx in range(planet["has_moons"]):
                moon_angle = time.time() * (0.6 - moon_idx * 0.1) + angle
                moon_dist = 3 + moon_idx
                mx = x + int(moon_dist * math.cos(moon_angle) * 2.5)
                my = y + int(moon_dist * math.sin(moon_angle))
                if 0 <= my < height and 0 <= mx < width:
                    if grid[my][mx] in [" ", "┄"]:
                        grid[my][mx] = "∘"

    # Draw a comet with gradient tail
    comet_angle = time.time() * 0.003
    comet_distance = 35 + 15 * math.sin(time.time() * 0.002)
    comet_x = sun_x + int(comet_distance * math.cos(comet_angle) * 2.5)
    comet_y = sun_y + int(comet_distance * math.sin(comet_angle))

    # Draw comet nucleus
    if 0 <= comet_y < height and 0 <= comet_x < width:
        grid[comet_y][comet_x] = "◆"

    # Draw tail pointing away from sun
    tail_length = max(8, int(30 / (comet_distance + 1)))
    for i in range(1, tail_length):
        dx = comet_x - sun_x
        dy = comet_y - sun_y
        dist = math.sqrt(dx**2 + dy**2) + 0.01
        tail_x = comet_x + int((dx / dist) * i * 0.8)
        tail_y = comet_y + int((dy / dist) * i * 0.3)

        if 0 <= tail_y < height and 0 <= tail_x < width:
            if grid[tail_y][tail_x] == " ":
                if i < tail_length // 3:
                    grid[tail_y][tail_x] = "∴"
                elif i < 2 * tail_length // 3:
                    grid[tail_y][tail_x] = "∘"
                else:
                    grid[tail_y][tail_x] = "·"

    # Add minimal background stars (reduced density drastically)
    for _ in range(int(width * height * 0.0003)):  # Reduced from 0.002
        star_x = random.randint(0, width - 1)
        star_y = random.randint(0, height - 1)
        if grid[star_y][star_x] == " ":
            grid[star_y][star_x] = "·"

    return "\n".join(["".join(row) for row in grid])
