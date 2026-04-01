import random
import math


def generate(width, height):
    """Generates a 2D representation of gravitational lensing."""
    grid = [[" " for _ in range(width)] for _ in range(height)]

    lensing_mass_x, lensing_mass_y = width // 2, height // 2
    einstein_radius = 15

    # Generate a background of stars
    if not hasattr(generate, "stars"):
        generate.stars = []
        for _ in range(200):
            x = random.randint(0, width - 1)
            y = random.randint(0, height - 1)
            generate.stars.append((x, y))

    for star_x, star_y in generate.stars:
        dx = star_x - lensing_mass_x
        dy = star_y - lensing_mass_y
        distance = math.sqrt(dx**2 + dy**2)

        if distance == 0:
            continue

        # Simplified lensing formula
        if distance < einstein_radius * 2:
            deflection_angle = einstein_radius / distance
            lensed_x = int(star_x + dx * deflection_angle)
            lensed_y = int(star_y + dy * deflection_angle)
        else:
            lensed_x, lensed_y = star_x, star_y

        if 0 <= lensed_y < height and 0 <= lensed_x < width:
            grid[lensed_y][lensed_x] = random.choice([".", "*"])

    # Draw the lensing mass
    for i in range(lensing_mass_y - 2, lensing_mass_y + 2):
        for j in range(lensing_mass_x - 2, lensing_mass_x + 2):
            if 0 <= i < height and 0 <= j < width:
                grid[i][j] = "#"

    return "\n".join(["".join(row) for row in grid])
