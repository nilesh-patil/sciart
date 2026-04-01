import math
import time


def smooth_noise(x, y, scale, seed_offset=0, time_offset=0):
    """Generate smooth noise using interpolated random values with time evolution."""
    # Grid coordinates (add time for slow evolution)
    x0 = int((x + time_offset) / scale)
    y0 = int((y + time_offset * 0.5) / scale)
    x1 = x0 + 1
    y1 = y0 + 1

    # Fractional part
    fx = (x / scale) - x0
    fy = (y / scale) - y0

    # Smooth interpolation (cosine)
    sx = (1 - math.cos(fx * math.pi)) * 0.5
    sy = (1 - math.cos(fy * math.pi)) * 0.5

    # Corner values (pseudo-random based on position)
    def hash_noise(ix, iy):
        n = ix * 374761393 + iy * 668265263 + seed_offset
        n = (n ^ (n >> 13)) * 1274126177
        return ((n ^ (n >> 16)) & 0x7FFFFFFF) / 0x7FFFFFFF

    n00 = hash_noise(x0, y0)
    n10 = hash_noise(x1, y0)
    n01 = hash_noise(x0, y1)
    n11 = hash_noise(x1, y1)

    # Bilinear interpolation
    n0 = n00 * (1 - sx) + n10 * sx
    n1 = n01 * (1 - sx) + n11 * sx
    return n0 * (1 - sy) + n1 * sy


# Define color mappings for temperature levels (pure blocks)
TEMP_CHARS = ["░", "▒", "▓"]


def generate(grid_width, grid_height):
    """Generates Cosmic Microwave Background with scale-invariant fluctuations."""
    grid = [[" " for _ in range(grid_width)] for _ in range(grid_height)]

    # Add title
    title = "COSMIC MICROWAVE BACKGROUND"
    subtitle = "Temperature fluctuations from early universe"
    title_x = (grid_width - len(title)) // 2
    subtitle_x = (grid_width - len(subtitle)) // 2

    if grid_height > 2:
        for i, char in enumerate(title):
            if 0 <= title_x + i < grid_width:
                grid[0][title_x + i] = char
        for i, char in enumerate(subtitle):
            if 0 <= subtitle_x + i < grid_width:
                grid[1][subtitle_x + i] = char

    # Very slow time evolution
    t = time.time() * 0.05

    # Multi-scale noise for realistic CMB structure
    for i in range(grid_height):
        for j in range(grid_width):
            # Combine multiple octaves of noise
            value = 0.0

            # Large-scale fluctuations (acoustic peaks)
            value += smooth_noise(j, i, 20.0, 1000, t) * 0.5

            # Medium-scale structures
            value += smooth_noise(j, i, 10.0, 2000, t * 0.5) * 0.3

            # Small-scale variations
            value += smooth_noise(j, i, 5.0, 3000, t * 0.3) * 0.15

            # Very fine details
            value += smooth_noise(j, i, 2.0, 4000, t * 0.2) * 0.05

            # Normalize to 0-1 range
            value = value / (0.5 + 0.3 + 0.15 + 0.05)

            # Add subtle bias toward middle values
            value = (value + 0.5) / 2.0

            # Map to character (only use block characters)
            char_idx = int(value * (len(TEMP_CHARS) - 1))
            char_idx = max(0, min(len(TEMP_CHARS) - 1, char_idx))
            grid[i][j] = TEMP_CHARS[char_idx]

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
