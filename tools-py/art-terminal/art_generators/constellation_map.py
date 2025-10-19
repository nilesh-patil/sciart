import random
import time
import math
from colorama import Fore, Style, init

init(autoreset=True)

# Enhanced constellations with star magnitudes, names, and deep-sky objects
CONSTELLATIONS = {
    "Orion": {
        "stars": [
            (0.5, 0.25, 0.0, "Betelgeuse", "red"),  # Red supergiant
            (0.65, 0.7, 0.3, "Rigel", "blue"),  # Blue supergiant
            (0.45, 0.5, 1.8, "Alnitak", "blue"),  # Belt left
            (0.5, 0.52, 2.0, "Alnilam", "blue"),  # Belt center
            (0.55, 0.5, 1.7, "Mintaka", "blue"),  # Belt right
            (0.42, 0.35, 2.2, "Bellatrix", "blue"),  # Left shoulder
            (0.48, 0.72, 2.8, "Saiph", "blue"),  # Left foot
            (0.58, 0.35, 3.0, None, "white"),  # Right shoulder
            (0.62, 0.72, 3.0, None, "white"),  # Right foot
        ],
        "lines": [
            (0, 5),
            (5, 2),
            (2, 3),
            (3, 4),
            (4, 7),
            (7, 1),
            (1, 8),
            (8, 4),
            (2, 6),
            (4, 1),
        ],
        "deep_sky": [(0.5, 0.62, "M42", "nebula")],  # Orion Nebula
        "myth": "The Hunter",
        "season": "winter",
    },
    "Ursa Major": {
        "stars": [
            (0.2, 0.3, 1.8, "Dubhe", "orange"),
            (0.3, 0.25, 2.3, "Merak", "white"),
            (0.42, 0.35, 2.4, "Phecda", "white"),
            (0.52, 0.3, 2.4, "Megrez", "white"),
            (0.62, 0.35, 1.8, "Alioth", "white"),
            (0.72, 0.32, 2.2, "Mizar", "white"),
            (0.82, 0.38, 1.9, "Alkaid", "blue"),
        ],
        "lines": [(0, 1), (1, 2), (2, 3), (3, 4), (4, 5), (5, 6), (3, 0)],
        "deep_sky": [(0.65, 0.55, "M81", "galaxy"), (0.68, 0.6, "M82", "galaxy")],
        "myth": "The Great Bear",
        "season": "spring",
    },
    "Cassiopeia": {
        "stars": [
            (0.15, 0.5, 2.2, "Caph", "white"),
            (0.3, 0.35, 2.3, "Schedar", "orange"),
            (0.5, 0.45, 2.5, "Navi", "blue"),
            (0.7, 0.32, 2.7, "Ruchbah", "white"),
            (0.85, 0.5, 3.4, "Segin", "blue"),
        ],
        "lines": [(0, 1), (1, 2), (2, 3), (3, 4)],
        "deep_sky": [(0.45, 0.6, "M52", "cluster")],
        "myth": "The Queen",
        "season": "autumn",
    },
    "Scorpius": {
        "stars": [
            (0.5, 0.4, 0.9, "Antares", "red"),  # Red supergiant
            (0.45, 0.35, 2.6, None, "white"),
            (0.52, 0.3, 2.3, "Dschubba", "blue"),
            (0.58, 0.38, 2.9, None, "white"),
            (0.63, 0.48, 2.3, "Shaula", "blue"),
            (0.68, 0.58, 1.6, None, "blue"),
            (0.72, 0.68, 2.7, None, "white"),
            (0.75, 0.75, 3.0, None, "white"),
        ],
        "lines": [(0, 1), (1, 2), (2, 3), (3, 4), (4, 5), (5, 6), (6, 7)],
        "deep_sky": [(0.6, 0.5, "M4", "cluster")],
        "myth": "The Scorpion",
        "season": "summer",
    },
    "Cygnus": {
        "stars": [
            (0.5, 0.2, 1.3, "Deneb", "white"),  # Northern Cross head
            (0.5, 0.5, 2.2, "Sadr", "yellow"),  # Center
            (0.5, 0.75, 1.3, "Albireo", "orange"),  # Foot (double star)
            (0.25, 0.5, 2.5, None, "white"),  # Left wing
            (0.75, 0.5, 2.9, None, "white"),  # Right wing
        ],
        "lines": [(0, 1), (1, 2), (1, 3), (1, 4)],
        "deep_sky": [(0.6, 0.45, "Veil", "nebula")],
        "myth": "The Swan",
        "season": "summer",
    },
    "Leo": {
        "stars": [
            (0.4, 0.5, 1.4, "Regulus", "blue"),  # Heart of the Lion
            (0.5, 0.45, 2.6, None, "white"),
            (0.6, 0.4, 2.0, "Denebola", "white"),
            (0.65, 0.3, 2.1, None, "white"),
            (0.7, 0.35, 3.4, None, "white"),
            (0.35, 0.35, 2.3, "Algieba", "orange"),
            (0.3, 0.45, 3.5, None, "white"),
        ],
        "lines": [(0, 1), (1, 2), (2, 3), (3, 4), (2, 5), (5, 6), (6, 0)],
        "deep_sky": [(0.55, 0.55, "M65", "galaxy"), (0.58, 0.58, "M66", "galaxy")],
        "myth": "The Lion",
        "season": "spring",
    },
    "Andromeda": {
        "stars": [
            (0.35, 0.4, 2.1, "Alpheratz", "blue"),
            (0.5, 0.35, 2.0, "Mirach", "orange"),
            (0.65, 0.45, 2.3, "Almach", "orange"),
            (0.48, 0.25, 3.3, None, "white"),
        ],
        "lines": [(0, 1), (1, 2), (1, 3)],
        "deep_sky": [(0.52, 0.3, "M31", "galaxy")],  # Andromeda Galaxy
        "myth": "The Princess",
        "season": "autumn",
    },
    "Aquila": {
        "stars": [
            (0.5, 0.5, 0.8, "Altair", "white"),  # One of summer triangle
            (0.45, 0.42, 2.7, "Tarazed", "orange"),
            (0.55, 0.42, 2.7, "Alshain", "white"),
            (0.42, 0.6, 3.2, None, "white"),
            (0.58, 0.6, 3.2, None, "white"),
        ],
        "lines": [(1, 0), (0, 2), (0, 3), (0, 4)],
        "deep_sky": [],
        "myth": "The Eagle",
        "season": "summer",
    },
    "Lyra": {
        "stars": [
            (0.5, 0.3, 0.0, "Vega", "blue"),  # Brightest in northern sky
            (0.42, 0.5, 3.2, None, "white"),
            (0.58, 0.5, 3.2, None, "white"),
            (0.48, 0.6, 4.3, None, "white"),
            (0.52, 0.6, 4.3, None, "white"),
        ],
        "lines": [(0, 1), (0, 2), (1, 3), (2, 4), (3, 4)],
        "deep_sky": [(0.55, 0.45, "M57", "nebula")],  # Ring Nebula
        "myth": "The Lyre",
        "season": "summer",
    },
    "Taurus": {
        "stars": [
            (0.4, 0.55, 0.9, "Aldebaran", "orange"),  # Eye of the bull
            (0.5, 0.4, 2.9, None, "white"),
            (0.35, 0.35, 3.0, None, "blue"),
            (0.3, 0.45, 3.4, None, "blue"),  # Pleiades region
            (0.55, 0.6, 1.7, "Elnath", "blue"),
        ],
        "lines": [(0, 1), (0, 2), (0, 3), (1, 4)],
        "deep_sky": [
            (0.28, 0.38, "M45", "cluster"),
            (0.42, 0.52, "M1", "nebula"),
        ],  # Pleiades & Crab
        "myth": "The Bull",
        "season": "winter",
    },
    "Gemini": {
        "stars": [
            (0.35, 0.3, 1.6, "Pollux", "orange"),
            (0.45, 0.25, 1.9, "Castor", "white"),
            (0.32, 0.55, 3.0, None, "white"),
            (0.42, 0.6, 2.9, None, "white"),
            (0.38, 0.7, 3.5, None, "white"),
            (0.48, 0.68, 3.3, None, "white"),
        ],
        "lines": [(0, 1), (0, 2), (2, 4), (1, 3), (3, 5)],
        "deep_sky": [(0.4, 0.45, "M35", "cluster")],
        "myth": "The Twins",
        "season": "winter",
    },
    "Pegasus": {
        "stars": [
            (0.25, 0.25, 2.4, "Markab", "blue"),
            (0.7, 0.25, 2.8, "Scheat", "red"),
            (0.7, 0.7, 2.4, "Algenib", "blue"),
            (0.25, 0.7, 2.5, "Alpheratz", "blue"),
        ],
        "lines": [(0, 1), (1, 2), (2, 3), (3, 0)],  # Great Square
        "deep_sky": [(0.48, 0.48, "M15", "cluster")],
        "myth": "The Winged Horse",
        "season": "autumn",
    },
}

# Define color mappings for stars and objects
COLOR_MAP = {
    "red": Fore.RED,
    "blue": Fore.BLUE,
    "white": Fore.WHITE,
    "orange": Fore.YELLOW,
    "yellow": Fore.LIGHTYELLOW_EX,
    "nebula": Fore.MAGENTA,
    "galaxy": Fore.CYAN,
    "cluster": Fore.GREEN,
}


def draw_line(grid, x1, y1, x2, y2):
    """Draws a line on the grid using Bresenham's line algorithm."""
    dx = abs(x2 - x1)
    dy = abs(y2 - y1)
    sx = 1 if x1 < x2 else -1
    sy = 1 if y1 < y2 else -1
    err = dx - dy

    while True:
        if 0 <= y1 < len(grid) and 0 <= x1 < len(grid[0]):
            if grid[y1][x1] == " ":
                grid[y1][x1] = "."
        if x1 == x2 and y1 == y2:
            break
        e2 = 2 * err
        if e2 > -dy:
            err -= dy
            x1 += sx
        if e2 < dx:
            err += dx
            y1 += sy


def generate(grid_width, grid_height):
    """Generates an enhanced constellation map with magnitudes, colors, and deep-sky objects."""
    grid = [[" " for _ in range(grid_width)] for _ in range(grid_height)]

    # Initialize background stars with varied brightness
    if not hasattr(generate, "background_stars"):
        generate.background_stars = []
        for _ in range(int(grid_width * grid_height * 0.03)):
            generate.background_stars.append(
                {
                    "x": random.randint(0, grid_width - 1),
                    "y": random.randint(0, grid_height - 1),
                    "brightness": random.choice([0, 0, 0, 1, 1, 2]),  # Mostly faint
                    "twinkle_phase": random.random() * 6.28,
                }
            )

        # Add Milky Way band (denser star region)
        milky_way_center_y = grid_height // 2
        for _ in range(int(grid_width * grid_height * 0.02)):
            # Gaussian distribution around MW band
            y_offset = int(random.gauss(0, grid_height * 0.15))
            generate.background_stars.append(
                {
                    "x": random.randint(0, grid_width - 1),
                    "y": max(0, min(grid_height - 1, milky_way_center_y + y_offset)),
                    "brightness": random.choice([0, 0, 1, 1, 2]),
                    "twinkle_phase": random.random() * 6.28,
                }
            )

    t = time.time()

    # Draw background stars with twinkling
    bg_chars = [".", "·", "∙"]
    for star in generate.background_stars:
        x, y = star["x"], star["y"]
        if 0 <= y < grid_height and 0 <= x < grid_width:
            # Subtle twinkling
            twinkle = 0.7 + 0.3 * math.sin(t * 2 + star["twinkle_phase"])
            if random.random() < twinkle:
                grid[y][x] = bg_chars[min(star["brightness"], 2)]

    # Choose constellation (change less frequently)
    if not hasattr(generate, "constellation_name") or random.random() < 0.005:
        generate.constellation_name = random.choice(list(CONSTELLATIONS.keys()))

    constellation = CONSTELLATIONS[generate.constellation_name]

    # Draw constellation lines first (so stars appear on top)
    star_coords = []
    for star_data in constellation["stars"]:
        x_norm, y_norm = star_data[0], star_data[1]
        x = int(x_norm * grid_width)
        y = int(y_norm * grid_height)
        star_coords.append((x, y))

    for start_idx, end_idx in constellation["lines"]:
        if start_idx < len(star_coords) and end_idx < len(star_coords):
            x1, y1 = star_coords[start_idx]
            x2, y2 = star_coords[end_idx]
            draw_line(grid, x1, y1, x2, y2)

    # Draw constellation stars with magnitude-based size and color indicators
    for idx, star_data in enumerate(constellation["stars"]):
        x_norm, y_norm, magnitude, name, color = star_data
        x, y = star_coords[idx]

        if 0 <= y < grid_height and 0 <= x < grid_width:
            # Brightness based on magnitude (lower = brighter)
            if magnitude < 1.5:
                # Very bright stars get larger representation
                main_char = (
                    COLOR_MAP[color]
                    + ("✦" if color == "blue" else ("◉" if color == "red" else "★"))
                    + Style.RESET_ALL
                )
                grid[y][x] = main_char

                # Add glow around bright stars
                for dy, dx in [(-1, 0), (1, 0), (0, -1), (0, 1)]:
                    ny, nx = y + dy, x + dx
                    if (
                        0 <= ny < grid_height
                        and 0 <= nx < grid_width
                        and grid[ny][nx] in [" ", ".", "·"]
                    ):
                        grid[ny][nx] = COLOR_MAP[color] + "·" + Style.RESET_ALL

                # Add twinkling to very bright stars
                twinkle = math.sin(t * 3 + idx)
                if twinkle > 0.5:
                    for dy, dx in [(-1, -1), (-1, 1), (1, -1), (1, 1)]:
                        ny, nx = y + dy, x + dx
                        if (
                            0 <= ny < grid_height
                            and 0 <= nx < grid_width
                            and grid[ny][nx] == " "
                        ):
                            grid[ny][nx] = COLOR_MAP[color] + "·" + Style.RESET_ALL

            elif magnitude < 2.5:
                # Bright stars
                grid[y][x] = (
                    COLOR_MAP[color]
                    + (
                        "★"
                        if color in ["white", "blue"]
                        else ("◉" if color == "red" else "◎")
                    )
                    + Style.RESET_ALL
                )
            else:
                # Dimmer stars
                grid[y][x] = (
                    COLOR_MAP[color]
                    + ("*" if color in ["white", "blue"] else "o")
                    + Style.RESET_ALL
                )

            # Draw star names for named bright stars
            if name and magnitude < 2.0:
                name_x = x + 2
                name_y = y
                if 0 <= name_y < grid_height and name_x + len(name) < grid_width:
                    for i, char in enumerate(name):
                        if grid[name_y][name_x + i] == " ":
                            grid[name_y][name_x + i] = (
                                COLOR_MAP[color] + char + Style.RESET_ALL
                            )

    # Draw deep-sky objects
    for dso in constellation["deep_sky"]:
        dso_x_norm, dso_y_norm, dso_name, dso_type = dso
        dso_x = int(dso_x_norm * grid_width)
        dso_y = int(dso_y_norm * grid_height)

        if dso_type == "nebula":
            # Draw nebula as diffuse cloud
            for dy in range(-2, 3):
                for dx in range(-4, 5):
                    dist = math.sqrt((dx / 2) ** 2 + dy**2)
                    if dist < 2.5:
                        ny, nx = dso_y + dy, dso_x + dx
                        if 0 <= ny < grid_height and 0 <= nx < grid_width:
                            if grid[ny][nx] in [" ", ".", "·"]:
                                grid[ny][nx] = (
                                    COLOR_MAP["nebula"]
                                    + random.choice(["░", "▒", "~", "≈"])
                                    + Style.RESET_ALL
                                )
        elif dso_type == "galaxy":
            # Draw galaxy as elongated ellipse
            for dy in range(-1, 2):
                for dx in range(-3, 4):
                    if abs(dy) + abs(dx) / 2 < 2:
                        ny, nx = dso_y + dy, dso_x + dx
                        if 0 <= ny < grid_height and 0 <= nx < grid_width:
                            if grid[ny][nx] in [" ", ".", "·"]:
                                grid[ny][nx] = (
                                    COLOR_MAP["galaxy"]
                                    + ("∴" if abs(dy) == 0 else "·")
                                    + Style.RESET_ALL
                                )
        elif dso_type == "cluster":
            # Draw cluster as tight group of stars
            for _ in range(8):
                dy = random.randint(-1, 1)
                dx = random.randint(-2, 2)
                ny, nx = dso_y + dy, dso_x + dx
                if 0 <= ny < grid_height and 0 <= nx < grid_width:
                    if grid[ny][nx] in [" ", ".", "·"]:
                        grid[ny][nx] = (
                            COLOR_MAP["cluster"]
                            + random.choice(["·", "*", "+"])
                            + Style.RESET_ALL
                        )

        # Label deep-sky object
        if dso_x + len(dso_name) + 1 < grid_width and dso_y + 3 < grid_height:
            label_y = dso_y + 3
            for i, char in enumerate(dso_name):
                if 0 <= label_y < grid_height and 0 <= dso_x + i < grid_width:
                    if grid[label_y][dso_x + i] == " ":
                        grid[label_y][dso_x + i] = (
                            COLOR_MAP["white"] + char + Style.RESET_ALL
                        )

    # Draw information panel at bottom
    name = generate.constellation_name
    myth = constellation["myth"]
    season = constellation["season"].capitalize()

    info_line = f"{name} - {myth} - {season}"
    info_x = grid_width // 2 - len(info_line) // 2
    info_y = grid_height - 2

    if 0 <= info_y < grid_height and info_x >= 0:
        for i, char in enumerate(info_line):
            x = info_x + i
            if 0 <= x < grid_width:
                grid[info_y][x] = COLOR_MAP["white"] + char + Style.RESET_ALL

    # Add celestial coordinate indicator
    legend = "✦=bright star  ░=nebula  ∴=galaxy"
    legend_x = grid_width // 2 - len(legend) // 2
    legend_y = grid_height - 1
    if 0 <= legend_y < grid_height and legend_x >= 0:
        for i, char in enumerate(legend):
            x = legend_x + i
            if 0 <= x < grid_width:
                grid[legend_y][x] = COLOR_MAP["white"] + char + Style.RESET_ALL

    return "\n".join(["".join(row) for row in grid])


if __name__ == "__main__":
    import os

    try:
        width, height = os.get_terminal_size()
    except OSError:
        width, height = 80, 24  # Default size

    while True:
        output = generate(width, height)
        os.system("clear" if os.name == "posix" else "cls")
        print(output)
        time.sleep(0.1)
