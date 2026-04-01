import random
import math
import time
from colorama import Fore, Style  # Adding color support


def generate(width, height):
    """Enhanced gravitational lensing with multiple lenses, Einstein rings, crosses, and arcs.

    Features:
    - Multiple lensing masses (galaxy cluster)
    - Animated background quasar showing time-variable lensing
    - Einstein rings for perfect alignment
    - Einstein Cross configuration
    - Realistic gravitational arcs
    - Magnification and distortion effects
    """
    grid = [[" " for _ in range(width)] for _ in range(height)]

    # Primary lens at center
    lens_center_x, lens_center_y = width // 2, height // 2

    # Multiple lensing masses (galaxy cluster)
    if not hasattr(generate, "lenses"):
        generate.lenses = [
            {"x": lens_center_x, "y": lens_center_y, "mass": 100, "einstein_r": 12},
            {
                "x": lens_center_x + 15,
                "y": lens_center_y - 8,
                "mass": 40,
                "einstein_r": 6,
            },
            {
                "x": lens_center_x - 12,
                "y": lens_center_y + 10,
                "mass": 35,
                "einstein_r": 5,
            },
        ]

    # Generate background sources
    if not hasattr(generate, "sources"):
        generate.sources = []

        # Perfectly aligned quasar for primary Einstein ring
        generate.sources.append(
            {
                "x": lens_center_x,
                "y": lens_center_y,
                "type": "quasar",
                "brightness": 5,
                "size": 1,
            }
        )

        # Quasar for Einstein Cross (4-image configuration)
        generate.sources.append(
            {
                "x": lens_center_x + 3,
                "y": lens_center_y + 2,
                "type": "quasar",
                "brightness": 4,
                "size": 1,
            }
        )

        # Extended background galaxies
        for _ in range(15):
            generate.sources.append(
                {
                    "x": random.uniform(width * 0.2, width * 0.8),
                    "y": random.uniform(height * 0.2, height * 0.8),
                    "type": "galaxy",
                    "brightness": random.randint(2, 3),
                    "size": random.randint(2, 4),
                    "orientation": random.uniform(0, math.pi),
                }
            )

        # Point sources (distant stars/quasars)
        for _ in range(100):
            generate.sources.append(
                {
                    "x": random.uniform(0, width),
                    "y": random.uniform(0, height),
                    "type": "star",
                    "brightness": random.randint(1, 2),
                    "size": 1,
                }
            )

    t = time.time()

    # Add quasar variability (brightness changes)
    quasar_brightness_mod = 1.0 + 0.3 * math.sin(t * 0.5)

    # Draw faint background stars (unlensed, very distant)
    for source in generate.sources:
        if source["type"] == "star":
            sx, sy = int(source["x"]), int(source["y"])
            # Check if far from all lenses (minimal lensing)
            min_dist_to_lens = min(
                math.sqrt(
                    (source["x"] - lens["x"]) ** 2 + (source["y"] - lens["y"]) ** 2
                )
                for lens in generate.lenses
            )
            if min_dist_to_lens > 25:  # Far from lensing region
                if 0 <= sy < height and 0 <= sx < width:
                    grid[sy][sx] = (
                        Fore.LIGHTBLACK_EX + "·" + Style.RESET_ALL
                        if source["brightness"] == 1
                        else Fore.WHITE + "*" + Style.RESET_ALL
                    )

    # Process lensing for each source
    for source_idx, source in enumerate(generate.sources):
        if source["type"] in ["quasar", "galaxy"]:
            sx, sy = source["x"], source["y"]

            # Calculate lensing effect from all masses
            lensed_positions = []

            for lens in generate.lenses:
                dx = sx - lens["x"]
                dy = sy - lens["y"]
                dist = math.sqrt(dx**2 + dy**2)

                if dist < 0.5:
                    continue

                # Primary lens creates ring for aligned source
                if dist < 3 and source_idx == 0:  # Perfectly aligned quasar
                    # Draw Einstein ring
                    for angle in range(0, 360, 2):
                        rad = math.radians(angle)
                        ring_r = lens["einstein_r"] * (
                            1.0 + 0.05 * math.sin(t + angle / 10)
                        )  # Subtle pulsing
                        ring_x = lens["x"] + int(ring_r * math.cos(rad) * 2.5)
                        ring_y = lens["y"] + int(ring_r * math.sin(rad))
                        if 0 <= ring_y < height and 0 <= ring_x < width:
                            brightness_char = (
                                Fore.YELLOW + "#" + Style.RESET_ALL
                                if quasar_brightness_mod > 1.1
                                else Fore.LIGHTYELLOW_EX + "*" + Style.RESET_ALL
                            )
                            if grid[ring_y][ring_x] == " ":
                                grid[ring_y][ring_x] = brightness_char

                # Einstein Cross configuration (4 images)
                elif dist < 8 and source_idx == 1:  # Slightly offset quasar
                    angle = math.atan2(dy, dx)
                    # Create 4 images at cardinal directions
                    for img_angle in [0, math.pi / 2, math.pi, 3 * math.pi / 2]:
                        img_r = lens["einstein_r"] * 0.8
                        img_x = lens["x"] + int(
                            img_r * math.cos(angle + img_angle) * 2.5
                        )
                        img_y = lens["y"] + int(img_r * math.sin(angle + img_angle))
                        if 0 <= img_y < height and 0 <= img_x < width:
                            grid[img_y][img_x] = Fore.MAGENTA + "✦" + Style.RESET_ALL

                # Gravitational arcs from extended galaxies
                elif source["type"] == "galaxy" and dist < lens["einstein_r"] * 2:
                    angle = math.atan2(dy, dx)
                    # Create arc
                    arc_length = 60  # degrees
                    for arc_angle in range(-arc_length // 2, arc_length // 2, 3):
                        arc_rad = math.radians(arc_angle)
                        arc_r = lens["einstein_r"] * (1.0 + random.uniform(-0.2, 0.2))
                        arc_x = lens["x"] + int(arc_r * math.cos(angle + arc_rad) * 2.5)
                        arc_y = lens["y"] + int(arc_r * math.sin(angle + arc_rad))
                        if 0 <= arc_y < height and 0 <= arc_x < width:
                            if grid[arc_y][arc_x] in [" ", Fore.LIGHTBLACK_EX + "·"]:
                                grid[arc_y][arc_x] = (
                                    random.choice(
                                        [
                                            Fore.CYAN + "~",
                                            Fore.BLUE + "≈",
                                            Fore.LIGHTCYAN_EX + "∼",
                                        ]
                                    )
                                    + Style.RESET_ALL
                                )

                # Standard lensing (magnified and demagnified images)
                if 3 < dist < lens["einstein_r"] * 3:
                    angle = math.atan2(dy, dx)
                    deflection = (
                        (lens["einstein_r"] ** 2) / (dist**2) if dist > 0 else 0
                    )

                    # Magnified image (outer)
                    r_mag = dist * (1 + deflection * 0.5)
                    lx_mag = lens["x"] + int(r_mag * math.cos(angle) * 2.5)
                    ly_mag = lens["y"] + int(r_mag * math.sin(angle))
                    if 0 <= ly_mag < height and 0 <= lx_mag < width:
                        lensed_positions.append(
                            (
                                lx_mag,
                                ly_mag,
                                Fore.LIGHTYELLOW_EX + "+" + Style.RESET_ALL,
                            )
                        )

                    # Demagnified image (inner)
                    if dist < lens["einstein_r"] * 2:
                        r_demag = dist * (1 - deflection * 0.3)
                        if r_demag > 2:
                            lx_demag = lens["x"] + int(r_demag * math.cos(angle) * 2.5)
                            ly_demag = lens["y"] + int(r_demag * math.sin(angle))
                            if 0 <= ly_demag < height and 0 <= lx_demag < width:
                                lensed_positions.append(
                                    (
                                        lx_demag,
                                        ly_demag,
                                        Fore.LIGHTBLACK_EX + "·" + Style.RESET_ALL,
                                    )
                                )

            # Draw lensed images
            for lx, ly, char in lensed_positions:
                if grid[ly][lx] in [" ", Fore.LIGHTBLACK_EX + "·"]:
                    grid[ly][lx] = char

    # Draw lensing galaxy cluster
    for lens in generate.lenses:
        lx, ly = int(lens["x"]), int(lens["y"])
        size = int(lens["mass"] / 20)

        for dy in range(-size, size + 1):
            for dx in range(-int(size * 2.5), int(size * 2.5) + 1):
                dist = math.sqrt((dx / 2.5) ** 2 + dy**2)
                if dist <= size:
                    y, x = ly + dy, lx + dx
                    if 0 <= y < height and 0 <= x < width:
                        if dist < size * 0.4:
                            grid[y][x] = Fore.YELLOW + "█" + Style.RESET_ALL
                        elif dist < size * 0.7:
                            grid[y][x] = Fore.LIGHTYELLOW_EX + "▓" + Style.RESET_ALL
                        elif dist < size * 0.9:
                            grid[y][x] = Fore.LIGHTBLACK_EX + "▒" + Style.RESET_ALL
                        else:
                            if grid[y][x] == " ":
                                grid[y][x] = Fore.LIGHTBLACK_EX + "░" + Style.RESET_ALL

    # Draw critical curves (caustics) - regions of infinite magnification
    primary_lens = generate.lenses[0]
    for angle in range(0, 360, 8):
        rad = math.radians(angle)
        # Critical curve at Einstein radius
        crit_r = primary_lens["einstein_r"] * 0.95
        crit_x = primary_lens["x"] + int(crit_r * math.cos(rad) * 2.5)
        crit_y = primary_lens["y"] + int(crit_r * math.sin(rad))
        if 0 <= crit_y < height and 0 <= crit_x < width:
            if grid[crit_y][crit_x] in [
                " ",
                Fore.LIGHTBLACK_EX + "·",
                Fore.LIGHTBLACK_EX + "░",
            ]:
                grid[crit_y][crit_x] = Fore.LIGHTCYAN_EX + "○" + Style.RESET_ALL

    # Info panel
    info_lines = [
        "Gravitational Lensing - Galaxy Cluster",
        f"{Fore.YELLOW}█{Style.RESET_ALL} = Lensing galaxies | {Fore.CYAN}~{Style.RESET_ALL} = Arcs | {Fore.MAGENTA}✦{Style.RESET_ALL} = Einstein Cross | {Fore.LIGHTCYAN_EX}○{Style.RESET_ALL} = Critical curve",
        f"Quasar brightness: {quasar_brightness_mod:.2f}x",
    ]

    for idx, line in enumerate(info_lines):
        if idx < height - 3:
            for i, char in enumerate(line):
                if i < width:
                    grid[height - 3 + idx][i] = char

    return "\n".join(["".join(row) for row in grid])
