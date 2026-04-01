import random
import math


class Particle:
    def __init__(self, x, y, vx, vy):
        self.x = x
        self.y = y
        self.vx = vx
        self.vy = vy

    def update(self, width, height, box_margin=2):
        """Update particle position and handle wall collisions."""
        self.x += self.vx
        self.y += self.vy

        # Bounce off walls (accounting for aspect ratio)
        if self.x <= box_margin or self.x >= width - box_margin:
            self.vx = -self.vx
            self.x = max(box_margin, min(width - box_margin, self.x))

        if self.y <= box_margin or self.y >= height - box_margin:
            self.vy = -self.vy
            self.y = max(box_margin, min(height - box_margin, self.y))


def generate(width, height):
    """Generates a 2D box with bouncing molecules.

    Simulates molecular dynamics with particle-particle and particle-wall collisions.
    """
    # Initialize particles on first call
    if not hasattr(generate, "particles"):
        generate.particles = []
        num_particles = 30
        for _ in range(num_particles):
            x = random.uniform(5, width - 5)
            y = random.uniform(5, height - 5)
            vx = random.uniform(-1, 1)
            vy = random.uniform(-0.5, 0.5)  # Smaller y velocity due to aspect ratio
            generate.particles.append(Particle(x, y, vx, vy))

    grid = [[" " for _ in range(width)] for _ in range(height)]

    # Draw bounding box
    for i in range(height):
        grid[i][0] = "|"
        grid[i][width - 1] = "|"
    for j in range(width):
        grid[0][j] = "-"
        grid[height - 1][j] = "-"
    grid[0][0] = "+"
    grid[0][width - 1] = "+"
    grid[height - 1][0] = "+"
    grid[height - 1][width - 1] = "+"

    # Update and draw particles
    for particle in generate.particles:
        particle.update(width, height)

        # Simple particle-particle collision detection
        for other in generate.particles:
            if particle is not other:
                dx = particle.x - other.x
                dy = particle.y - other.y
                distance = math.sqrt(dx**2 + dy**2)

                if distance < 2 and distance > 0:  # Collision threshold
                    # Simple elastic collision - swap velocities
                    particle.vx, other.vx = other.vx, particle.vx
                    particle.vy, other.vy = other.vy, particle.vy

        # Draw particle with character based on velocity (temperature)
        x = int(particle.x)
        y = int(particle.y)
        if 0 <= y < height and 0 <= x < width:
            # Calculate particle speed (kinetic energy ~ temperature)
            speed = math.sqrt(particle.vx**2 + particle.vy**2)

            # Map speed to character (temperature visualization)
            if speed < 0.3:
                char = "."  # Cold/slow
            elif speed < 0.6:
                char = "o"  # Medium
            elif speed < 0.9:
                char = "O"  # Warm
            else:
                char = "@"  # Hot/fast

            grid[y][x] = char

    # Draw temperature scale legend
    legend_y = 1
    legend_x = 2
    legend = "Temp: . < o < O < @"
    if legend_y < height:
        for i, c in enumerate(legend):
            if legend_x + i < width - 1:
                grid[legend_y][legend_x + i] = c

    return "\n".join(["".join(row) for row in grid])


if __name__ == "__main__":
    import os
    import time

    try:
        width, height = os.get_terminal_size()
    except OSError:
        width, height = 80, 24

    while True:
        output = generate(width, height)
        os.system("clear" if os.name == "posix" else "cls")
        print(output)
        time.sleep(0.05)
