import random
import math
import time
from colorama import Fore, Style  # Adding color support


def generate(width, height):
    """Enhanced black hole with lensing, Hawking radiation, and frame dragging effects."""
    grid = [[" " for _ in range(width)] for _ in range(height)]

    bh_x, bh_y = width // 2, height // 2
    event_horizon = 4
    isco = 6  # Innermost stable circular orbit
    ergosphere = 7
    t = time.time()

    # Initialize particles for accretion disk
    if not hasattr(generate, "particles"):
        generate.particles = []
        for _ in range(800):
            angle = random.uniform(0, 2 * math.pi)
            distance = random.uniform(isco, event_horizon + 20)
            # Keplerian velocity: v ∝ 1/√r
            speed = 1.5 / math.sqrt(distance)
            # Random vertical displacement for 3D effect
            z = random.uniform(-2, 2)
            # Temperature (brightness) increases closer to black hole
            temp_factor = 1.0 / (distance / isco)
            brightness = min(3, int(temp_factor * 2))
            generate.particles.append(
                {
                    "angle": angle,
                    "distance": distance,
                    "speed": speed,
                    "z": z,
                    "brightness": brightness,
                }
            )

        # Background stars for lensing demonstration
        generate.background_stars = []
        for _ in range(100):
            generate.background_stars.append(
                {
                    "x": random.uniform(0, width),
                    "y": random.uniform(0, height),
                    "brightness": random.choice([0, 1, 2]),
                }
            )

    # Draw background stars with gravitational lensing
    for star in generate.background_stars:
        sx, sy = star["x"], star["y"]
        dx = sx - bh_x
        dy = sy - bh_y
        dist = math.sqrt(dx**2 + dy**2)

        # Stars close to black hole get lensed (bent light)
        if dist > event_horizon * 2 and dist < ergosphere * 4:
            # Light bending angle
            deflection_angle = (event_horizon / dist) ** 2 * 10
            angle = math.atan2(dy, dx)

            # Lensed positions (can create multiple images)
            for lens_mult in [1, -0.5]:  # Primary and secondary image
                lensed_dist = dist * (1 + deflection_angle * lens_mult)
                lensed_x = bh_x + int(lensed_dist * math.cos(angle) / 2.5)
                lensed_y = bh_y + int(lensed_dist * math.sin(angle))

                if 0 <= lensed_y < height and 0 <= lensed_x < width:
                    char = [Fore.BLUE + "·", Fore.CYAN + "*", Fore.WHITE + "✦"][
                        star["brightness"]
                    ] + Style.RESET_ALL
                    if grid[lensed_y][lensed_x] == " ":
                        grid[lensed_y][lensed_x] = char
        elif dist >= ergosphere * 4:
            # Distant stars appear normal
            ix, iy = int(sx), int(sy)
            if 0 <= iy < height and 0 <= ix < width:
                char = [Fore.BLUE + ".", Fore.CYAN + "·", Fore.WHITE + "*"][
                    star["brightness"]
                ] + Style.RESET_ALL
                if grid[iy][ix] == " ":
                    grid[iy][ix] = char

    # Update particle positions with frame dragging
    for particle in generate.particles:
        # Frame dragging: spacetime itself rotates near BH
        frame_drag = 0.05 * (event_horizon / particle["distance"]) ** 2
        particle["angle"] += particle["speed"] + frame_drag

        # Slowly spiral inward
        if random.random() < 0.001:
            particle["distance"] *= 0.99
            # Reset if too close to ISCO
            if particle["distance"] < isco + 1:
                particle["distance"] = event_horizon + 20
                particle["angle"] = random.uniform(0, 2 * math.pi)
                # Re-calculate brightness
                temp_factor = 1.0 / (particle["distance"] / isco)
                particle["brightness"] = min(3, int(temp_factor * 2))

    # Draw photon sphere and ergosphere
    for angle in range(0, 360, 3):
        rad = math.radians(angle)
        # Photon sphere (where light orbits)
        photon_r = event_horizon * 1.5
        x = bh_x + int(photon_r * math.cos(rad) * 2.5)
        y = bh_y + int(photon_r * math.sin(rad))
        if 0 <= y < height and 0 <= x < width:
            if grid[y][x] in [" ", "."]:
                grid[y][x] = Fore.YELLOW + "○" + Style.RESET_ALL

        # Ergosphere boundary
        for r in range(ergosphere, ergosphere + 2):
            x = bh_x + int(r * math.cos(rad) * 2.5)
            y = bh_y + int(r * math.sin(rad))
            if 0 <= y < height and 0 <= x < width:
                if grid[y][x] in [" ", "."]:
                    grid[y][x] = (
                        Fore.MAGENTA + ":"
                        if r == ergosphere
                        else Fore.LIGHTMAGENTA_EX + "."
                    ) + Style.RESET_ALL

    # Draw accretion disk particles
    chars = [
        Fore.LIGHTBLACK_EX + ".",
        Fore.YELLOW + ":",
        Fore.LIGHTYELLOW_EX + "*",
        Fore.RED + "#",
    ]
    for p in generate.particles:
        # Apply 3D perspective (particles with higher z appear dimmer)
        if abs(p["z"]) < 1.5:  # Only show particles near the viewing plane
            x = bh_x + int(p["distance"] * math.cos(p["angle"]) * 2.5)
            y = bh_y + int(p["distance"] * math.sin(p["angle"]))

            if 0 <= y < height and 0 <= x < width:
                # Brightness affected by height (z) and intrinsic brightness
                brightness = max(0, p["brightness"] - int(abs(p["z"])))
                grid[y][x] = chars[min(brightness, 3)] + Style.RESET_ALL

    # Draw event horizon with Hawking radiation
    for dy in range(-event_horizon, event_horizon + 1):
        for dx in range(-int(event_horizon * 2.5), int(event_horizon * 2.5) + 1):
            dist = math.sqrt((dx / 2.5) ** 2 + dy**2)
            if dist <= event_horizon:
                y, x = bh_y + dy, bh_x + dx
                if 0 <= y < height and 0 <= x < width:
                    grid[y][x] = Fore.RED + "█" + Style.RESET_ALL
            elif dist <= event_horizon + 0.5:
                # Hawking radiation - faint glow at horizon
                y, x = bh_y + dy, bh_x + dx
                if 0 <= y < height and 0 <= x < width:
                    if random.random() < 0.3:  # Quantum fluctuations
                        grid[y][x] = (
                            random.choice(
                                [
                                    Fore.LIGHTBLACK_EX + "░",
                                    Fore.LIGHTBLACK_EX + "▒",
                                    Fore.LIGHTBLACK_EX + ".",
                                ]
                            )
                            + Style.RESET_ALL
                        )

    # Draw relativistic jets shooting from poles
    jet_length = min(height // 2 - 5, 20)
    jet_chars = [
        Fore.RED + "#",
        Fore.YELLOW + "=",
        Fore.LIGHTYELLOW_EX + ":",
        Fore.LIGHTBLACK_EX + ".",
    ]

    for direction in [-1, 1]:  # Top and bottom jets
        for dist in range(event_horizon + 2, jet_length):
            # Jet width decreases slightly with distance
            jet_width = max(1, int(3 - dist * 0.1))
            # Pulsing effect
            pulse = math.sin(t * 3 + dist * 0.3) * 0.5 + 0.5

            for offset in range(-jet_width, jet_width + 1):
                y = bh_y + direction * dist
                x = bh_x + int(offset * 2.5)

                if 0 <= y < height and 0 <= x < width:
                    # Intensity based on distance and pulse
                    intensity = max(0, min(3, int((1 - dist / jet_length) * 3 * pulse)))
                    if intensity > 0:
                        char = jet_chars[3 - intensity] + Style.RESET_ALL
                        if grid[y][x] == " ":
                            grid[y][x] = char

    # Add some jet turbulence particles
    if not hasattr(generate, "jet_particles"):
        generate.jet_particles = []

    # Spawn jet particles
    if random.random() < 0.3:
        for direction in [-1, 1]:
            generate.jet_particles.append(
                {
                    "x": bh_x,
                    "y": bh_y + direction * (event_horizon + 3),
                    "vy": direction * random.uniform(0.5, 1.2),
                    "vx": random.uniform(-0.3, 0.3),
                    "age": 0,
                }
            )

    # Update and draw jet particles
    particles_to_remove = []
    for jp in generate.jet_particles:
        jp["x"] += jp["vx"]
        jp["y"] += jp["vy"]
        jp["age"] += 1

        if (
            jp["age"] > 40
            or jp["y"] < 0
            or jp["y"] >= height
            or jp["x"] < 0
            or jp["x"] >= width
        ):
            particles_to_remove.append(jp)
        else:
            jx, jy = int(jp["x"]), int(jp["y"])
            if 0 <= jy < height and 0 <= jx < width and grid[jy][jx] == " ":
                grid[jy][jx] = Fore.LIGHTCYAN_EX + "+" + Style.RESET_ALL

    for jp in particles_to_remove:
        generate.jet_particles.remove(jp)

    # Info panel
    info_lines = [
        "Rotating Black Hole (Kerr)",
        Fore.RED
        + "█"
        + Style.RESET_ALL
        + "=Event Horizon | "
        + Fore.LIGHTBLACK_EX
        + "░"
        + Style.RESET_ALL
        + "=Hawking Radiation | "
        + Fore.YELLOW
        + "○"
        + Style.RESET_ALL
        + "=Photon Sphere | "
        + Fore.MAGENTA
        + ":"
        + Style.RESET_ALL
        + "=Ergosphere",
        f"Frame Dragging: Active | ISCO: {isco} units | Jets: Relativistic",
    ]

    for idx, line in enumerate(info_lines):
        if height - 3 + idx < height:
            for i, char in enumerate(line):
                if i < width:
                    grid[height - 3 + idx][i] = char

    return "\n".join(["".join(row) for row in grid])
