import random
import time


def generate(width, height):
    """Generates a visualization of radioactive decay with half-life graph.

    Shows unstable isotopes decaying over time with radiation particles
    and a real-time decay curve.
    """
    # Initialize isotopes on first call or if dimensions changed
    sample_width = int(width * 0.5)  # Reduced from 0.6 for more space for graph

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
                row.append({"decayed": False})
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
        generate.radiation_particles = []
        elapsed = 0

    grid = [[" " for _ in range(width)] for _ in range(height)]
    # Add title
    title = "RADIOACTIVE DECAY"
    subtitle = "Exponential half-life process"
    title_x = (width - len(title)) // 2
    subtitle_x = (width - len(subtitle)) // 2

    if height > 2:
        for i, char in enumerate(title):
            if 0 <= title_x + i < width:
                grid[0][title_x + i] = char
        for i, char in enumerate(subtitle):
            if 0 <= subtitle_x + i < width:
                grid[1][subtitle_x + i] = char


    # Update isotopes
    undecayed_count = 0
    total_count = 0
    for i in range(height):
        for j in range(sample_width):
            isotope = generate.isotopes[i][j]
            total_count += 1

            if not isotope["decayed"]:
                undecayed_count += 1
                # Probabilistic decay
                decay_chance = 1 - (0.5 ** (1 / (half_life * 10)))  # per frame
                if random.random() < decay_chance:
                    isotope["decayed"] = True

                    # Emit radiation particle
                    if random.random() < 0.15:
                        speed = random.uniform(0.5, 1.5)
                        generate.radiation_particles.append(
                            {
                                "x": j,
                                "y": i,
                                "vx": speed * random.choice([-1, 1]),
                                "vy": speed * random.choice([-1, 1]),
                                "age": 0,
                            }
                        )

            # Draw isotope: ● for undecayed, ∘ for decayed
            if isotope["decayed"]:
                grid[i][j] = "∘"
            else:
                grid[i][j] = "●"

    # Update and draw radiation particles with trajectories ╱╲
    particles_to_remove = []
    for particle in generate.radiation_particles:
        particle["x"] += particle["vx"]
        particle["y"] += particle["vy"]
        particle["age"] += 1

        if particle["age"] > 30 or particle["y"] < 0 or particle["y"] >= height:
            particles_to_remove.append(particle)
        else:
            px, py = int(particle["x"]), int(particle["y"])

            # Draw trajectory based on direction
            if 0 <= py < height and 0 <= px < width:
                # Draw radiation particle as ·
                if grid[py][px] == " ":
                    grid[py][px] = "·"

    for p in particles_to_remove:
        generate.radiation_particles.remove(p)

    # Draw decay curve graph on right side as pure dot plot
    graph_x_start = sample_width + 3
    graph_width = width - graph_x_start - 2
    graph_height = height - 4

    if graph_width > 10 and graph_height > 5:
        # Plot decay curve using only dots
        total_isotopes = len(generate.isotopes) * sample_width

        # Plot expected decay curve
        for plot_x in range(graph_width - 1):
            # Time represented by this x position
            t = (plot_x / graph_width) * 60  # 60 second span

            # Expected number undecayed at time t
            expected_fraction = 0.5 ** (t / half_life)

            # Plot position
            plot_y = int(height - 2 - expected_fraction * graph_height)
            actual_x = graph_x_start + plot_x

            if 1 <= plot_y < height - 1 and actual_x < width:
                if grid[plot_y][actual_x] == " ":
                    grid[plot_y][actual_x] = "·"

        # Mark current time with ※ (progress marker)
        if elapsed < 60:
            current_x = int(graph_x_start + (elapsed / 60) * graph_width)
            current_fraction = undecayed_count / total_isotopes if total_count > 0 else 0
            current_y = int(height - 2 - current_fraction * graph_height)

            if 1 <= current_y < height - 1 and current_x < width:
                grid[current_y][current_x] = "※"

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
