import math
import time


def generate(width, height):
    """Generates a visualization of the Doppler effect.

    Shows a wave source moving across the screen with compressed waves
    in front and stretched waves behind.
    """
    grid = [[" " for _ in range(width)] for _ in range(height)]

    # Source moves from left to right
    period = 30  # seconds for one complete cycle
    progress = (time.time() % period) / period
    source_x = int(width * progress)
    source_y = height // 2

    # Draw concentric wave circles
    # Circles emitted at different times from different positions
    num_waves = 15
    for wave_idx in range(num_waves):
        # Time when this wave was emitted
        wave_time = progress - (wave_idx * 0.05)
        if wave_time < 0:
            wave_time += 1

        # Position where wave was emitted
        emit_x = int(width * wave_time)
        emit_y = source_y

        # Current radius of this wave
        wave_radius = (progress - wave_time) * width * 0.5
        if wave_radius < 0:
            wave_radius += width * 0.5

        # Draw wave circle
        for angle in range(0, 360, 10):
            rad = math.radians(angle)
            x = emit_x + int(wave_radius * math.cos(rad) * 2.5)
            y = emit_y + int(wave_radius * math.sin(rad))

            if 0 <= y < height and 0 <= x < width:
                grid[y][x] = "~"

    # Draw the moving source
    if 0 <= source_y < height and 0 <= source_x < width:
        grid[source_y][source_x] = "@"

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
