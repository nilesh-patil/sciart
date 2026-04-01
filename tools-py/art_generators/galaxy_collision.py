import random
import math


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
            brightness = random.choice([0, 1, 2]) if distance < self.width / 6 else 0

            stars.append(
                {
                    "local_x": star_x,
                    "local_y": star_y,
                    "angle": angle,
                    "distance": distance,
                    "brightness": brightness,
                }
            )
        return stars

    def update(self, other_galaxy=None):
        """Update galaxy position with gravitational influence."""
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

            positions.append((world_x, world_y, star["brightness"]))
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
    star_chars = [".", "·", "*", "#"]

    # Draw stars from both galaxies
    for galaxy in generate.galaxies:
        for star_x, star_y, brightness in galaxy.get_star_positions():
            ix, iy = int(star_x), int(star_y)
            if 0 <= iy < height and 0 <= ix < width:
                grid[iy][ix] = star_chars[brightness]

    # Draw galactic cores
    for galaxy in generate.galaxies:
        cx, cy = int(galaxy.x), int(galaxy.y)
        if 0 <= cy < height and 0 <= cx < width:
            grid[cy][cx] = "@"
        # Core glow
        for dx, dy in [(-1, 0), (1, 0), (0, -1), (0, 1)]:
            gx, gy = cx + dx, cy + dy
            if 0 <= gy < height and 0 <= gx < width:
                grid[gy][gx] = "o"

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
