import math
import time


def generate(width, height):
    """Generates a double-slit wave interference pattern.

    Shows two point sources emitting circular waves that interfere,
    creating alternating bright and dark bands characteristic of
    wave interference.
    """
    grid = [[" " for _ in range(width)] for _ in range(height)]
    # Add title
    title = "WAVE INTERFERENCE"
    subtitle = "Superposition of two coherent sources"
    title_x = (width - len(title)) // 2
    subtitle_x = (width - len(subtitle)) // 2

    if height > 2:
        for i, char in enumerate(title):
            if 0 <= title_x + i < width:
                grid[0][title_x + i] = char
        for i, char in enumerate(subtitle):
            if 0 <= subtitle_x + i < width:
                grid[1][subtitle_x + i] = char


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
            # Constructive: ●◉⊙ Destructive: sparse ·
            if intensity > 0.85:
                char = "⊙"  # Maximum constructive
            elif intensity > 0.75:
                char = "◉"  # Strong constructive
            elif intensity > 0.65:
                char = "●"  # Medium constructive
            elif intensity > 0.55:
                char = "◎"  # Wave
            elif intensity > 0.48:
                char = "○"  # Wave
            elif intensity > 0.42:
                char = "∘"  # Wave
            elif intensity > 0.25:
                char = "·"  # Destructive
            else:
                char = " "  # Dark (destructive)

            # Only draw if not background
            if char != " ":
                grid[i][j] = char

    # Draw the two sources as ✦
    if 0 <= source1_y < height and 0 <= source1_x < width:
        grid[source1_y][source1_x] = "✦"

    if 0 <= source2_y < height and 0 <= source2_x < width:
        grid[source2_y][source2_x] = "✦"

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
