import random
import time


def generate(width, height):
    """Generates a visualization of radioactive decay with half-life graph.

    Shows unstable isotopes decaying over time with radiation particles
    and a real-time decay curve.
    """
    # Initialize isotopes on first call or if dimensions changed
    sample_width = int(width * 0.6)

    if (
        not hasattr(generate, "isotopes")
        or len(generate.isotopes) != height
        or (len(generate.isotopes) > 0 and len(generate.isotopes[0]) != sample_width)
    ):
        # Reinitialize if dimensions changed
        generate.isotopes = []
        generate.start_time = time.time()
        generate.radiation_particles = []

        for i in range(height):
            row = []
            for j in range(sample_width):
                row.append({"decayed": False, "char": "U"})  # U for Uranium
            generate.isotopes.append(row)

    # Calculate decay based on elapsed time
    elapsed = time.time() - generate.start_time
    half_life = 15.0  # seconds

    # Reset after 60 seconds
    if elapsed > 60:
        generate.start_time = time.time()
        for row in generate.isotopes:
            for isotope in row:
                isotope["decayed"] = False
                isotope["char"] = "U"
        generate.radiation_particles = []
        elapsed = 0

    grid = [[" " for _ in range(width)] for _ in range(height)]

    # Update isotopes (sample_width already calculated above)
    undecayed_count = 0
    for i in range(height):
        for j in range(sample_width):
            isotope = generate.isotopes[i][j]

            if not isotope["decayed"]:
                undecayed_count += 1
                # Probabilistic decay
                decay_chance = 1 - (0.5 ** (1 / (half_life * 10)))  # per frame
                if random.random() < decay_chance:
                    isotope["decayed"] = True
                    isotope["char"] = random.choice(["·", ".", "+"])

                    # Emit radiation particle
                    if random.random() < 0.1:
                        generate.radiation_particles.append(
                            {
                                "x": j,
                                "y": i,
                                "vx": random.uniform(-1, 1),
                                "vy": random.uniform(-1, 1),
                                "age": 0,
                            }
                        )

            grid[i][j] = isotope["char"]

    # Update and draw radiation particles
    particles_to_remove = []
    for particle in generate.radiation_particles:
        particle["x"] += particle["vx"]
        particle["y"] += particle["vy"]
        particle["age"] += 1

        if particle["age"] > 30 or particle["y"] < 0 or particle["y"] >= height:
            particles_to_remove.append(particle)
        else:
            px, py = int(particle["x"]), int(particle["y"])
            if 0 <= py < height and 0 <= px < sample_width and grid[py][px] == " ":
                grid[py][px] = "γ"  # gamma radiation

    for p in particles_to_remove:
        generate.radiation_particles.remove(p)

    # Draw decay curve graph on right side
    graph_x_start = sample_width + 2
    graph_width = width - graph_x_start - 2
    graph_height = height - 6

    if graph_width > 10 and graph_height > 5:
        # Draw axes
        for y in range(2, height - 2):
            if graph_x_start < width:
                grid[y][graph_x_start] = "│"

        for x in range(graph_x_start, min(width, graph_x_start + graph_width)):
            if height - 3 < height:
                grid[height - 3][x] = "─"

        # Plot decay curve
        total_isotopes = len(generate.isotopes) * sample_width
        for plot_x in range(graph_width - 1):
            # Time represented by this x position
            t = (plot_x / graph_width) * 60  # 60 second span

            # Expected number undecayed at time t
            expected_fraction = 0.5 ** (t / half_life)

            # Plot position
            plot_y = int(height - 4 - expected_fraction * graph_height)
            actual_x = graph_x_start + 1 + plot_x

            if 2 <= plot_y < height - 3 and actual_x < width:
                grid[plot_y][actual_x] = "*"

            # Mark current time
            if abs(t - elapsed) < 0.5:
                current_fraction = undecayed_count / total_isotopes
                current_y = int(height - 4 - current_fraction * graph_height)
                if 2 <= current_y < height - 3 and actual_x < width:
                    grid[current_y][actual_x] = "●"

        # Labels
        title = "Half-life Decay"
        if len(title) + graph_x_start < width:
            for i, c in enumerate(title):
                if graph_x_start + i < width:
                    grid[0][graph_x_start + i] = c

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
        time.sleep(0.2)
