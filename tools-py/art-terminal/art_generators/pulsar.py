import math
import time


def generate(width, height):
    """Generates pulsar with sweeping beams and magnetic field lines."""
    grid = [[" " for _ in range(width)] for _ in range(height)]

    pulsar_x, pulsar_y = width // 2, height // 2

    # Rotation angle
    angle = time.time() * 3

    # Draw magnetic field lines (faint curves around pulsar)
    for field_angle in range(0, 360, 30):
        base_angle = math.radians(field_angle)
        for dist in range(5, 20):
            curve = math.sin(dist * 0.2) * 2
            x = pulsar_x + int(
                (dist * math.cos(base_angle) + curve * math.sin(base_angle)) * 2.5
            )
            y = pulsar_y + int(
                (dist * math.sin(base_angle) - curve * math.cos(base_angle))
            )
            if 0 <= y < height and 0 <= x < width and dist > 8:
                if grid[y][x] == " ":
                    grid[y][x] = "~" if dist % 2 == 0 else "≈"

    # Draw two opposing radio beams with cone shape
    for beam_num in range(2):
        beam_angle = angle + (beam_num * math.pi)

        for i in range(1, min(width, height) // 2):
            beam_width = i // 5

            for offset in range(-beam_width, beam_width + 1):
                perp_angle = beam_angle + math.pi / 2
                off_x = offset * math.cos(perp_angle)
                off_y = offset * math.sin(perp_angle)

                x = pulsar_x + int((i * math.cos(beam_angle) + off_x) * 2.5)
                y = pulsar_y + int((i * math.sin(beam_angle) + off_y))

                if 0 <= y < height and 0 <= x < width:
                    intensity = max(0, 3 - (i // 8) - abs(offset))
                    if intensity > 0:
                        chars = ["·", "╱" if beam_num == 0 else "╲", "╱" if beam_num == 0 else "╲", "│"]
                        if grid[y][x] == " ":
                            grid[y][x] = chars[min(intensity, 3)]

    # Draw pulsar core
    for dy in range(-2, 3):
        for dx in range(-4, 5):
            dist = math.sqrt((dx / 2.5) ** 2 + dy**2)
            if dist <= 2:
                y, x = pulsar_y + dy, pulsar_x + dx
                if 0 <= y < height and 0 <= x < width:
                    if dist < 0.5:
                        grid[y][x] = "⊙"
                    elif dist < 1:
                        grid[y][x] = "◉"
                    else:
                        grid[y][x] = "○"

    return "\n".join(["".join(row) for row in grid])
