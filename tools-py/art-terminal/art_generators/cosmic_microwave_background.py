import random
import math
import time
from colorama import Fore, init

init(autoreset=True)


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


# Define color mappings for temperature levels
TEMP_COLORS = [
    Fore.BLUE + " ",  # Coldest
    Fore.BLUE + ".",
    Fore.CYAN + ":",
    Fore.LIGHTCYAN_EX + "-",
    Fore.WHITE + "=",
    Fore.LIGHTYELLOW_EX + "+",
    Fore.YELLOW + "*",
    Fore.LIGHTRED_EX + "#",
    Fore.RED + "@",  # Hottest
]


# Resolve variable shadowing by renaming parameters in the generate function
def generate(grid_width, grid_height):
    """Generates a realistic Cosmic Microwave Background with scale-invariant fluctuations.

    The CMB shows temperature fluctuations at multiple scales, representing
    the seeds of galaxy formation from the early universe. Includes slow
    wave propagation to show subtle evolution.
    """
    grid = [[" " for _ in range(grid_width)] for _ in range(grid_height)]

    # Very slow time evolution (like cosmic expansion)
    t = time.time() * 0.05  # Extremely slow for CMB

    # Multi-scale noise for realistic CMB structure
    for i in range(grid_height):
        for j in range(grid_width):
            # Combine multiple octaves of noise (like Perlin noise)
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

            # Add subtle bias toward middle values (most areas near average temperature)
            value = (value + 0.5) / 2.0

            # Map to color
            char_idx = int(value * (len(TEMP_COLORS) - 1))
            char_idx = max(0, min(len(TEMP_COLORS) - 1, char_idx))
            grid[i][j] = TEMP_COLORS[char_idx]

    # Add a few "hot spots" and "cold spots" (primordial density fluctuations)
    for _ in range(5):
        cx, cy = random.randint(0, grid_width - 1), random.randint(0, grid_height - 1)
        is_hot = random.choice([True, False])
        radius = random.randint(3, 8)

        for dy in range(-radius, radius + 1):
            for dx in range(-radius, radius + 1):
                dist = math.sqrt(dx**2 + dy**2)
                if dist <= radius:
                    y, x = cy + dy, cx + dx
                    if 0 <= y < grid_height and 0 <= x < grid_width:
                        if is_hot:
                            grid[y][x] = random.choice(
                                [Fore.RED + "#", Fore.RED + "@", Fore.LIGHTRED_EX + "*"]
                            )
                        else:
                            grid[y][x] = random.choice(
                                [Fore.BLUE + " ", Fore.BLUE + ".", Fore.CYAN + ":"]
                            )

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
