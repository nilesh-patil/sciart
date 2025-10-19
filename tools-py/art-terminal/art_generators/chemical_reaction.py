import random
import math


class Molecule:
    def __init__(self, x, y, mol_type):
        self.x = x
        self.y = y
        self.vx = random.uniform(-1, 1)
        self.vy = random.uniform(-0.5, 0.5)
        self.type = mol_type
        self.size = {"A": 1, "B": 1, "C": 2}[mol_type]
        self.char = {"A": "A", "B": "B", "C": "C"}[mol_type]


def generate(width, height):
    """Generates a chemical reaction visualization with equilibrium.

    Shows molecules A and B reacting to form C in a reversible reaction:
    A + B ⇌ C

    Demonstrates collision theory, reaction kinetics, and chemical equilibrium.
    """
    # Initialize reaction
    if not hasattr(generate, "molecules"):
        generate.molecules = []

        # Initial conditions: 20 A molecules, 20 B molecules
        for _ in range(20):
            generate.molecules.append(
                Molecule(
                    random.uniform(5, width - 5), random.uniform(5, height - 10), "A"
                )
            )

        for _ in range(20):
            generate.molecules.append(
                Molecule(
                    random.uniform(5, width - 5), random.uniform(5, height - 10), "B"
                )
            )

        generate.history = {"A": [20], "B": [20], "C": [0]}

    grid = [[" " for _ in range(width)] for _ in range(height)]

    # Reaction rate constants
    k_forward = 0.3  # A + B -> C
    k_reverse = 0.1  # C -> A + B

    # Collision radius
    collision_dist = 3

    # Update molecules
    box_margin = 2
    for mol in generate.molecules:
        # Brownian motion
        mol.x += mol.vx
        mol.y += mol.vy

        # Boundary collisions
        if mol.x <= box_margin or mol.x >= width - box_margin:
            mol.vx = -mol.vx
            mol.x = max(box_margin, min(width - box_margin, mol.x))

        if mol.y <= box_margin or mol.y >= height - 8:
            mol.vy = -mol.vy
            mol.y = max(box_margin, min(height - 8, mol.y))

    # Check for reactions
    molecules_to_add = []
    molecules_to_remove = []

    # Forward reaction: A + B -> C
    for i, mol1 in enumerate(generate.molecules):
        if mol1 in molecules_to_remove:
            continue

        for j, mol2 in enumerate(generate.molecules[i + 1 :], i + 1):
            if mol2 in molecules_to_remove:
                continue

            # Check if A and B are colliding
            if (mol1.type == "A" and mol2.type == "B") or (
                mol1.type == "B" and mol2.type == "A"
            ):
                distance = math.sqrt((mol1.x - mol2.x) ** 2 + (mol1.y - mol2.y) ** 2)

                if distance < collision_dist:
                    # Reaction occurs with probability
                    if random.random() < k_forward:
                        # Create C molecule
                        new_x = (mol1.x + mol2.x) / 2
                        new_y = (mol1.y + mol2.y) / 2
                        molecules_to_add.append(Molecule(new_x, new_y, "C"))

                        # Remove reactants
                        molecules_to_remove.append(mol1)
                        molecules_to_remove.append(mol2)
                        break

    # Reverse reaction: C -> A + B
    for mol in generate.molecules:
        if mol in molecules_to_remove:
            continue

        if mol.type == "C":
            # Spontaneous decomposition
            if random.random() < k_reverse * 0.1:  # Scaled probability
                # Create A and B molecules
                angle = random.uniform(0, 2 * math.pi)
                offset = 2

                mol_a = Molecule(
                    mol.x + offset * math.cos(angle),
                    mol.y + offset * math.sin(angle),
                    "A",
                )
                mol_b = Molecule(
                    mol.x - offset * math.cos(angle),
                    mol.y - offset * math.sin(angle),
                    "B",
                )

                molecules_to_add.append(mol_a)
                molecules_to_add.append(mol_b)
                molecules_to_remove.append(mol)

    # Apply changes
    for mol in molecules_to_remove:
        if mol in generate.molecules:
            generate.molecules.remove(mol)

    for mol in molecules_to_add:
        generate.molecules.append(mol)

    # Draw container box
    for i in range(height - 7):
        grid[i][0] = "│"
        grid[i][width - 1] = "│"

    for j in range(width):
        grid[0][j] = "─"
        grid[height - 8][j] = "─"

    grid[0][0] = "┌"
    grid[0][width - 1] = "┐"
    grid[height - 8][0] = "└"
    grid[height - 8][width - 1] = "┘"

    # Draw molecules
    for mol in generate.molecules:
        mx, my = int(mol.x), int(mol.y)

        if 0 <= my < height - 8 and 0 <= mx < width:
            grid[my][mx] = mol.char

            # Larger molecules (C) have extended shape
            if mol.type == "C" and mol.size >= 2:
                for dx, dy in [(-1, 0), (1, 0)]:
                    nx, ny = mx + dx, my + dy
                    if 0 <= ny < height - 8 and 0 <= nx < width:
                        if grid[ny][nx] in [" ", "─", "│"]:
                            grid[ny][nx] = "c"

    # Count molecules
    counts = {"A": 0, "B": 0, "C": 0}
    for mol in generate.molecules:
        counts[mol.type] += 1

    # Update history
    generate.history["A"].append(counts["A"])
    generate.history["B"].append(counts["B"])
    generate.history["C"].append(counts["C"])

    # Keep history limited
    max_history = 60
    for key in generate.history:
        if len(generate.history[key]) > max_history:
            generate.history[key].pop(0)

    # Draw concentration graph
    graph_y_start = height - 6
    graph_height = 5
    graph_width = min(60, width - 4)

    # Graph axes
    for y in range(graph_y_start, height):
        if y < height:
            grid[y][2] = "│"

    for x in range(2, 2 + graph_width):
        if x < width and graph_y_start + graph_height - 1 < height:
            grid[graph_y_start + graph_height - 1][x] = "─"

    # Plot concentrations
    history_len = len(generate.history["A"])
    for i, key in enumerate(["A", "B", "C"]):
        char = {"A": "a", "B": "b", "C": "c"}[key]

        for plot_x in range(min(history_len, graph_width)):
            idx = max(0, history_len - graph_width + plot_x)
            concentration = generate.history[key][idx]

            # Scale to graph
            plot_y = (
                graph_y_start
                + graph_height
                - 1
                - int((concentration / 25) * (graph_height - 1))
            )

            screen_x = 3 + plot_x
            if 0 <= plot_y < height and 0 <= screen_x < width:
                if grid[plot_y][screen_x] in [" ", "─"]:
                    grid[plot_y][screen_x] = char

    # Labels
    title = "CHEMICAL REACTION: A + B ⇌ C"
    for i, c in enumerate(title):
        if i + 2 < width and height - 7 < height:
            grid[height - 7][i + 2] = c

    # Current counts
    count_text = f"[A]={counts['A']} [B]={counts['B']} [C]={counts['C']}"
    count_x = width - len(count_text) - 2
    if count_x > 0 and height - 7 < height:
        for i, c in enumerate(count_text):
            if count_x + i < width:
                grid[height - 7][count_x + i] = c

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
