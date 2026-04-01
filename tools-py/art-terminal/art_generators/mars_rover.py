import random
import math


def simple_noise(x, seed=0):
    """Simple 1D pseudo-random noise function."""
    n = int(x) * 374761393 + seed * 668265263
    n = (n ^ (n >> 13)) * 1274126177
    return ((n ^ (n >> 16)) & 0x7FFFFFFF) / 0x7FFFFFFF


def smooth_noise_1d(x, scale=10.0, seed=0):
    """Generate smooth 1D noise using interpolation."""
    x_scaled = x / scale
    x0 = int(x_scaled)
    x1 = x0 + 1

    # Fractional part
    fx = x_scaled - x0

    # Smooth interpolation
    sx = (1 - math.cos(fx * math.pi)) * 0.5

    # Get noise values
    n0 = simple_noise(x0, seed)
    n1 = simple_noise(x1, seed)

    # Interpolate
    return n0 * (1 - sx) + n1 * sx


def generate(width, height):
    """Generates a Mars rover traversing procedurally generated Martian terrain.

    Shows autonomous rover with realistic terrain features including hills,
    craters, rocks, and atmospheric dust.
    """
    # Initialize state
    if not hasattr(generate, "distance"):
        generate.distance = 0
        generate.dust_particles = []

    grid = [[" " for _ in range(width)] for _ in range(height)]

    # Rover stays in left third of screen
    rover_x = width // 4
    rover_y = height - 8

    # Generate terrain
    terrain_y = height - 5  # Base terrain level

    # Draw terrain across width
    terrain_heights = []
    for x in range(width):
        # Multi-octave noise for varied terrain
        h = 0
        scroll_pos = generate.distance + x

        # Large hills
        h += smooth_noise_1d(scroll_pos, 50.0, 100) * 8
        # Medium features
        h += smooth_noise_1d(scroll_pos, 20.0, 200) * 4
        # Small details
        h += smooth_noise_1d(scroll_pos, 8.0, 300) * 2

        terrain_height = int(terrain_y - h)
        terrain_heights.append(terrain_height)

        # Draw terrain surface and below
        for y in range(terrain_height, height):
            if y >= height:
                break
            if y == terrain_height:
                # Surface characters
                grid[y][x] = random.choice(["~", "^", "·", "-"])
            elif y == terrain_height + 1:
                grid[y][x] = random.choice(["▓", "░", "▒"])
            else:
                grid[y][x] = random.choice(["▓", "█"])

    # Add craters
    for crater_center in range(0, width + 100, 40):
        crater_x = (crater_center - int(generate.distance)) % width
        if 0 <= crater_x < width:
            crater_width = 12
            crater_depth = 3

            for dx in range(-crater_width, crater_width + 1):
                x = crater_x + dx
                if 0 <= x < width:
                    # Crater profile (parabolic)
                    dist_norm = abs(dx) / crater_width
                    if dist_norm <= 1:
                        depth = int(crater_depth * (1 - dist_norm**2))
                        crater_y = terrain_heights[x] + depth

                        if crater_y < height:
                            # Rim is slightly raised
                            if 0.8 < dist_norm <= 1.0:
                                for dy in range(2):
                                    ry = crater_y - dy - 1
                                    if 0 <= ry < height and ry >= terrain_heights[x]:
                                        grid[ry][x] = "^"
                            else:
                                # Crater floor
                                for dy in range(depth + 1):
                                    cy = terrain_heights[x] + dy
                                    if cy < height:
                                        grid[cy][x] = "·"

    # Add rocks
    for rock_pos in range(0, int(generate.distance) + width, 15):
        rock_x = rock_pos - int(generate.distance)
        if 0 <= rock_x < width:
            rock_hash = simple_noise(rock_pos, 42)
            if rock_hash > 0.5:  # 50% chance
                rock_y = terrain_heights[rock_x] - 1
                if 0 <= rock_y < height - 1:
                    rock_size = int(rock_hash * 3) + 1
                    rock_char = random.choice(["⬬", "◆", "▲", "■"])
                    if grid[rock_y][rock_x] in [" ", "~", "^", "·", "-"]:
                        grid[rock_y][rock_x] = rock_char
                        # Larger rocks
                        if rock_size > 1 and rock_x + 1 < width:
                            if grid[rock_y][rock_x + 1] in [" ", "~", "^", "·", "-"]:
                                grid[rock_y][rock_x + 1] = rock_char

    # Draw rover (multi-line ASCII art)
    rover_art = ["  [CAMERA]  ", "    |_|     ", " ==={█}===  ", "  O  O  O   "]

    for i, line in enumerate(rover_art):
        y = rover_y + i
        if 0 <= y < height:
            for j, char in enumerate(line):
                x = rover_x + j - len(line) // 2
                if 0 <= x < width and char != " ":
                    grid[y][x] = char

    # Add occasional dust particles
    if random.random() < 0.15:
        generate.dust_particles.append(
            {
                "x": random.randint(0, width - 1),
                "y": random.randint(0, height - 6),
                "vx": random.uniform(-0.5, -0.2),
                "vy": random.uniform(-0.1, 0.1),
                "age": 0,
            }
        )

    # Update and draw dust
    particles_to_remove = []
    for dust in generate.dust_particles:
        dust["x"] += dust["vx"]
        dust["y"] += dust["vy"]
        dust["age"] += 1

        if dust["age"] > 30 or dust["x"] < 0 or dust["x"] >= width:
            particles_to_remove.append(dust)
        else:
            dx, dy = int(dust["x"]), int(dust["y"])
            if 0 <= dy < height - 5 and 0 <= dx < width:
                if grid[dy][dx] == " ":
                    grid[dy][dx] = random.choice(["·", "′", "."])

    for dust in particles_to_remove:
        generate.dust_particles.remove(dust)

    # Draw HUD
    generate.distance += 1.5

    # Distance display
    dist_text = f"DISTANCE: {int(generate.distance)}m"
    for i, char in enumerate(dist_text):
        if i < width:
            grid[1][i + 2] = char

    # Altitude display (based on current terrain)
    if rover_x < len(terrain_heights):
        altitude = height - terrain_heights[rover_x]
        alt_text = f"ALT: {altitude}m"
        for i, char in enumerate(alt_text):
            if i < width:
                grid[2][i + 2] = char

    # Mission time
    mission_time = int(generate.distance / 10)
    time_text = f"SOL: {mission_time // 60:02d}:{mission_time % 60:02d}"
    for i, char in enumerate(time_text):
        x = width - len(time_text) - 2 + i
        if 0 <= x < width:
            grid[1][x] = char

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
