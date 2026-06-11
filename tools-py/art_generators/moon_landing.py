import random
import math
import time


def simple_noise(x, y, seed=0):
    """2D pseudo-random noise."""
    n = int(x) * 374761393 + int(y) * 668265263 + seed
    n = (n ^ (n >> 13)) * 1274126177
    return ((n ^ (n >> 16)) & 0x7FFFFFFF) / 0x7FFFFFFF


def generate(width, height):
    """Generates a first-person lunar landing sequence.

    Shows descending view of lunar surface with craters and dust effects from engine exhaust.
    """
    # Initialize landing sequence
    if not hasattr(generate, "start_time"):
        generate.start_time = time.time()
        generate.dust_particles = []
        generate.landed = False

    elapsed = time.time() - generate.start_time

    # Landing duration: 30 seconds
    if elapsed > 35:
        # Reset
        generate.start_time = time.time()
        generate.dust_particles = []
        generate.landed = False
        elapsed = 0

    grid = [[" " for _ in range(width)] for _ in range(height)]
    # Add title
    title = "LUNAR DESCENT"
    subtitle = "Powered descent to lunar surface"
    title_x = (width - len(title)) // 2
    subtitle_x = (width - len(subtitle)) // 2

    if height > 2:
        for i, char in enumerate(title):
            if 0 <= title_x + i < width:
                grid[0][title_x + i] = char
        for i, char in enumerate(subtitle):
            if 0 <= subtitle_x + i < width:
                grid[1][subtitle_x + i] = char

    # Calculate descent parameters
    landing_duration = 30.0
    progress = min(elapsed / landing_duration, 1.0)

    # Surface zoom level (increases as we descend)
    zoom = 1 + progress * 15

    # Generate lunar surface - much sparser
    surface_detail_y = int(height * 0.7)

    for i in range(height):
        for j in range(width):
            # Create crater field
            # Scale coordinates by zoom
            world_x = (j - width / 2) / zoom
            world_y = (i - surface_detail_y) / zoom

            # Multiple craters at different positions
            in_crater = False
            crater_char = " "

            for cx in range(-5, 6):
                for cy in range(-5, 6):
                    crater_center_x = cx * 40 + simple_noise(cx, cy, 100) * 20
                    crater_center_y = cy * 40 + simple_noise(cx, cy, 200) * 20

                    dx = world_x - crater_center_x
                    dy = world_y - crater_center_y
                    crater_dist = math.sqrt(dx**2 + dy**2)

                    crater_radius = 8 + simple_noise(cx, cy, 300) * 8

                    if crater_dist < crater_radius:
                        in_crater = True
                        # Crater depth shading
                        depth_factor = 1 - (crater_dist / crater_radius)

                        if crater_dist > crater_radius * 0.9:
                            # Crater rim
                            if dx > 0:
                                crater_char = "◠"
                            else:
                                crater_char = "◡"
                        elif depth_factor > 0.6:
                            crater_char = " "  # Less dense
                        elif depth_factor > 0.3:
                            crater_char = " "  # Less dense
                        else:
                            crater_char = " "  # Less dense
                        break

                if in_crater:
                    break

            # Base regolith texture - much sparser
            if not in_crater:
                noise_val = simple_noise(world_x * 5, world_y * 5, 400)
                if noise_val > 0.85:  # Much more selective
                    crater_char = "·"
                else:
                    crater_char = " "

            # Only show surface in lower portion of screen
            if i >= surface_detail_y - int(20 / zoom):
                grid[i][j] = crater_char

    # Draw terrain lines - using approved characters
    terrain_y = int(height * 0.7)
    for x in range(0, width, 5):
        if terrain_y < height and x < width:
            noise_y = int(simple_noise(x / zoom, 0, 500) * 3)
            y = terrain_y + noise_y
            if 0 <= y < height:
                grid[y][x] = random.choice(["~", "─", "≈"])

    # Engine thrust indicator and dust
    if progress < 1.0:
        # Draw dust kicked up in radial pattern
        exhaust_y = int(height * 0.4)
        exhaust_spread = int(15 * progress)  # Dust spreads as we get closer

        # Radial dust particles - sparse
        if exhaust_spread > 5:
            center_x = width // 2
            for angle in range(0, 360, 30):  # Every 30 degrees
                rad = math.radians(angle)
                for dist in range(5, exhaust_spread, 3):
                    x = center_x + int(dist * math.cos(rad) * 0.8)
                    y = exhaust_y + int(dist * math.sin(rad) * 0.5)
                    if 0 <= y < height and 0 <= x < width:
                        if random.random() < 0.3 and grid[y][x] == " ":
                            grid[y][x] = "∴"

        # Dust particles kicked up
        if progress > 0.4 and random.random() < 0.1:  # Reduced frequency
            generate.dust_particles.append(
                {
                    "x": width / 2 + random.uniform(-10, 10),
                    "y": exhaust_y + exhaust_spread,
                    "vx": random.uniform(-2, 2),
                    "vy": random.uniform(-0.5, -2),
                    "age": 0,
                }
            )

    # Update dust particles
    particles_to_remove = []
    for dust in generate.dust_particles:
        dust["x"] += dust["vx"]
        dust["y"] += dust["vy"]
        dust["vy"] += 0.05  # Gravity (very weak)
        dust["age"] += 1

        if (
            dust["age"] > 40
            or dust["y"] >= height
            or dust["x"] < 0
            or dust["x"] >= width
        ):
            particles_to_remove.append(dust)
        else:
            px, py = int(dust["x"]), int(dust["y"])
            if 0 <= py < height and 0 <= px < width:
                if grid[py][px] == " ":
                    grid[py][px] = "·"

    for dust in particles_to_remove:
        generate.dust_particles.remove(dust)

    # Landing shadow - grows as we descend
    if progress > 0.3:
        shadow_radius = int(8 * progress)
        shadow_x = width // 2
        shadow_y = int(height * 0.7)

        for dy in range(-shadow_radius, shadow_radius + 1):
            for dx in range(-int(shadow_radius * 2.5), int(shadow_radius * 2.5) + 1):
                dist = math.sqrt((dx / 2.5) ** 2 + dy**2)
                if dist <= shadow_radius:
                    y = shadow_y + dy
                    x = shadow_x + dx
                    if 0 <= y < height and 0 <= x < width:
                        if grid[y][x] == " " and random.random() < 0.3:
                            grid[y][x] = "▓"

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
