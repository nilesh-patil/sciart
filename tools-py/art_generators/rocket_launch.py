import random
import time


def generate(width, height):
    """Generates a vertical rocket launch animation.

    Shows a rocket launching upward with smoke and fire effects.
    """
    # Initialize rocket state
    if not hasattr(generate, "start_time"):
        generate.start_time = time.time()
        generate.rocket_y = height - 5

    elapsed = time.time() - generate.start_time

    # Reset after 15 seconds
    if elapsed > 15:
        generate.start_time = time.time()
        generate.rocket_y = height - 5
        elapsed = 0

    grid = [[" " for _ in range(width)] for _ in range(height)]

    rocket_x = width // 2

    # Countdown phase (first 3 seconds)
    if elapsed < 3:
        countdown = 3 - int(elapsed)
        text = str(countdown)
        text_x = rocket_x - len(text) // 2
        text_y = height // 2
        if 0 <= text_y < height:
            for i, char in enumerate(text):
                if 0 <= text_x + i < width:
                    grid[text_y][text_x + i] = char
    else:
        # Launch phase - rocket accelerates upward
        launch_time = elapsed - 3
        # Acceleration: y = y0 - 0.5 * a * t^2
        generate.rocket_y = int(height - 5 - (launch_time**2) * 2)

        # Draw rocket if still on screen
        if generate.rocket_y >= -4 and generate.rocket_y < height:
            # Multi-line ASCII rocket
            rocket = ["  ^  ", " / \\ ", "|   |", "|___| ", " | | "]

            for i, line in enumerate(rocket):
                y = generate.rocket_y + i
                if 0 <= y < height:
                    x_start = rocket_x - len(line) // 2
                    for j, char in enumerate(line):
                        x = x_start + j
                        if 0 <= x < width and char != " ":
                            grid[y][x] = char

            # Draw exhaust/smoke below rocket
            exhaust_y_start = generate.rocket_y + len(rocket)
            for i in range(min(15, height - exhaust_y_start)):
                y = exhaust_y_start + i
                if 0 <= y < height:
                    # Expanding smoke cloud
                    spread = i // 2
                    for dx in range(-spread, spread + 1):
                        x = rocket_x + dx
                        if 0 <= x < width and random.random() < 0.7:
                            grid[y][x] = random.choice(["*", ".", "~", "+"])

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
