import random


def generate(width, height):
    """Generates a representation of the Cosmic Microwave Background.

    The CMB is the faint afterglow of the Big Bang, represented as
    a field of subtle temperature fluctuations.
    """
    grid = [[" " for _ in range(width)] for _ in range(height)]

    # Characters representing different temperature densities
    chars = [".", ":", "-", "=", "+", "*", "#"]

    for i in range(height):
        for j in range(width):
            # Random character representing temperature fluctuations
            grid[i][j] = random.choice(chars)

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
        time.sleep(0.5)
