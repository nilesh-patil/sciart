from colorama import Fore, init
import random
import time

# Initialize colorama
init(autoreset=True)


def generate(screen_width, screen_height):
    """Generates a vertical rocket launch animation with staging and enhanced effects.

    Shows a rocket launching upward with animated flames, smoke, staging, and boosters.
    """
    # Initialize rocket state
    if not hasattr(generate, "start_time"):
        generate.start_time = time.time()
        generate.rocket_y = screen_height - 5
        generate.stage = 1
        generate.boosters = []

    elapsed = time.time() - generate.start_time

    # Reset after 20 seconds
    if elapsed > 20:
        generate.start_time = time.time()
        generate.rocket_y = screen_height - 5
        generate.stage = 1
        generate.boosters = []
        elapsed = 0

    grid = [[" " for _ in range(screen_width)] for _ in range(screen_height)]

    rocket_x = screen_width // 2

    # Countdown phase (first 3 seconds)
    if elapsed < 3:
        countdown = 3 - int(elapsed)
        text = str(countdown)
        text_x = rocket_x - len(text) // 2
        text_y = screen_height // 2
        if 0 <= text_y < screen_height:
            for i, char in enumerate(text):
                if 0 <= text_x + i < screen_width:
                    grid[text_y][text_x + i] = Fore.YELLOW + char

        # Draw rocket on launchpad
        rocket = ["  ^  ", " /|\\ ", "|   |", "|___| ", " | | "]
        rocket_start_y = screen_height - 8
        for i, line in enumerate(rocket):
            y = rocket_start_y + i
            if 0 <= y < screen_height:
                x_start = rocket_x - len(line) // 2
                for j, char in enumerate(line):
                    x = x_start + j
                    if 0 <= x < screen_width and char != " ":
                        grid[y][x] = Fore.WHITE + char
    else:
        # Launch phase - rocket accelerates upward
        launch_time = elapsed - 3
        # Acceleration: y = y0 - 0.5 * a * t^2
        generate.rocket_y = int(screen_height - 5 - (launch_time**2) * 2)

        # Stage separation at 6 seconds (launch_time = 3)
        if launch_time > 3 and generate.stage == 1:
            generate.stage = 2
            # Create booster separation
            generate.boosters.append(
                {"y": generate.rocket_y + 5, "x_offset": -4, "vy": 0.5, "vx": -0.3}
            )
            generate.boosters.append(
                {"y": generate.rocket_y + 5, "x_offset": 4, "vy": 0.5, "vx": 0.3}
            )

        # Update and draw falling boosters
        boosters_to_remove = []
        for booster in generate.boosters:
            booster["y"] += booster["vy"]
            booster["x_offset"] += booster["vx"]
            booster["vy"] += 0.1  # Gravity

            if booster["y"] >= screen_height:
                boosters_to_remove.append(booster)
            else:
                bx = rocket_x + int(booster["x_offset"])
                by = int(booster["y"])
                # Draw small booster
                if 0 <= by < screen_height and 0 <= bx < screen_width:
                    grid[by][bx] = Fore.CYAN + "["
                if 0 <= by + 1 < screen_height and 0 <= bx < screen_width:
                    grid[by + 1][bx] = Fore.CYAN + "]"

        for b in boosters_to_remove:
            generate.boosters.remove(b)

        # Draw rocket if still on screen
        if generate.rocket_y >= -6 and generate.rocket_y < screen_height:
            # Multi-line ASCII rocket (smaller in stage 2)
            if generate.stage == 1:
                rocket = ["  ^  ", " /|\\ ", "|   |", "|___| ", " ||| "]
            else:
                rocket = ["  ^  ", " / \\", "|   |", "|___| "]

            for i, line in enumerate(rocket):
                y = generate.rocket_y + i
                if 0 <= y < screen_height:
                    x_start = rocket_x - len(line) // 2
                    for j, char in enumerate(line):
                        x = x_start + j
                        if 0 <= x < screen_width and char != " ":
                            grid[y][x] = Fore.WHITE + char

            # Draw animated exhaust flames
            exhaust_y_start = generate.rocket_y + len(rocket)

            for i in range(min(12, screen_height - exhaust_y_start)):
                y = exhaust_y_start + i
                if 0 <= y < screen_height:
                    # Expanding flame cone
                    spread = max(1, i // 3)
                    for dx in range(-spread, spread + 1):
                        x = rocket_x + dx
                        if 0 <= x < screen_width:
                            prob = 0.8 - (i * 0.05)
                            if random.random() < prob:
                                # Animated flame characters
                                if i < 3:
                                    char = Fore.RED + random.choice(
                                        ["▲", "△", "^", "*"]
                                    )
                                elif i < 6:
                                    char = Fore.YELLOW + random.choice(
                                        ["*", "+", "×", "·"]
                                    )
                                else:
                                    char = Fore.LIGHTYELLOW_EX + random.choice(
                                        ["~", ".", "·", " "]
                                    )
                                grid[y][x] = char

            # Ground smoke cloud
            smoke_y = screen_height - 3
            for y in range(max(0, smoke_y), min(screen_height, smoke_y + 3)):
                for x in range(max(0, rocket_x - 15), min(screen_width, rocket_x + 15)):
                    if random.random() < 0.15:
                        grid[y][x] = Fore.LIGHTBLACK_EX + random.choice(
                            ["░", "▒", "~", "."]
                        )

    return "\n".join(["".join(row) for row in grid])


if __name__ == "__main__":
    import os

    try:
        term_width, term_height = os.get_terminal_size()
    except OSError:
        term_width, term_height = 80, 24

    while True:
        output = generate(term_width, term_height)
        os.system("clear" if os.name == "posix" else "cls")
        print(output)
        time.sleep(0.1)
