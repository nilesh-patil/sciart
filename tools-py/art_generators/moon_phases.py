import math
import time


def generate(width, height):
    """Generates realistic moon phases with proper shading and terminator line."""
    grid = [[" " for _ in range(width)] for _ in range(height)]

    # Clamp to >= 1: the radius is used as a divisor and small terminals
    # would otherwise collapse it to 0
    moon_radius = max(1, min(width // 6, height // 3))
    moon_x, moon_y = width // 2, height // 2

    # Cycle through phases over 60 seconds
    cycle_time = time.time() % 60
    phase = cycle_time / 60  # 0 to 1

    # Calculate illumination angle
    light_angle = phase * 2 * math.pi

    # Characters for different shading levels
    lit_chars = ["@", "#", "%", "*", "+", ":", ".", " "]

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

                    # Map illumination to character
                    if illumination > 0:
                        # Lit side with shading
                        shade_index = int((1 - illumination) * (len(lit_chars) - 1))
                        grid[i][j] = lit_chars[min(shade_index, len(lit_chars) - 4)]
                    else:
                        # Dark side
                        grid[i][j] = "."

    # Add phase label
    phase_names = [
        "New Moon",
        "Waxing Crescent",
        "First Quarter",
        "Waxing Gibbous",
        "Full Moon",
        "Waning Gibbous",
        "Last Quarter",
        "Waning Crescent",
    ]
    phase_index = int((phase + 0.0625) * 8) % 8
    label = phase_names[phase_index]

    # Draw label at bottom
    label_x = moon_x - len(label) // 2
    label_y = moon_y + moon_radius + 3
    if 0 <= label_y < height:
        for idx, char in enumerate(label):
            x = label_x + idx
            if 0 <= x < width:
                grid[label_y][x] = char

    return "\n".join(["".join(row) for row in grid])
