import math
import time


def generate(width, height):
    """Generates a double-slit wave interference pattern.

    Shows two point sources emitting circular waves that interfere,
    creating alternating bright and dark bands characteristic of
    wave interference.
    """
    grid = [[" " for _ in range(width)] for _ in range(height)]

    # Two wave sources separated vertically
    source1_x = width // 4
    source1_y = height // 2 - 8

    source2_x = width // 4
    source2_y = height // 2 + 8

    # Wave parameters
    wavelength = 8.0  # pixels
    k = 2 * math.pi / wavelength  # wave number
    omega = 2.0  # angular frequency
    t = time.time()

    # Calculate interference pattern for each point
    for i in range(height):
        for j in range(width):
            # Distance to each source
            d1 = math.sqrt((j - source1_x) ** 2 + (i - source1_y) ** 2)
            d2 = math.sqrt((j - source2_x) ** 2 + (i - source2_y) ** 2)

            # Wave from source 1
            wave1 = math.sin(k * d1 - omega * t)

            # Wave from source 2
            wave2 = math.sin(k * d2 - omega * t)

            # Superposition
            amplitude = wave1 + wave2

            # Normalize to 0-1
            intensity = (amplitude + 2) / 4

            # Map to characters based on intensity
            if intensity > 0.85:
                char = "@"
            elif intensity > 0.7:
                char = "#"
            elif intensity > 0.55:
                char = "*"
            elif intensity > 0.45:
                char = ":"
            elif intensity > 0.3:
                char = "."
            else:
                char = " "

            grid[i][j] = char

    # Draw the two sources
    if 0 <= source1_y < height and 0 <= source1_x < width:
        grid[source1_y][source1_x] = "S"
        # Source glow
        for dx, dy in [(-1, 0), (1, 0), (0, -1), (0, 1)]:
            sx, sy = source1_x + dx, source1_y + dy
            if 0 <= sy < height and 0 <= sx < width:
                grid[sy][sx] = "◎"

    if 0 <= source2_y < height and 0 <= source2_x < width:
        grid[source2_y][source2_x] = "S"
        # Source glow
        for dx, dy in [(-1, 0), (1, 0), (0, -1), (0, 1)]:
            sx, sy = source2_x + dx, source2_y + dy
            if 0 <= sy < height and 0 <= sx < width:
                grid[sy][sx] = "◎"

    # Label interference regions
    if height > 10:
        # Constructive interference label (bright band)
        label_y = height // 2
        label_x = width - 20

        if label_x > 0 and 0 <= label_y < height:
            # Find a bright spot
            for test_x in range(width // 2, width):
                test_intensity = 0
                d1 = math.sqrt((test_x - source1_x) ** 2 + (label_y - source1_y) ** 2)
                d2 = math.sqrt((test_x - source2_x) ** 2 + (label_y - source2_y) ** 2)
                wave1 = math.sin(k * d1 - omega * t)
                wave2 = math.sin(k * d2 - omega * t)
                test_intensity = (wave1 + wave2 + 2) / 4

                if test_intensity > 0.8:
                    label_x = test_x + 2
                    break

            constructive = "Constructive"
            if label_x + len(constructive) < width:
                for i, c in enumerate(constructive):
                    if label_x + i < width:
                        grid[label_y][label_x + i] = c

        # Destructive interference label (dark band)
        dark_label_y = height // 2 + 4
        if 0 <= dark_label_y < height:
            # Find a dark spot
            for test_x in range(width // 2, width):
                d1 = math.sqrt(
                    (test_x - source1_x) ** 2 + (dark_label_y - source1_y) ** 2
                )
                d2 = math.sqrt(
                    (test_x - source2_x) ** 2 + (dark_label_y - source2_y) ** 2
                )
                wave1 = math.sin(k * d1 - omega * t)
                wave2 = math.sin(k * d2 - omega * t)
                test_intensity = (wave1 + wave2 + 2) / 4

                if test_intensity < 0.3:
                    label_x = test_x + 2
                    break

            destructive = "Destructive"
            if label_x + len(destructive) < width:
                for i, c in enumerate(destructive):
                    if label_x + i < width:
                        grid[dark_label_y][label_x + i] = c

    # Title
    title = "WAVE INTERFERENCE"
    title_x = 2
    if 0 < height:
        for i, c in enumerate(title):
            if title_x + i < width:
                grid[1][title_x + i] = c

    # Info
    info = f"λ={wavelength:.1f}px"
    info_x = 2
    if 2 < height:
        for i, c in enumerate(info):
            if info_x + i < width:
                grid[2][info_x + i] = c

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
        time.sleep(0.05)
