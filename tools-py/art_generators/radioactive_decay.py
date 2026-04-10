import random
import time


def generate(width, height):
    """Generates a visualization of radioactive decay.

    Shows unstable isotopes randomly decaying over time,
    following a half-life decay curve.
    """
    # Initialize isotopes on first call or when the terminal is resized
    if getattr(generate, "dims", None) != (width, height):
        generate.dims = (width, height)
        generate.isotopes = []
        generate.start_time = time.time()
        for i in range(height):
            row = []
            for j in range(width):
                row.append({"decayed": False, "char": "U"})  # U for Uranium
            generate.isotopes.append(row)

    # Calculate decay probability based on elapsed time
    # Half-life of about 20 seconds for visualization
    elapsed = time.time() - generate.start_time
    half_life = 20.0
    decay_probability = 1 - (0.5 ** (elapsed / half_life))

    grid = [[" " for _ in range(width)] for _ in range(height)]

    for i in range(height):
        for j in range(width):
            isotope = generate.isotopes[i][j]

            if not isotope["decayed"]:
                # Check if this isotope should decay
                if random.random() < decay_probability:
                    isotope["decayed"] = True
                    isotope["char"] = random.choice([".", "*", "+"])  # Decay products

            grid[i][j] = isotope["char"]

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
        time.sleep(0.2)
