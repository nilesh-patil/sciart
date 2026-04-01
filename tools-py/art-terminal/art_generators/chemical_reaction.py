import random
import math


class Molecule:
    def __init__(self, x, y, mol_type):
        self.x = x
        self.y = y
        self.vx = random.uniform(-1, 1)
        self.vy = random.uniform(-0.5, 0.5)
        self.type = mol_type
        # Size based on type: A=· (small), B=∘ (medium), C=○ (large/product)
        self.size = {"A": 1, "B": 1, "C": 2}[mol_type]
        self.char = {"A": "·", "B": "∘", "C": "○"}[mol_type]


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
                    random.uniform(5, width - 5), random.uniform(5, height - 5), "A"
                )
            )

        for _ in range(20):
            generate.molecules.append(
                Molecule(
                    random.uniform(5, width - 5), random.uniform(5, height - 5), "B"
                )
            )

        generate.history = {"A": [20], "B": [20], "C": [0]}

    grid = [[" " for _ in range(width)] for _ in range(height)]
    # Add title
    title = "CHEMICAL EQUILIBRIUM"
    subtitle = "Reversible reaction dynamics"
    title_x = (width - len(title)) // 2
    subtitle_x = (width - len(subtitle)) // 2

    if height > 2:
        for i, char in enumerate(title):
            if 0 <= title_x + i < width:
                grid[0][title_x + i] = char
        for i, char in enumerate(subtitle):
            if 0 <= subtitle_x + i < width:
                grid[1][subtitle_x + i] = char


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

        if mol.y <= box_margin or mol.y >= height - box_margin:
            mol.vy = -mol.vy
            mol.y = max(box_margin, min(height - box_margin, mol.y))

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

                        # Show collision flash with ✦
                        cx, cy = int(new_x), int(new_y)
                        if 0 <= cy < height and 0 <= cx < width:
                            grid[cy][cx] = "✦"

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

    # Draw molecules using distinct sizes: · for A, ∘ for B, ○ for C
    for mol in generate.molecules:
        mx, my = int(mol.x), int(mol.y)

        if 0 <= my < height and 0 <= mx < width:
            grid[my][mx] = mol.char

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

    # Draw concentration as pure dot histogram (no axes) - bottom section
    graph_y_start = height - 8
    graph_height = 7
    graph_width = min(width - 4, 60)

    # Plot concentrations as vertical dot columns
    history_len = len(generate.history["A"])
    for plot_x in range(min(history_len, graph_width)):
        idx = max(0, history_len - graph_width + plot_x)

        # Get concentrations for each type
        conc_a = generate.history["A"][idx]
        conc_b = generate.history["B"][idx]
        conc_c = generate.history["C"][idx]

        screen_x = 2 + plot_x

        # Draw A concentration (bottom)
        for y_offset in range(min(conc_a // 2, graph_height)):
            plot_y = graph_y_start + graph_height - 1 - y_offset
            if 0 <= plot_y < height and 0 <= screen_x < width:
                if grid[plot_y][screen_x] == " ":
                    grid[plot_y][screen_x] = "·"  # A type

        # Draw B concentration (stacked on A)
        for y_offset in range(min(conc_b // 2, graph_height)):
            plot_y = graph_y_start + graph_height - 1 - (conc_a // 2) - y_offset
            if 0 <= plot_y < height and 0 <= screen_x < width:
                if grid[plot_y][screen_x] == " ":
                    grid[plot_y][screen_x] = "∘"  # B type

        # Draw C concentration (stacked on A+B)
        for y_offset in range(min(conc_c // 2, graph_height)):
            plot_y = graph_y_start + graph_height - 1 - (conc_a // 2) - (conc_b // 2) - y_offset
            if 0 <= plot_y < height and 0 <= screen_x < width:
                if grid[plot_y][screen_x] == " ":
                    grid[plot_y][screen_x] = "○"  # C type

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
