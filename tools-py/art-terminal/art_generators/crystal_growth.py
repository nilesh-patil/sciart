import random
import math


def generate(width, height):
    """Generates a snowflake-like dendritic crystal with hexagonal symmetry.

    Simulates crystal growth with directional preference along six
    symmetric axes, creating natural snowflake patterns.
    """
    # Initialize crystal grid on first call
    if not hasattr(generate, "crystal"):
        generate.crystal = {}  # Use dict for sparse storage
        generate.age = 0
        # Seed crystal in the center using ⬡ (hexagon)
        center_x, center_y = width // 2, height // 2
        generate.crystal[(center_y, center_x)] = {"age": 0, "type": "core"}

    grid = [[" " for _ in range(width)] for _ in range(height)]
    # Add title
    title = "CRYSTAL GROWTH"
    subtitle = "Hexagonal symmetry formation"
    title_x = (width - len(title)) // 2
    subtitle_x = (width - len(subtitle)) // 2

    if height > 2:
        for i, char in enumerate(title):
            if 0 <= title_x + i < width:
                grid[0][title_x + i] = char
        for i, char in enumerate(subtitle):
            if 0 <= subtitle_x + i < width:
                grid[1][subtitle_x + i] = char


    center_x, center_y = width // 2, height // 2

    # Growth step with hexagonal symmetry
    new_cells = []
    generate.age += 1

    # Get all current crystal cells
    for (cy, cx), cell_data in list(generate.crystal.items()):
        # Try to grow in 6 symmetric directions (hexagonal)
        for angle_deg in [0, 60, 120, 180, 240, 300]:
            angle_rad = math.radians(angle_deg)

            # Primary growth along main axes
            for distance in [1, 2]:
                dx = int(distance * math.cos(angle_rad) * 2.5)
                dy = int(distance * math.sin(angle_rad))
                ny, nx = cy + dy, cx + dx

                if 0 <= ny < height and 0 <= nx < width:
                    if (ny, nx) not in generate.crystal:
                        # Distance from center
                        dist_from_center = math.sqrt(
                            ((nx - center_x) / 2.5) ** 2 + (ny - center_y) ** 2
                        )

                        # Calculate angle from center for directional growth
                        cell_angle = math.atan2(ny - center_y, (nx - center_x) / 2.5)
                        # Promote growth along 6-fold symmetric directions
                        angle_alignment = min(
                            [
                                abs(math.degrees(cell_angle) - a) % 60
                                for a in [0, 60, 120, 180, 240, 300]
                            ]
                        )

                        # Higher probability along main axes
                        base_prob = 0.08
                        if angle_alignment < 15:
                            base_prob = 0.25  # Strong growth along axes
                        elif angle_alignment < 30:
                            base_prob = 0.15  # Side branches

                        # Reduce probability with distance (finite crystal)
                        prob = base_prob * (
                            1 - dist_from_center / (min(width, height) / 2)
                        )

                        if random.random() < prob:
                            new_cells.append(
                                (ny, nx, "branch" if distance > 1 else "main")
                            )

    # Add new cells
    for ny, nx, cell_type in new_cells:
        if (ny, nx) not in generate.crystal:
            generate.crystal[(ny, nx)] = {"age": generate.age, "type": cell_type}

    # Draw crystal with different characters based on type
    for (cy, cx), cell_data in generate.crystal.items():
        if 0 <= cy < height and 0 <= cx < width:
            cell_type = cell_data["type"]

            # Character based on type - using ⬡ for center, ╱─╲│ for branches, ◆ at endpoints
            if cell_type == "core":
                char = "⬡"  # Hexagon center seed
            elif cell_type == "main":
                # Main branches - use directional characters
                dx = cx - center_x
                dy = cy - center_y
                angle = math.atan2(dy, dx / 2.5)
                angle_deg = math.degrees(angle) % 360

                # 6-fold symmetry: determine which axis this is on
                if 75 <= angle_deg < 105 or 255 <= angle_deg < 285:
                    char = "│"  # Vertical
                elif 15 <= angle_deg < 75 or 195 <= angle_deg < 255:
                    char = "╱"  # Diagonal up
                elif 105 <= angle_deg < 165 or 285 <= angle_deg < 345:
                    char = "╲"  # Diagonal down
                else:
                    char = "─"  # Horizontal
            else:  # branch
                # Side branches - smaller characters
                dx = cx - center_x
                dy = cy - center_y
                dist = math.sqrt((dx / 2.5) ** 2 + dy**2)

                # Endpoints get ◆
                if dist > min(width, height) / 3:
                    char = "◆"
                else:
                    # Inner branches
                    angle = math.atan2(dy, dx / 2.5)
                    angle_deg = math.degrees(angle) % 360

                    if 60 <= angle_deg < 120 or 240 <= angle_deg < 300:
                        char = "╲" if angle_deg < 180 else "╱"
                    else:
                        char = "╱" if 0 <= angle_deg < 180 else "╲"

            grid[cy][cx] = char

    # Add subtle "frost" pattern in background (very sparse)
    if generate.age % 30 == 0:  # Occasionally add background frost
        for _ in range(3):  # Reduced from 5
            fx, fy = random.randint(0, width - 1), random.randint(0, height - 1)
            if grid[fy][fx] == " ":
                grid[fy][fx] = "·"

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
