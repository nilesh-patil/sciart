import math
import time


def generate(width, height):
    """Generates an enhanced visualization of the Doppler effect.

    Shows a wave source moving across the screen with compressed waves
    in front (blue-shifted) and stretched waves behind (red-shifted).
    """
    grid = [[" " for _ in range(width)] for _ in range(height)]
    # Add title
    title = "DOPPLER EFFECT"
    subtitle = "Frequency shift from source motion"
    title_x = (width - len(title)) // 2
    subtitle_x = (width - len(subtitle)) // 2

    if height > 2:
        for i, char in enumerate(title):
            if 0 <= title_x + i < width:
                grid[0][title_x + i] = char
        for i, char in enumerate(subtitle):
            if 0 <= subtitle_x + i < width:
                grid[1][subtitle_x + i] = char


    # Source moves from left to right
    period = 30  # seconds for one complete cycle
    progress = (time.time() % period) / period
    source_x = int(width * progress)
    source_y = height // 2

    # Draw concentric wave circles with Doppler effect
    num_waves = 15  # Reduced from 20 for less density
    wave_speed = 0.5
    emission_interval = 0.05

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

        # Draw wave circle - sparse, only at specific angles
        for angle in range(0, 360, 15):  # Increased from 5 to 15 for sparsity
            rad = math.radians(angle)
            x = emit_x + int(wave_radius * math.cos(rad) * 2.5)
            y = emit_y + int(wave_radius * math.sin(rad))

            if 0 <= y < height and 0 <= x < width:
                # Determine if in front or behind source
                if x > source_x:
                    # In front - compressed (higher frequency)
                    # Tight spacing: ○○○
                    if wave_idx % 2 == 0:
                        char = "○"
                    else:
                        char = "◎"
                else:
                    # Behind - stretched (lower frequency)
                    # Wide spacing: ○ ○ ○
                    if wave_idx % 3 == 0:
                        char = "○"
                    else:
                        char = " "  # Skip to create wider spacing

                if char != " " and grid[y][x] == " ":
                    grid[y][x] = char

    # Draw the moving source as ◉ with → direction
    if 0 <= source_y < height and 0 <= source_x < width:
        grid[source_y][source_x] = "◉"

        # Arrow showing direction
        if source_x + 1 < width:
            grid[source_y][source_x + 1] = "→"

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
