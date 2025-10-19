import math
import time
import random


def generate(width, height):
    """Generates a particle accelerator with colliding beams.

    Shows protons and antiprotons racing around a ring, accelerating,
    and colliding to produce particle showers.
    """
    # Initialize accelerator state
    if not hasattr(generate, "start_time"):
        generate.start_time = time.time()
        generate.particle1_angle = 0
        generate.particle2_angle = math.pi
        generate.collision_particles = []
        generate.energy = 0

    elapsed = time.time() - generate.start_time

    # Reset cycle every 20 seconds
    if elapsed > 20:
        generate.start_time = time.time()
        generate.particle1_angle = 0
        generate.particle2_angle = math.pi
        generate.collision_particles = []
        generate.energy = 0
        elapsed = 0

    grid = [[" " for _ in range(width)] for _ in range(height)]

    # Accelerator ring parameters
    center_x, center_y = width // 2, height // 2
    ring_radius_x = min(width // 2 - 5, 35)
    ring_radius_y = min(height // 2 - 3, 15)

    # Draw accelerator ring
    for angle in range(0, 360, 3):
        rad = math.radians(angle)
        x = center_x + int(ring_radius_x * math.cos(rad))
        y = center_y + int(ring_radius_y * math.sin(rad))
        if 0 <= y < height and 0 <= x < width:
            grid[y][x] = "·"

    # Draw detector segments (4 quadrants)
    detector_positions = [0, 90, 180, 270]
    for det_angle in detector_positions:
        rad = math.radians(det_angle)
        det_x = center_x + int((ring_radius_x + 3) * math.cos(rad))
        det_y = center_y + int((ring_radius_y + 2) * math.sin(rad))

        # Detector symbol
        if 0 <= det_y < height and 0 <= det_x < width:
            grid[det_y][det_x] = "▭"

    # Acceleration phase (first 10 seconds)
    if elapsed < 10:
        # Particles accelerate
        speed = (elapsed / 10) * 0.3
        generate.energy = elapsed / 10 * 14  # TeV
    else:
        # Full speed
        speed = 0.3
        generate.energy = 14

    # Update particle positions
    generate.particle1_angle += speed
    generate.particle2_angle -= speed

    # Collision detection (particles meet at top)
    p1_norm = generate.particle1_angle % (2 * math.pi)
    p2_norm = generate.particle2_angle % (2 * math.pi)
    angle_diff = abs(p1_norm - p2_norm)

    # Collision occurs
    if elapsed >= 10 and angle_diff < 0.2 and len(generate.collision_particles) == 0:
        # Create particle shower
        num_particles = random.randint(20, 35)
        for _ in range(num_particles):
            angle = random.uniform(0, 2 * math.pi)
            velocity = random.uniform(1, 4)
            generate.collision_particles.append(
                {
                    "x": center_x,
                    "y": center_y - ring_radius_y,
                    "vx": velocity * math.cos(angle),
                    "vy": velocity * math.sin(angle),
                    "age": 0,
                    "char": random.choice(["*", "+", "·", "×", "π", "μ", "e"]),
                }
            )

    # Draw particles on ring
    # Particle 1 (proton)
    p1_x = center_x + int(ring_radius_x * math.cos(generate.particle1_angle))
    p1_y = center_y + int(ring_radius_y * math.sin(generate.particle1_angle))
    if 0 <= p1_y < height and 0 <= p1_x < width:
        grid[p1_y][p1_x] = "P"

    # Particle 2 (antiproton)
    p2_x = center_x + int(ring_radius_x * math.cos(generate.particle2_angle))
    p2_y = center_y + int(ring_radius_y * math.sin(generate.particle2_angle))
    if 0 <= p2_y < height and 0 <= p2_x < width:
        grid[p2_y][p2_x] = "P̄"

    # Update and draw collision particles
    particles_to_remove = []
    for particle in generate.collision_particles:
        particle["x"] += particle["vx"]
        particle["y"] += particle["vy"]
        particle["age"] += 1

        # Fade out
        if particle["age"] > 25:
            particles_to_remove.append(particle)
        else:
            px, py = int(particle["x"]), int(particle["y"])
            if 0 <= py < height and 0 <= px < width:
                # Detector hit detection
                dist_from_center = math.sqrt(
                    ((px - center_x) / ring_radius_x) ** 2
                    + ((py - center_y) / ring_radius_y) ** 2
                )

                if dist_from_center > 1.2:
                    # Hit detector
                    grid[py][px] = "!"
                else:
                    grid[py][px] = particle["char"]

    for particle in particles_to_remove:
        generate.collision_particles.remove(particle)

    # HUD - Energy display
    energy_display = [
        "╔══════════════════╗",
        "║ LHC ACCELERATOR  ║",
        "╠══════════════════╣",
        f"║ Energy: {generate.energy:5.1f} TeV ║",
        "╚══════════════════╝",
    ]

    for i, line in enumerate(energy_display):
        if i < height:
            for j, char in enumerate(line):
                if j < width:
                    grid[i][j] = char

    # Status display
    if elapsed < 10:
        status = "ACCELERATING"
    elif len(generate.collision_particles) > 0:
        status = "COLLISION!"
    else:
        status = "CIRCULATING"

    status_text = f"STATUS: {status}"
    status_y = height - 2
    if 0 <= status_y < height:
        for i, char in enumerate(status_text):
            if i + 2 < width:
                grid[status_y][i + 2] = char

    # Lap counter
    laps = int(generate.particle1_angle / (2 * math.pi))
    lap_text = f"LAPS: {laps}"
    lap_x = width - len(lap_text) - 2
    if lap_x > 0 and status_y >= 0:
        for i, char in enumerate(lap_text):
            if lap_x + i < width:
                grid[status_y][lap_x + i] = char

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
        time.sleep(0.05)
