import pyautogui
import time
import random
from rich.console import Console
from rich.live import Live
from rich.layout import Layout
import colorsys
import importlib
import pkgutil
import sys
import art_generators


def move_mouse():
    """Moves the mouse to a random position within a restricted area on the screen."""
    screen_width, screen_height = pyautogui.size()
    margin = 50
    x = random.randint(margin, screen_width - margin)
    y = random.randint(margin, screen_height - margin)
    pyautogui.moveTo(x, y, duration=0.5)


def generate_random_color_text(text):
    """Generates a random brighter color for each character of the text."""
    colored_text = ""
    for char in text:
        if char != " ":
            h = random.random()
            s = 1.0
            v = 1.0
            r, g, b = [int(c * 255) for c in colorsys.hsv_to_rgb(h, s, v)]
            colored_text += f"[rgb({r},{g},{b})]{char}[/rgb({r},{g},{b})]"
        else:
            colored_text += " "
    return colored_text


def load_art_generators():
    """Dynamically loads all art generator modules."""
    generators = []
    for _, name, _ in pkgutil.iter_modules(art_generators.__path__):
        module = importlib.import_module(f"art_generators.{name}")
        if hasattr(module, "generate"):
            generators.append(module.generate)
    return generators


def main():
    """Main function to run the mouse mover with full screen ASCII art."""
    console = Console()
    console.print("Press Ctrl-C to quit.", style="bold red")

    art_generators = load_art_generators()
    if not art_generators:
        console.print("No art generators found.", style="bold red")
        return

    # Check for command-line argument to run a specific generator
    if len(sys.argv) > 1:
        art_name = sys.argv[1]
        generator = next(
            (g for g in art_generators if g.__module__.endswith(art_name)), None
        )
        if not generator:
            console.print(f"Art generator '{art_name}' not found.", style="bold red")
            return
        art_generators = [generator]  # Run only the specified generator

    layout = Layout(name="art")

    try:
        with Live(layout, console=console, screen=True, redirect_stderr=False) as live:
            while True:
                move_mouse()
                live.console.print(f"Moved mouse to {pyautogui.position()}")

                width, height = console.width, console.height
                generator = random.choice(art_generators)
                art = generator(width, height)
                colored_art = generate_random_color_text(art)
                layout.update(colored_art)

                time.sleep(15)

    except KeyboardInterrupt:
        console.print("\nExiting.", style="bold red")


if __name__ == "__main__":
    main()
