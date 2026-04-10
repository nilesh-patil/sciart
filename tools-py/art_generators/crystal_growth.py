import random


def generate(width, height):
    """Generates a simulation of crystal growth from a central seed.

    Uses a simple cellular automaton where cells adjacent to
    existing crystal cells can become part of the crystal.
    """
    # Initialize crystal grid on first call or when the terminal is resized
    if getattr(generate, "dims", None) != (width, height):
        generate.dims = (width, height)
        generate.crystal = [[False for _ in range(width)] for _ in range(height)]
        # Seed crystal in the center
        center_x, center_y = width // 2, height // 2
        if 0 <= center_y < height and 0 <= center_x < width:
            generate.crystal[center_y][center_x] = True

    grid = [[" " for _ in range(width)] for _ in range(height)]

    # Growth step - check all empty cells adjacent to crystal
    new_crystal_cells = []
    for i in range(height):
        for j in range(width):
            if not generate.crystal[i][j]:
                # Check if adjacent to existing crystal
                is_adjacent = False
                for di in [-1, 0, 1]:
                    for dj in [-1, 0, 1]:
                        if di == 0 and dj == 0:
                            continue
                        ni, nj = i + di, j + dj
                        if (
                            0 <= ni < height
                            and 0 <= nj < width
                            and generate.crystal[ni][nj]
                        ):
                            is_adjacent = True
                            break
                    if is_adjacent:
                        break

                # Random chance to grow if adjacent
                if is_adjacent and random.random() < 0.1:
                    new_crystal_cells.append((i, j))

    # Add new crystal cells
    for i, j in new_crystal_cells:
        generate.crystal[i][j] = True

    # Draw crystal
    for i in range(height):
        for j in range(width):
            if generate.crystal[i][j]:
                # Vary crystal characters for visual interest
                grid[i][j] = random.choice(["*", "#", "+", "x"])

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
