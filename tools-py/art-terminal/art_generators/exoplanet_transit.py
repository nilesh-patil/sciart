import math
import time


def generate(width, height):
    """Generates exoplanet transit with light curve visualization."""
    grid = [[" " for _ in range(width)] for _ in range(height)]

    # Add title
    title = "EXOPLANET TRANSIT"
    subtitle = "Brightness dip detection method"
    title_x = (width - len(title)) // 2
    subtitle_x = (width - len(subtitle)) // 2

    if height > 2:
        for i, char in enumerate(title):
            if 0 <= title_x + i < width:
                grid[0][title_x + i] = char
        for i, char in enumerate(subtitle):
            if 0 <= subtitle_x + i < width:
                grid[1][subtitle_x + i] = char

    star_radius = min(width // 8, height // 4)
    star_x, star_y = width // 3, height // 2

    planet_radius = max(2, star_radius // 4)
    transit_duration = 40  # seconds
    progress = (time.time() % transit_duration) / transit_duration

    # Planet moves across the star
    planet_x = int(star_x - star_radius * 3 + progress * star_radius * 6)
    planet_y = star_y

    # Draw the star with gradient shading
    for i in range(height):
        for j in range(width):
            dx = (j - star_x) / 2.5
            dy = i - star_y
            dist = math.sqrt(dx**2 + dy**2)

            if dist <= star_radius:
                # Limb darkening effect
                brightness = 1 - (dist / star_radius) ** 2 * 0.4
                if brightness > 0.8:
                    grid[i][j] = "⊙"
                elif brightness > 0.6:
                    grid[i][j] = "◉"
                elif brightness > 0.4:
                    grid[i][j] = "○"
                else:
                    grid[i][j] = "∘"

    # Draw the planet (dark circle)
    for i in range(height):
        for j in range(width):
            dx = (j - planet_x) / 2.5
            dy = i - planet_y
            dist = math.sqrt(dx**2 + dy**2)
            if dist <= planet_radius:
                grid[i][j] = "●"

    # Draw light curve on the right side
    curve_x_start = int(width * 0.6)
    curve_y_center = height // 2
    curve_height = min(20, height - 4)

    # Draw axes (minimal)
    for y in range(
        curve_y_center - curve_height // 2, curve_y_center + curve_height // 2
    ):
        if 0 <= y < height and curve_x_start < width:
            if y % 3 == 0:  # Sparse axis
                grid[y][curve_x_start] = "│"

    for x in range(curve_x_start, min(width, curve_x_start + 30)):
        if 0 <= curve_y_center < height:
            if x % 3 == 0:  # Sparse axis
                grid[curve_y_center][x] = "─"

    # Plot light curve
    for plot_x in range(curve_x_start + 1, min(width - 1, curve_x_start + 28)):
        # Relative position in the curve
        curve_progress = (plot_x - curve_x_start - 1) / 27

        # Calculate brightness based on overlap
        sim_planet_x = star_x - star_radius * 3 + curve_progress * star_radius * 6
        sim_dx = (sim_planet_x - star_x) / 2.5
        sim_dist = abs(sim_dx)

        if sim_dist < (star_radius + planet_radius):
            # During transit - calculate dimming
            overlap_factor = max(0, 1 - (sim_dist / star_radius))
            dimming = overlap_factor * (planet_radius / star_radius) ** 2 * 0.3
        else:
            dimming = 0

        # Plot point
        brightness = 1 - dimming
        plot_y = curve_y_center - int((brightness - 0.85) * curve_height)
        if 0 <= plot_y < height:
            grid[plot_y][plot_x] = "·"

        # Current position marker
        if abs(curve_progress - progress) < 0.02:
            if 0 <= plot_y < height:
                grid[plot_y][plot_x] = "※"

    return "\n".join(["".join(row) for row in grid])
