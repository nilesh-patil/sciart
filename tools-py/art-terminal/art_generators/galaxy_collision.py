import random
import math
from colorama import Fore, Style  # Adding color support


class Galaxy:
    def __init__(self, width, height, num_stars, x, y, vx, vy, rotation):
        self.width = width
        self.height = height
        self.num_stars = num_stars
        self.x = x
        self.y = y
        self.vx = vx
        self.vy = vy
        self.rotation = rotation
        self.stars = self._generate_spiral_stars()
        self.age = 0
        self.star_formation_burst = []  # Regions of new star formation
        self.tidal_tail_stars = []  # Stars pulled out into tidal tails

    def _generate_spiral_stars(self):
        """Generate stars in spiral arm pattern."""
        stars = []
        num_arms = 2
        for _ in range(self.num_stars):
            # Which spiral arm
            arm = random.randint(0, num_arms - 1)
            # Distance from center (exponential distribution for realistic galaxy)
            distance = abs(random.gauss(0, self.width / 10))
            # Angle around the spiral
            base_angle = (arm * 2 * math.pi / num_arms) + (distance * 0.3)
            angle = base_angle + random.gauss(0, 0.3)  # Add spread

            star_x = distance * math.cos(angle)
            star_y = distance * math.sin(angle)
            # Star brightness based on position
            brightness = random.choice([0, 1, 2, 3]) if distance < self.width / 6 else 0

            # Assign color based on brightness
            color = (
                [Fore.BLUE, Fore.CYAN, Fore.YELLOW, Fore.RED][brightness]
                if brightness > 0
                else ""
            )

            stars.append(
                {
                    "local_x": star_x,
                    "local_y": star_y,
                    "angle": angle,
                    "distance": distance,
                    "brightness": brightness,
                    "color": color,
                }
            )
        return stars

    def update(self, other_galaxy=None):
        """Update galaxy position with gravitational influence and tidal effects."""
        self.x += self.vx
        self.y += self.vy
        self.age += 1

        # Apply gravitational attraction if another galaxy is nearby
        if other_galaxy:
            dx = other_galaxy.x - self.x
            dy = other_galaxy.y - self.y
            dist = math.sqrt(dx**2 + dy**2) + 0.1

            if dist < self.width:  # Only if close enough
                # Gravitational force affects velocity
                force = 0.02 / (dist**2)
                self.vx += force * dx
                self.vy += force * dy

                # Tidal forces during close approach
                if dist < self.width * 0.6:
                    # Create tidal tail by pulling outer stars
                    for star in self.stars:
                        if star["distance"] > self.width / 8:  # Outer stars
                            if random.random() < 0.002:  # Occasional ejection
                                # Calculate tidal vector
                                star_angle = star["angle"] + self.age * self.rotation
                                star_world_x = self.x + star["distance"] * math.cos(
                                    star_angle
                                )
                                star_world_y = self.y + star["distance"] * math.sin(
                                    star_angle
                                )

                                # Velocity away from collision point
                                tidal_angle = math.atan2(dy, dx) + math.pi / 2
                                self.tidal_tail_stars.append(
                                    {
                                        "x": star_world_x,
                                        "y": star_world_y,
                                        "vx": math.cos(tidal_angle) * 0.5,
                                        "vy": math.sin(tidal_angle) * 0.5,
                                        "age": 0,
                                    }
                                )

                # Star formation burst during collision
                if dist < self.width * 0.4:
                    if random.random() < 0.05:
                        # Create new bright stars (O and B type)
                        burst_angle = random.uniform(0, 2 * math.pi)
                        burst_dist = random.uniform(0, self.width / 10)
                        self.star_formation_burst.append(
                            {
                                "x": self.x + burst_dist * math.cos(burst_angle),
                                "y": self.y + burst_dist * math.sin(burst_angle),
                                "brightness": 4,
                                "age": 0,
                                "lifetime": random.randint(30, 60),
                            }
                        )

        # Update tidal tail stars
        for tstar in self.tidal_tail_stars:
            tstar["x"] += tstar["vx"]
            tstar["y"] += tstar["vy"]
            tstar["age"] += 1

        # Update star formation regions (they fade over time)
        self.star_formation_burst = [
            sf for sf in self.star_formation_burst if sf["age"] < sf["lifetime"]
        ]
        for sf in self.star_formation_burst:
            sf["age"] += 1

    def get_star_positions(self):
        """Get current positions of all stars."""
        positions = []
        for star in self.stars:
            # Rotate galaxy slowly
            rotation_angle = self.age * self.rotation
            cos_r, sin_r = math.cos(rotation_angle), math.sin(rotation_angle)

            # Apply rotation
            rx = star["local_x"] * cos_r - star["local_y"] * sin_r
            ry = star["local_x"] * sin_r + star["local_y"] * cos_r

            # World position
            world_x = self.x + rx
            world_y = self.y + ry

            positions.append((world_x, world_y, star["brightness"], star["color"]))
        return positions


def generate(width, height):
    """Generates two spiral galaxies colliding with gravitational interaction."""
    if not hasattr(generate, "galaxies"):
        generate.galaxies = [
            Galaxy(width, height, 300, width / 4, height / 2, 0.3, 0, 0.002),
            Galaxy(width, height, 300, 3 * width / 4, height / 2, -0.3, 0, -0.002),
        ]

    grid = [[" " for _ in range(width)] for _ in range(height)]

    # Update galaxies with gravitational attraction
    generate.galaxies[0].update(generate.galaxies[1])
    generate.galaxies[1].update(generate.galaxies[0])

    # Star characters by brightness
    star_chars = [".", "·", "*", "#", "✦"]

    # Draw tidal tail stars
    for galaxy in generate.galaxies:
        for tstar in galaxy.tidal_tail_stars[:100]:
            tx, ty = int(tstar["x"]), int(tstar["y"])
            if 0 <= ty < height and 0 <= tx < width:
                fade_factor = 1 - (tstar["age"] / 200)
                if fade_factor > 0:
                    grid[ty][tx] = (
                        Fore.LIGHTBLACK_EX + "." + Style.RESET_ALL
                        if fade_factor < 0.5
                        else Fore.WHITE + "·" + Style.RESET_ALL
                    )

    # Draw stars from both galaxies
    for galaxy in generate.galaxies:
        for star_x, star_y, brightness, color in galaxy.get_star_positions():
            ix, iy = int(star_x), int(star_y)
            if 0 <= iy < height and 0 <= ix < width:
                grid[iy][ix] = color + star_chars[brightness] + Style.RESET_ALL

    # Draw star formation bursts
    for galaxy in generate.galaxies:
        for sf in galaxy.star_formation_burst:
            sx, sy = int(sf["x"]), int(sf["y"])
            if 0 <= sy < height and 0 <= sx < width:
                grid[sy][sx] = Fore.MAGENTA + "✦" + Style.RESET_ALL
            for dy in range(-2, 3):
                for dx in range(-2, 3):
                    if abs(dy) + abs(dx) <= 2:
                        ry, rx = sy + dy, sx + dx
                        if 0 <= ry < height and 0 <= rx < width:
                            if grid[ry][rx] in [" ", Fore.LIGHTBLACK_EX + "."]:
                                grid[ry][rx] = (
                                    random.choice(
                                        [
                                            Fore.CYAN + "~",
                                            Fore.BLUE + "≈",
                                            Fore.WHITE + "░",
                                        ]
                                    )
                                    + Style.RESET_ALL
                                )

    # Draw galactic cores (supermassive black holes)
    for galaxy in generate.galaxies:
        cx, cy = int(galaxy.x), int(galaxy.y)
        if 0 <= cy < height and 0 <= cx < width:
            grid[cy][cx] = Fore.YELLOW + "◉" + Style.RESET_ALL
        for dy in range(-1, 2):
            for dx in range(-2, 3):
                if abs(dy) + abs(dx) <= 2:
                    gx, gy = cx + dx, cy + dy
                    if 0 <= gy < height and 0 <= gx < width:
                        if grid[gy][gx] in [
                            " ",
                            Fore.LIGHTBLACK_EX + ".",
                            Fore.WHITE + "·",
                        ]:
                            grid[gy][gx] = (
                                random.choice(
                                    [
                                        Fore.YELLOW + "▒",
                                        Fore.WHITE + "░",
                                        Fore.GREEN + "o",
                                    ]
                                )
                                + Style.RESET_ALL
                            )

    # Add static background stars
    for _ in range(int(width * height * 0.002)):
        bx, by = random.randint(0, width - 1), random.randint(0, height - 1)
        if grid[by][bx] == " ":
            grid[by][bx] = Fore.WHITE + "." + Style.RESET_ALL

    # Calculate collision phase
    dx = generate.galaxies[1].x - generate.galaxies[0].x
    dy = generate.galaxies[1].y - generate.galaxies[0].y
    separation = math.sqrt(dx**2 + dy**2)

    # Info panel
    info = f"Galaxy Collision | {Fore.YELLOW}◉{Style.RESET_ALL}=Supermassive BH | {Fore.MAGENTA}✦{Style.RESET_ALL}=Star Formation | {Fore.WHITE}·{Style.RESET_ALL}=Tidal Tails | Sep: {int(separation)} units"
    if height - 1 >= 0:
        for i, char in enumerate(info):
            if i < width:
                grid[height - 1][i] = char

    # Reset if galaxies move off screen
    if (
        generate.galaxies[0].x > width + 20
        or generate.galaxies[1].x < -20
        or abs(generate.galaxies[0].y - height / 2) > height
    ):
        generate.galaxies = [
            Galaxy(width, height, 300, width / 4, height / 2, 0.3, 0, 0.002),
            Galaxy(width, height, 300, 3 * width / 4, height / 2, -0.3, 0, -0.002),
        ]

    return "\n".join(["".join(row) for row in grid])
