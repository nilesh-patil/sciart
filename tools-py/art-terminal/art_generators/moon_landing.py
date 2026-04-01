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

    Shows descending view of lunar surface with craters, HUD displays,
    and dust effects from engine exhaust.
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

    # Calculate descent parameters
    landing_duration = 30.0
    progress = min(elapsed / landing_duration, 1.0)

    # Altitude decreases exponentially
    altitude = 1000 * (1 - progress) ** 2  # meters
    velocity = -50 * (1 - progress)  # m/s (negative = downward)
    fuel = max(0, 100 - progress * 80)  # percentage

    # Surface zoom level (increases as we descend)
    zoom = 1 + progress * 15

    # Generate lunar surface
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
                            # Crater rim (bright)
                            crater_char = "^"
                        elif depth_factor > 0.6:
                            crater_char = "▓"
                        elif depth_factor > 0.3:
                            crater_char = "▒"
                        else:
                            crater_char = "░"
                        break

                if in_crater:
                    break

            # Base regolith texture
            if not in_crater:
                noise_val = simple_noise(world_x * 5, world_y * 5, 400)
                if noise_val > 0.7:
                    crater_char = "·"
                elif noise_val > 0.4:
                    crater_char = "."
                else:
                    crater_char = " "

            # Only show surface in lower portion of screen
            if i >= surface_detail_y - int(20 / zoom):
                grid[i][j] = crater_char

    # Engine thrust indicator and dust
    if progress < 1.0:
        thrust_level = min(fuel / 100, 1.0)

        # Draw engine exhaust cone
        exhaust_y = int(height * 0.4)
        exhaust_spread = int(15 * progress)  # Dust spreads as we get closer

        for i in range(exhaust_spread):
            y = exhaust_y + i
            if y < height:
                spread_width = int(i * 0.8)
                for dx in range(-spread_width, spread_width + 1):
                    x = width // 2 + dx
                    if 0 <= x < width and random.random() < thrust_level * 0.6:
                        if grid[y][x] == " ":
                            char = random.choice(["*", "·", ".", "+", "~"])
                            grid[y][x] = char

        # Dust particles kicked up
        if progress > 0.4 and random.random() < 0.3:
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

    # HUD - Box in upper left
    hud_lines = [
        "╔═══════════════╗",
        "║ LUNAR MODULE  ║",
        "╠═══════════════╣",
        f"║ ALT: {altitude:7.1f}m ║",
        f"║ VEL: {velocity:6.1f}m/s║",
        f"║ FUEL: {fuel:5.1f}%  ║",
        "╠═══════════════╣",
        f"║ THR: {'█' * int(fuel / 10):10s} ║",
        "╚═══════════════╝",
    ]

    for i, line in enumerate(hud_lines):
        if i < height:
            for j, char in enumerate(line):
                if j < width:
                    grid[i][j] = char

    # Landing status
    if progress >= 1.0:
        if not generate.landed:
            generate.landed = True

        status_lines = [
            "╔═══════════════════════╗",
            "║   CONTACT LIGHT       ║",
            "║   ENGINE STOP         ║",
            "║   TRANQUILITY BASE    ║",
            "║   THE EAGLE HAS       ║",
            "║   LANDED              ║",
            "╚═══════════════════════╝",
        ]

        start_y = height // 2 - len(status_lines) // 2
        start_x = width // 2 - 12

        for i, line in enumerate(status_lines):
            y = start_y + i
            if 0 <= y < height:
                for j, char in enumerate(line):
                    x = start_x + j
                    if 0 <= x < width:
                        grid[y][x] = char

    # Crosshair in center
    if progress < 1.0:
        center_x, center_y = width // 2, height // 2

        # Horizontal crosshair
        for dx in range(-5, 6):
            x = center_x + dx
            if 0 <= x < width and abs(dx) > 2:
                grid[center_y][x] = "─"

        # Vertical crosshair
        for dy in range(-3, 4):
            y = center_y + dy
            if 0 <= y < height and abs(dy) > 1:
                grid[y][center_x] = "│"

        # Center dot
        grid[center_y][center_x] = "+"

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
