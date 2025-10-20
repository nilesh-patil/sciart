import math
import time


def generate(width, height):
    """Generates realistic moon phases with proper shading and terminator line."""
    grid = [[" " for _ in range(width)] for _ in range(height)]

    # Add title
    title = "LUNAR PHASES"
    subtitle = "Illumination cycle over 29.5 days"
    title_x = (width - len(title)) // 2
    subtitle_x = (width - len(subtitle)) // 2

    if height > 2:
        for i, char in enumerate(title):
            if 0 <= title_x + i < width:
                grid[0][title_x + i] = char
        for i, char in enumerate(subtitle):
            if 0 <= subtitle_x + i < width:
                grid[1][subtitle_x + i] = char

    moon_radius = min(width // 6, height // 3)
    moon_x, moon_y = width // 2, height // 2

    # Cycle through phases over 60 seconds
    cycle_time = time.time() % 60
    phase = cycle_time / 60  # 0 to 1

    # Calculate illumination angle
    light_angle = phase * 2 * math.pi

    for i in range(height):
        for j in range(width):
            # Adjust for aspect ratio
            dx = (j - moon_x) / 2.5
            dy = i - moon_y
            distance = math.sqrt(dx**2 + dy**2)

            if distance <= moon_radius:
                # Normalized coordinates on the moon surface
                norm_x = dx / moon_radius
                norm_y = dy / moon_radius

                # Calculate 3D sphere position (z-coordinate)
                z_squared = 1 - norm_x**2 - norm_y**2
                if z_squared > 0:
                    z = math.sqrt(z_squared)

                    # Light direction based on phase
                    light_x = math.cos(light_angle)
                    light_y = 0
                    light_z = math.sin(light_angle)

                    # Surface normal (pointing outward from sphere)
                    normal_x, normal_y, normal_z = norm_x, norm_y, z

                    # Dot product for lighting (Lambertian reflection)
                    illumination = (
                        normal_x * light_x + normal_y * light_y + normal_z * light_z
                    )

                    # Determine character based on illumination
                    if illumination > 0.7:
                        grid[i][j] = "●"
                    elif illumination > 0.4:
                        grid[i][j] = "◉"
                    elif illumination > 0.1:
                        grid[i][j] = "◎"
                    elif illumination > -0.1:
                        grid[i][j] = "│"  # Terminator line
                    elif illumination > -0.4:
                        grid[i][j] = "░"
                    elif illumination > -0.7:
                        grid[i][j] = "▒"
                    else:
                        grid[i][j] = "▓"

    return "\n".join(["".join(row) for row in grid])
