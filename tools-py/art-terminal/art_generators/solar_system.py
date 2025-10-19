import math
import time
import random


def generate(width, height):
    """Generates an enhanced 2D representation of the solar system with orbital paths."""
    grid = [[" " for _ in range(width)] for _ in range(height)]

    sun_x, sun_y = width // 2, height // 2

    # Enhanced planets with all 8 planets plus moons
    planets = [
        {
            "name": "Mercury",
            "char": "\033[93m☿\033[0m",
            "distance": 6,
            "speed": 0.04,
            "size": 1,
            "moons": [],
        },
        {
            "name": "Venus",
            "char": "\033[92m♀\033[0m",
            "distance": 9,
            "speed": 0.025,
            "size": 1,
            "moons": [],
        },
        {
            "name": "Earth",
            "char": "\033[94m⊕\033[0m",
            "distance": 12,
            "speed": 0.02,
            "size": 1,
            "moons": [
                {
                    "name": "Moon",
                    "char": "\033[97m●\033[0m",
                    "distance": 2,
                    "speed": 0.5,
                }
            ],
        },
        {
            "name": "Mars",
            "char": "\033[91m♂\033[0m",
            "distance": 15,
            "speed": 0.015,
            "size": 1,
            "moons": [],
        },
        {
            "name": "Jupiter",
            "char": "\033[93m♃\033[0m",
            "distance": 20,
            "speed": 0.008,
            "size": 3,
            "moons": [
                {"name": "Io", "char": "\033[97m◦\033[0m", "distance": 3, "speed": 0.6},
                {
                    "name": "Europa",
                    "char": "\033[97m◦\033[0m",
                    "distance": 4,
                    "speed": 0.4,
                },
                {
                    "name": "Ganymede",
                    "char": "\033[97m○\033[0m",
                    "distance": 5,
                    "speed": 0.3,
                },
                {
                    "name": "Callisto",
                    "char": "\033[97m○\033[0m",
                    "distance": 6,
                    "speed": 0.2,
                },
            ],
        },
        {
            "name": "Saturn",
            "char": "\033[93m♄\033[0m",
            "distance": 25,
            "speed": 0.006,
            "size": 2,
            "moons": [
                {
                    "name": "Titan",
                    "char": "\033[97m○\033[0m",
                    "distance": 4,
                    "speed": 0.3,
                }
            ],
            "rings": True,
        },
        {
            "name": "Uranus",
            "char": "\033[96m⛢\033[0m",
            "distance": 29,
            "speed": 0.004,
            "size": 2,
            "moons": [],
        },
        {
            "name": "Neptune",
            "char": "\033[94m♆\033[0m",
            "distance": 32,
            "speed": 0.003,
            "size": 1,
            "moons": [],
        },
    ]

    # Draw orbital paths first (so planets appear on top)
    for planet in planets:
        for angle in range(0, 360, 5):
            rad = math.radians(angle)
            x = sun_x + int(planet["distance"] * math.cos(rad) * 2.5)
            y = sun_y + int(planet["distance"] * math.sin(rad))
            if 0 <= y < height and 0 <= x < width:
                grid[y][x] = "\033[90m·\033[0m"

    # Draw asteroid belt between Mars and Jupiter
    for _ in range(50):  # Reduced for performance
        angle = math.radians((_ * 3.6 + time.time() * 5) % 360)
        distance = 17 + (_ % 3) * 0.5
        x = sun_x + int(distance * math.cos(angle) * 2.5)
        y = sun_y + int(distance * math.sin(angle))
        if 0 <= y < height and 0 <= x < width:
            grid[y][x] = "\033[90m.\033[0m"

    # Draw the Sun with pulsating corona
    t = time.time()
    for dy in range(-2, 3):
        for dx in range(-5, 6):
            dist = math.sqrt((dx / 2.5) ** 2 + dy**2)
            if dist <= 2.5:
                y, x = sun_y + dy, sun_x + dx
                if 0 <= y < height and 0 <= x < width:
                    if dist < 0.8:
                        grid[y][x] = "\033[93m█\033[0m"
                    elif dist < 1.5:
                        grid[y][x] = "\033[93m▓\033[0m"
                    elif dist < 2:
                        grid[y][x] = "\033[93m▒\033[0m"
                    else:
                        grid[y][x] = "\033[93m░\033[0m"

    # Solar prominences (occasional)
    if math.sin(t * 0.5) > 0.3:
        for prom_y in [sun_y - 3, sun_y + 3]:
            for prom_x in [sun_x - 2, sun_x, sun_x + 2]:
                if 0 <= prom_y < height and 0 <= prom_x < width:
                    if grid[prom_y][prom_x] == " ":
                        grid[prom_y][prom_x] = "~"

    # Draw planets with motion trails, moons, and rings
    for planet in planets:
        angle = time.time() * planet["speed"]
        x = sun_x + int(planet["distance"] * math.cos(angle) * 2.5)
        y = sun_y + int(planet["distance"] * math.sin(angle))

        # Draw Saturn's rings
        if planet.get("rings") and 0 <= y < height:
            ring_width = 6
            ring_height = 2
            for ry in range(-ring_height, ring_height + 1):
                for rx in range(-ring_width, ring_width + 1):
                    ring_dist = math.sqrt((rx / 2.5) ** 2 + (ry) ** 2)
                    if 2.5 < ring_dist < 3.5:  # Ring band
                        ring_y, ring_x = y + ry, x + rx
                        if 0 <= ring_y < height and 0 <= ring_x < width:
                            grid[ring_y][ring_x] = (
                                "\033[93m≡\033[0m"
                                if abs(ry) < 1
                                else "\033[93m=\033[0m"
                            )

        # Draw planet
        if 0 <= y < height and 0 <= x < width:
            grid[y][x] = planet["char"]

            # Larger planets get extended representation
            if planet["size"] >= 2:
                for dy, dx in [(-1, 0), (1, 0), (0, -2), (0, 2)]:
                    py, px = y + dy, x + dx
                    if 0 <= py < height and 0 <= px < width:
                        if grid[py][px] in [" ", "·", ":", "."]:
                            grid[py][px] = "◎" if planet["size"] >= 3 else "○"

        # Draw moons
        for moon in planet.get("moons", []):
            moon_angle = time.time() * moon["speed"] + angle  # Orbit around planet
            mx = x + int(moon["distance"] * math.cos(moon_angle) * 2.5)
            my = y + int(moon["distance"] * math.sin(moon_angle))
            if 0 <= my < height and 0 <= mx < width:
                grid[my][mx] = moon["char"]

        # Draw trail (last few positions)
        for trail in range(1, 4):
            trail_angle = angle - (trail * 0.1)
            tx = sun_x + int(planet["distance"] * math.cos(trail_angle) * 2.5)
            ty = sun_y + int(planet["distance"] * math.sin(trail_angle))
            if 0 <= ty < height and 0 <= tx < width:
                grid[ty][tx] = "\033[90m: \033[0m" if trail == 1 else "\033[90m.\033[0m"

    # Draw a comet with gradient tail
    comet_angle = time.time() * 0.003
    comet_distance = 35 + 15 * math.sin(time.time() * 0.002)  # Elliptical orbit
    comet_x = sun_x + int(comet_distance * math.cos(comet_angle) * 2.5)
    comet_y = sun_y + int(comet_distance * math.sin(comet_angle))

    # Draw comet nucleus
    if 0 <= comet_y < height and 0 <= comet_x < width:
        grid[comet_y][comet_x] = "\033[97m☄\033[0m"

    # Draw tail pointing away from sun
    tail_length = max(
        8, int(30 / (comet_distance + 1))
    )  # Longer tail when closer to sun
    for i in range(1, tail_length):
        dx = comet_x - sun_x
        dy = comet_y - sun_y
        dist = math.sqrt(dx**2 + dy**2) + 0.01
        tail_x = comet_x + int((dx / dist) * i * 0.8)
        tail_y = comet_y + int((dy / dist) * i * 0.3)

        if 0 <= tail_y < height and 0 <= tail_x < width:
            if i < tail_length // 3:
                grid[tail_y][tail_x] = "\033[97m*\033[0m"
            elif i < 2 * tail_length // 3:
                grid[tail_y][tail_x] = "\033[90m·\033[0m"
            else:
                grid[tail_y][tail_x] = "\033[90m.\033[0m"

    # Add sparse background stars
    for _ in range(int(width * height * 0.002)):
        star_x = random.randint(0, width - 1)
        star_y = random.randint(0, height - 1)
        if grid[star_y][star_x] == " ":
            grid[star_y][star_x] = "\033[90m.\033[0m"

    # Info panel
    info = "Solar System | \033[93m█=Sun\033[0m ☿♀⊕♂♃♄⛢♆=Planets \033[97m●○=Moons\033[0m \033[93m≡=Saturn's Rings\033[0m ☄=Comet"
    info_y = height - 1
    if 0 <= info_y < height:
        for i, char in enumerate(info):
            if i < width:
                grid[info_y][i] = char

    return "\n".join(["".join(row) for row in grid])
