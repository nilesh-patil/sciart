import math
import time


def draw_line(grid, x1, y1, x2, y2, char="-"):
    """Draw a line between two points."""
    dx = abs(x2 - x1)
    dy = abs(y2 - y1)
    sx = 1 if x1 < x2 else -1
    sy = 1 if y1 < y2 else -1
    err = dx - dy

    while True:
        if 0 <= y1 < len(grid) and 0 <= x1 < len(grid[0]):
            if grid[y1][x1] == " ":
                grid[y1][x1] = char

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
    """Generates a spacetime grid warped by a massive object.

    Visualizes general relativity by showing how mass curves the
    fabric of spacetime, represented as a 2D grid. Includes
    gravitational wave oscillations.
    """
    grid = [[" " for _ in range(width)] for _ in range(height)]

    # Gravitational wave oscillation
    t = time.time()
    wave_amplitude = 2 * math.sin(t * 0.5)  # Slow oscillating wave

    # Massive object at center
    mass_x = width // 2
    mass_y = height // 2
    mass_radius = 4

    # Warp strength (gravitational constant analogue) - oscillates with wave
    G = 100.0 + wave_amplitude * 20

    # Grid parameters
    grid_spacing_x = 8
    grid_spacing_y = 4
    grid_points_x = width // grid_spacing_x + 2
    grid_points_y = height // grid_spacing_y + 2

    # Create warped grid
    warped_points = []

    for gy in range(grid_points_y):
        row_points = []
        for gx in range(grid_points_x):
            # Unwarped position
            base_x = gx * grid_spacing_x - grid_spacing_x
            base_y = gy * grid_spacing_y - grid_spacing_y

            # Distance to massive object
            dx = base_x - mass_x
            dy = base_y - mass_y
            distance = math.sqrt(dx**2 + dy**2)

            # Apply gravitational warping (pull toward mass)
            if distance > 0:
                # Warp decreases with distance
                warp_factor = G / (distance**2 + 50)

                # Normalize direction vector
                dir_x = -dx / distance
                dir_y = -dy / distance

                # Apply warp
                warped_x = int(base_x + dir_x * warp_factor)
                warped_y = int(base_y + dir_y * warp_factor)
            else:
                warped_x = base_x
                warped_y = base_y

            row_points.append((warped_x, warped_y))

        warped_points.append(row_points)

    # Draw horizontal grid lines
    for gy in range(len(warped_points)):
        for gx in range(len(warped_points[gy]) - 1):
            x1, y1 = warped_points[gy][gx]
            x2, y2 = warped_points[gy][gx + 1]
            draw_line(grid, x1, y1, x2, y2, "─")

    # Draw vertical grid lines
    for gx in range(len(warped_points[0])):
        for gy in range(len(warped_points) - 1):
            x1, y1 = warped_points[gy][gx]
            x2, y2 = warped_points[gy + 1][gx]
            draw_line(grid, x1, y1, x2, y2, "│")

    # Draw grid intersections
    for gy in range(len(warped_points)):
        for gx in range(len(warped_points[gy])):
            gx_pos, gy_pos = warped_points[gy][gx]
            if 0 <= gy_pos < height and 0 <= gx_pos < width:
                grid[gy_pos][gx_pos] = "+"

    # Draw the massive object creating the warp
    for dy in range(-mass_radius, mass_radius + 1):
        for dx in range(-mass_radius * 2, mass_radius * 2 + 1):
            dist = math.sqrt((dx / 2.0) ** 2 + dy**2)
            if dist <= mass_radius:
                y, x = mass_y + dy, mass_x + dx
                if 0 <= y < height and 0 <= x < width:
                    if dist < mass_radius * 0.5:
                        grid[y][x] = "@"
                    elif dist < mass_radius * 0.75:
                        grid[y][x] = "#"
                    else:
                        grid[y][x] = "*"

    # Optional: Add a test particle following geodesic
    if not hasattr(generate, "particle"):
        # Initialize particle in orbit
        generate.particle = {
            "x": mass_x + 20,
            "y": mass_y,
            "vx": 0,
            "vy": 2.0,
            "trail": [],
        }

    # Update particle (simplified geodesic)
    particle = generate.particle
    px, py = particle["x"], particle["y"]

    # Gravitational acceleration
    dx = mass_x - px
    dy = mass_y - py
    distance = math.sqrt(dx**2 + dy**2)

    if distance > mass_radius + 1:
        # Simple Newtonian approximation for visualization
        accel = 50.0 / (distance**2)
        particle["vx"] += (dx / distance) * accel * 0.1
        particle["vy"] += (dy / distance) * accel * 0.1

        # Update position
        particle["x"] += particle["vx"] * 0.1
        particle["y"] += particle["vy"] * 0.1

        # Add to trail
        particle["trail"].append((int(px), int(py)))
        if len(particle["trail"]) > 50:
            particle["trail"].pop(0)

        # Draw trail
        for tx, ty in particle["trail"]:
            if 0 <= ty < height and 0 <= tx < width:
                if grid[ty][tx] in [" ", "─", "│"]:
                    grid[ty][tx] = "·"

        # Draw particle
        ppx, ppy = int(particle["x"]), int(particle["y"])
        if 0 <= ppy < height and 0 <= ppx < width:
            grid[ppy][ppx] = "○"
    else:
        # Reset if particle falls into mass
        generate.particle = {
            "x": mass_x + 20,
            "y": mass_y,
            "vx": 0,
            "vy": 2.0,
            "trail": [],
        }

    # Labels
    title = "SPACETIME CURVATURE"
    for i, c in enumerate(title):
        if i + 2 < width and 1 < height:
            grid[1][i + 2] = c

    info = "Mass warps spacetime"
    for i, c in enumerate(info):
        if i + 2 < width and 2 < height:
            grid[2][i + 2] = c

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
