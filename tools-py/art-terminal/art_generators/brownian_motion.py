import random
import math
import time


def generate(width, height):
    """Enhanced Brownian motion with temperature, particle interactions, and energy transfer.

    Shows realistic Brownian motion with:
    - One large Brownian particle buffeted by smaller molecules
    - Temperature-dependent molecular speeds
    - Elastic collisions with momentum transfer
    - Heat map visualization
    - Real-time statistics
    """
    # Initialize on first call
    if not hasattr(generate, "particles"):
        generate.particles = []

        # Create many small "solvent" molecules (lighter, faster)
        for _ in range(50):
            angle = random.uniform(0, 2 * math.pi)
            speed = random.gauss(1.5, 0.5)  # Maxwell-Boltzmann-ish distribution
            generate.particles.append(
                {
                    "x": random.uniform(5, width - 5),
                    "y": random.uniform(5, height - 5),
                    "vx": speed * math.cos(angle),
                    "vy": speed * math.sin(angle),
                    "trail": [],
                    "mass": 1,
                    "type": "molecule",
                    "char": random.choice(["·", "∙", "•"]),
                }
            )

        # Create one large Brownian particle (heavier, slower)
        generate.particles.append(
            {
                "x": width / 2,
                "y": height / 2,
                "vx": 0.0,
                "vy": 0.0,
                "trail": [],
                "mass": 25,  # Much heavier
                "type": "brownian",
                "char": "◉",
            }
        )

        # Heat map for temperature visualization
        generate.heat_map = [
            [0.0 for _ in range(width // 4)] for _ in range(height // 4)
        ]

    grid = [[" " for _ in range(width)] for _ in range(height)]

    # Decay heat map slightly
    for y in range(len(generate.heat_map)):
        for x in range(len(generate.heat_map[0])):
            generate.heat_map[y][x] *= 0.95

    # Update particles with realistic physics
    for particle in generate.particles:
        # Add thermal fluctuation for molecules (Brownian kicks)
        if particle["type"] == "molecule":
            particle["vx"] += random.gauss(0, 0.1)
            particle["vy"] += random.gauss(0, 0.1)

        # Update position
        particle["x"] += particle["vx"]
        particle["y"] += particle["vy"]

        # Elastic boundary collisions
        if particle["x"] <= 1 or particle["x"] >= width - 2:
            particle["vx"] = -particle["vx"]
            particle["x"] = max(1, min(width - 2, particle["x"]))
        if particle["y"] <= 1 or particle["y"] >= height - 2:
            particle["vy"] = -particle["vy"]
            particle["y"] = max(1, min(height - 2, particle["y"]))

        # Update heat map based on particle speed (kinetic energy)
        hx = int(particle["x"] / 4)
        hy = int(particle["y"] / 4)
        if 0 <= hy < len(generate.heat_map) and 0 <= hx < len(generate.heat_map[0]):
            speed_sq = particle["vx"] ** 2 + particle["vy"] ** 2
            generate.heat_map[hy][hx] += speed_sq * 0.5

    # Handle particle-particle collisions (elastic)
    for i, p1 in enumerate(generate.particles):
        for p2 in generate.particles[i + 1 :]:
            dx = p2["x"] - p1["x"]
            dy = p2["y"] - p1["y"]
            dist = math.sqrt(dx**2 + dy**2)

            collision_dist = (
                1.5 if (p1["type"] == "brownian" or p2["type"] == "brownian") else 0.8
            )

            if dist < collision_dist and dist > 0:
                # Normalize collision vector
                nx, ny = dx / dist, dy / dist

                # Relative velocity
                dvx = p1["vx"] - p2["vx"]
                dvy = p1["vy"] - p2["vy"]

                # Relative velocity in collision normal direction
                dvn = dvx * nx + dvy * ny

                # Don't process if particles are separating
                if dvn > 0:
                    # Elastic collision with mass
                    m1, m2 = p1["mass"], p2["mass"]
                    impulse = 2 * dvn / (m1 + m2)

                    # Update velocities
                    p1["vx"] -= impulse * m2 * nx
                    p1["vy"] -= impulse * m2 * ny
                    p2["vx"] += impulse * m1 * nx
                    p2["vy"] += impulse * m1 * ny

                    # Separate overlapping particles
                    overlap = collision_dist - dist
                    p1["x"] -= overlap * nx * 0.5
                    p1["y"] -= overlap * ny * 0.5
                    p2["x"] += overlap * nx * 0.5
                    p2["y"] += overlap * ny * 0.5

        # Add to trail (only for Brownian particle)
        if particle["type"] == "brownian":
            particle["trail"].append((int(particle["x"]), int(particle["y"])))
            if len(particle["trail"]) > 150:
                particle["trail"].pop(0)

    # Draw heat map as background
    heat_chars = [" ", "░", "▒", "▓"]
    for y in range(len(generate.heat_map)):
        for x in range(len(generate.heat_map[0])):
            heat = generate.heat_map[y][x]
            if heat > 0.5:
                char_idx = min(int(heat / 3), len(heat_chars) - 1)
                # Draw 4x4 block for each heat map cell
                for dy in range(4):
                    for dx in range(4):
                        gy, gx = y * 4 + dy, x * 4 + dx
                        if 0 <= gy < height and 0 <= gx < width:
                            if grid[gy][gx] == " ":
                                grid[gy][gx] = heat_chars[char_idx]

    # Draw Brownian particle trail
    brownian = [p for p in generate.particles if p["type"] == "brownian"][0]
    trail_chars = ["·", ":", "*"]
    trail_len = len(brownian["trail"])
    for idx, (x, y) in enumerate(brownian["trail"]):
        if 0 <= y < height and 0 <= x < width:
            char_idx = min(
                int((idx / trail_len) * len(trail_chars)), len(trail_chars) - 1
            )
            if grid[y][x] in [" ", "░", "▒"]:
                grid[y][x] = trail_chars[char_idx]

    # Draw particles
    for particle in generate.particles:
        px, py = int(particle["x"]), int(particle["y"])
        if 0 <= py < height and 0 <= px < width:
            grid[py][px] = particle["char"]

            # Brownian particle gets visual emphasis
            if particle["type"] == "brownian":
                for dy, dx in [(-1, 0), (1, 0), (0, -1), (0, 1)]:
                    ny, nx = py + dy, px + dx
                    if 0 <= ny < height and 0 <= nx < width:
                        if grid[ny][nx] in [" ", "░", "▒", "·", ":"]:
                            grid[ny][nx] = "○"

    # Calculate and display statistics
    avg_speed = sum(
        math.sqrt(p["vx"] ** 2 + p["vy"] ** 2)
        for p in generate.particles
        if p["type"] == "molecule"
    ) / sum(1 for p in generate.particles if p["type"] == "molecule")
    brownian_speed = math.sqrt(brownian["vx"] ** 2 + brownian["vy"] ** 2)

    # Temperature proportional to average kinetic energy
    temperature = avg_speed**2

    # Info panel
    info_lines = [
        "Brownian Motion Simulation",
        f"Molecules: {sum(1 for p in generate.particles if p['type'] == 'molecule')} | Avg Speed: {avg_speed:.2f}",
        f"Large Particle Speed: {brownian_speed:.2f} | T: {temperature:.1f}K",
        "░▒▓ = Heat zones | ◉ = Brownian particle",
    ]

    # Draw info panel at top
    for idx, line in enumerate(info_lines):
        if idx < height:
            for i, char in enumerate(line):
                if i < width:
                    grid[idx][i] = char

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
