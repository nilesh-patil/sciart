import math
import time


def generate(width, height):
    """Generates Voyager's historic grand tour past Jupiter and Saturn.

    Shows the spacecraft passing by both gas giants with their distinctive
    features and a gravity assist trajectory.
    """
    grid = [[" " for _ in range(width)] for _ in range(height)]
    # Add title
    title = "VOYAGER MISSION"
    subtitle = "Grand tour of outer solar system"
    title_x = (width - len(title)) // 2
    subtitle_x = (width - len(subtitle)) // 2

    if height > 2:
        for i, char in enumerate(title):
            if 0 <= title_x + i < width:
                grid[0][title_x + i] = char
        for i, char in enumerate(subtitle):
            if 0 <= subtitle_x + i < width:
                grid[1][subtitle_x + i] = char


    # Timeline: 60 seconds total
    elapsed = time.time() % 60

    # Probe stays fixed on left side
    probe_x = width // 5
    probe_y = height // 2

    # Background stars - much sparser
    for _ in range(20):  # Reduced from 80
        star_seed = _ * 7919
        sx = star_seed % width
        sy = (star_seed // width) % height
        if 0 <= sy < height and 0 <= sx < width:
            grid[sy][sx] = "·"

    # Jupiter encounter: 0-35s
    if elapsed < 35:
        jupiter_progress = elapsed / 35
        # Jupiter approaches from right
        jupiter_x = int(width - jupiter_progress * (width - probe_x - 30))
        jupiter_y = height // 2
        jupiter_radius = 12

        # Draw Jupiter if on screen
        if jupiter_x < width + jupiter_radius:
            # Draw the planet
            for dy in range(-jupiter_radius, jupiter_radius + 1):
                for dx in range(
                    -int(jupiter_radius * 2.5), int(jupiter_radius * 2.5) + 1
                ):
                    dist = math.sqrt((dx / 2.5) ** 2 + dy**2)
                    if dist <= jupiter_radius:
                        y, x = jupiter_y + dy, jupiter_x + dx
                        if 0 <= y < height and 0 <= x < width:
                            # Banding pattern
                            band_pattern = (y + int(time.time() * 2)) % 4
                            if dist < jupiter_radius - 2:
                                # Great Red Spot
                                grs_dx = dx / 2.5 - 3
                                grs_dy = dy + 2
                                grs_dist = math.sqrt(grs_dx**2 + (grs_dy * 1.5) ** 2)
                                if grs_dist < 3:
                                    grid[y][x] = "◉"
                                elif band_pattern == 0:
                                    grid[y][x] = "≈"
                                elif band_pattern == 1:
                                    grid[y][x] = "≈"
                                else:
                                    grid[y][x] = "▓" if (y % 2) == 0 else " "
                            else:
                                grid[y][x] = "░" if dist > jupiter_radius - 1 else " "

    # Saturn encounter: 25-60s
    if elapsed >= 25:
        saturn_progress = (elapsed - 25) / 35
        saturn_x = int(width - saturn_progress * (width - probe_x - 30))
        saturn_y = height // 2 + 3
        saturn_radius = 10

        # Draw Saturn if on screen
        if saturn_x < width + saturn_radius + 15:
            # Draw the planet
            for dy in range(-saturn_radius, saturn_radius + 1):
                for dx in range(
                    -int(saturn_radius * 2.5), int(saturn_radius * 2.5) + 1
                ):
                    dist = math.sqrt((dx / 2.5) ** 2 + dy**2)
                    if dist <= saturn_radius:
                        y, x = saturn_y + dy, saturn_x + dx
                        if 0 <= y < height and 0 <= x < width:
                            # Saturn's pale color
                            if dist < saturn_radius - 1:
                                grid[y][x] = "◎" if (x + y) % 3 == 0 else " "
                            else:
                                grid[y][x] = "░"

            # Draw rings
            ring_inner = saturn_radius + 2
            ring_outer = saturn_radius + 8
            for ring_dist in range(ring_inner, ring_outer + 1):
                # Gaps in rings (Cassini division)
                if ring_dist == saturn_radius + 5:
                    continue

                for angle in range(0, 360, 3):
                    rad = math.radians(angle)
                    # Rings appear as ellipse due to viewing angle
                    ring_x = saturn_x + int(ring_dist * math.cos(rad) * 2.5)
                    ring_y = saturn_y + int(
                        ring_dist * math.sin(rad) * 0.3
                    )  # Compressed

                    if 0 <= ring_y < height and 0 <= ring_x < width:
                        # Rings behind planet are dimmer
                        dx_to_planet = ring_x - saturn_x
                        dy_to_planet = (
                            ring_y - saturn_y
                        ) * 3.3  # Adjust for compression
                        if (
                            math.sqrt((dx_to_planet / 2.5) ** 2 + dy_to_planet**2)
                            > saturn_radius
                        ):
                            # In front of planet
                            char = "≈"
                            grid[ring_y][ring_x] = char
                        else:
                            # Behind planet (dimmer)
                            if grid[ring_y][ring_x] in ["░", "◎", " "]:
                                grid[ring_y][ring_x] = "·"

    # Draw Voyager probe - using approved characters
    probe_art = [
        "  │  ",
        " ═╬═ ",
        "  ◇  ",
        " ═╬═ ",
        "  │  "
    ]

    for i, line in enumerate(probe_art):
        y = probe_y + i - len(probe_art) // 2
        if 0 <= y < height:
            for j, char in enumerate(line):
                x = probe_x + j - len(line) // 2
                if 0 <= x < width and char != " ":
                    grid[y][x] = char

    # Draw trajectory path (gravity assist curve) - sparser
    path_length = 25
    for i in range(0, path_length, 2):  # Every 2nd point for sparseness
        # Curved path showing gravity assist
        path_progress = i / path_length

        # Path curves during planetary encounters
        if elapsed < 20:
            # Approaching Jupiter
            curve = math.sin(path_progress * math.pi) * 3
        elif elapsed < 35:
            # Jupiter flyby
            curve = -math.sin((elapsed - 20) / 15 * math.pi) * 5
        else:
            # Approaching Saturn
            curve = math.sin((elapsed - 35) / 25 * math.pi) * 4

        path_x = probe_x + i
        path_y = probe_y + int(curve)

        if 0 <= path_y < height and 0 <= path_x < width:
            if grid[path_y][path_x] == " ":
                grid[path_y][path_x] = "·"

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
