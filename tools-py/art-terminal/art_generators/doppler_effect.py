import math
import time


def generate(width, height):
    """Generates an enhanced visualization of the Doppler effect.

    Shows a wave source moving across the screen with compressed waves
    in front (blue-shifted) and stretched waves behind (red-shifted).
    Includes wave density visualization.
    """
    grid = [[" " for _ in range(width)] for _ in range(height)]

    # Source moves from left to right
    period = 30  # seconds for one complete cycle
    progress = (time.time() % period) / period
    source_x = int(width * progress)
    source_y = height // 2

    # Draw concentric wave circles with Doppler effect
    num_waves = 20
    wave_speed = 0.5
    emission_interval = 0.04

    for wave_idx in range(num_waves):
        # Time when this wave was emitted
        wave_time = progress - (wave_idx * emission_interval)
        if wave_time < 0:
            wave_time += 1

        # Position where wave was emitted
        emit_x = int(width * wave_time)
        emit_y = source_y

        # Current radius of this wave
        time_since_emission = progress - wave_time
        if time_since_emission < 0:
            time_since_emission += 1

        wave_radius = time_since_emission * width * wave_speed

        # Draw wave circle with varying intensity
        for angle in range(0, 360, 5):
            rad = math.radians(angle)
            x = emit_x + int(wave_radius * math.cos(rad) * 2.5)
            y = emit_y + int(wave_radius * math.sin(rad))

            if 0 <= y < height and 0 <= x < width:
                # Determine if in front or behind source
                if x > source_x:
                    # In front - compressed (higher frequency)
                    char = "≈" if wave_idx % 2 == 0 else "~"
                else:
                    # Behind - stretched (lower frequency)
                    char = "~" if wave_idx % 3 == 0 else "·"

                if grid[y][x] == " ":
                    grid[y][x] = char

    # Draw wave density bars to show compression
    density_y = height - 5
    if density_y > 0:
        # Calculate local wave density at different x positions
        for check_x in range(0, width, 2):
            # Count waves at this position
            density = 0
            for wave_idx in range(num_waves):
                wave_time = progress - (wave_idx * emission_interval)
                if wave_time < 0:
                    wave_time += 1

                emit_x = int(width * wave_time)
                time_since_emission = progress - wave_time
                if time_since_emission < 0:
                    time_since_emission += 1

                wave_radius = time_since_emission * width * wave_speed

                # Check if wave passes through this x position
                dist = abs(check_x - emit_x)
                if abs(dist - wave_radius) < 3:
                    density += 1

            # Draw density bar
            bar_height = min(4, density)
            for bar_y in range(bar_height):
                y = density_y + bar_y
                if 0 <= y < height and 0 <= check_x < width:
                    grid[y][check_x] = "│"

    # Draw the moving source with direction indicator
    if 0 <= source_y < height and 0 <= source_x < width:
        grid[source_y][source_x] = "@"
        # Arrow showing direction
        if source_x + 1 < width:
            grid[source_y][source_x + 1] = "→"

    # Add labels
    label_y = 1
    if label_y < height:
        front_label = "Compressed (blue-shift)"
        behind_label = "Stretched (red-shift)"

        # Front label
        if source_x + 5 < width - len(front_label):
            for i, c in enumerate(front_label):
                if source_x + 5 + i < width:
                    grid[label_y][source_x + 5 + i] = c

        # Behind label
        if source_x > len(behind_label) + 5:
            for i, c in enumerate(behind_label):
                if source_x - len(behind_label) - 5 + i >= 0:
                    grid[label_y][source_x - len(behind_label) - 5 + i] = c

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
        time.sleep(0.1)
