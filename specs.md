# Terminal Visualization Tool - Technical Specification

## Executive Summary

A Python-based terminal visualization system that displays animated ASCII art of astronomical and physics phenomena while preventing screen sleep through periodic mouse movement. The tool leverages Python 3.13+ free-threading for parallel rendering performance and emphasizes simplicity, robustness, and scientific accuracy.

**Core Principles:**
- **Simplicity First**: Minimal configuration, no interactive controls, fail-fast error handling
- **Performance Through Parallelism**: Active use of free-threading for generator computations and pre-rendering
- **Robustness**: Multi-layer fallback system with hard resource limits
- **Internal Quality**: Comprehensive testing without external plugin complexity

---

## Table of Contents

1. [Use Cases](#use-cases)
2. [Operation Modes](#operation-modes)
3. [Architecture](#architecture)
4. [Generator System](#generator-system)
5. [Color System](#color-system)
6. [Resource Management](#resource-management)
7. [Configuration](#configuration)
8. [Testing Strategy](#testing-strategy)
9. [System Requirements](#system-requirements)
10. [User Experience](#user-experience)
11. [Security & Quality](#security--quality)
12. [Implementation Roadmap](#implementation-roadmap)

---

## Use Cases

**Primary Use Cases:**
- Educational demonstrations of scientific concepts in terminal environment
- Terminal screensaver with scientifically-themed animations
- Background activity maintenance during presentations or long-running monitoring tasks
- Visual testing of terminal rendering capabilities and performance benchmarking

**Target Users:**
- Developers and system administrators who work in terminal environments
- Educators demonstrating physics and astronomy concepts
- Terminal enthusiasts and ASCII art aficionados

---

## Operation Modes

### Mode Overview

The tool supports four distinct operation modes, selectable through an interactive menu or command-line flags:

| Mode | Flag | Description | Use Case |
|------|------|-------------|----------|
| **Random Cycle** | *(default)* | Cycles through all visualizations with mouse movement | Standard screensaver mode |
| **Visualization Only** | `--no-mouse` | Display visualizations without mouse movement | Testing, pure entertainment |
| **Mouse Only** | `--headless` | Mouse movement without visualization | Minimal resource sleep prevention |
| **Benchmark** | `--benchmark` | Performance testing of all generators | Development, CI/CD |

### Mode Specifications

#### 1. Random Cycle Mode (Default)

**Behavior:**
- Automatically rotates through all 25 visualizations
- Changes visualization every 15 seconds (configurable via `.ascii-viz.toml`)
- Moves mouse to random screen position between cycles
- Full-screen colorful rendering with thematic color palettes
- Uniform random selection (each generator has equal 1/25 probability)

**Technical Details:**
- Mouse movement: 50px margin from screen edges, 0.5-second smooth motion
- Mouse target: Configurable per-display (primary only, all displays, specific display)
- State persistence: Generator state persists in memory across cycles
- Pre-rendering: Next generator is pre-computed in parallel during current display

#### 2. Visualization Only Mode (`--no-mouse`)

**Behavior:**
- Identical to random cycle mode but without mouse automation
- Useful for testing visualizations or when screen sleep prevention is not needed
- Lower resource usage (no PyAutoGUI overhead)

#### 3. Mouse Only Mode (`--headless`)

**Behavior:**
- Moves mouse at configured intervals without any visualization
- Minimal resource footprint for sleep prevention only
- No terminal rendering, no color computation
- Logs to stderr only

#### 4. Benchmark Mode (`--benchmark`)

**Behavior:**
- Executes each generator exactly once
- Measures and reports performance metrics:
  - Execution time: min/max/avg across all generators
  - Memory allocation delta: before/after comparison
- Outputs results as JSON to stdout
- Exits immediately after completion (no cycling)

**Output Format:**
```json
{
  "generators": [
    {
      "name": "solar_system",
      "execution_time_ms": 23.4,
      "memory_delta_mb": 0.8,
      "status": "pass"
    },
    {
      "name": "galaxy_collision",
      "execution_time_ms": 89.2,
      "memory_delta_mb": 2.1,
      "status": "pass"
    }
  ],
  "summary": {
    "total_generators": 25,
    "passed": 25,
    "failed": 0,
    "avg_execution_time_ms": 42.1
  }
}
```

---

## Architecture

### High-Level Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                      Main Orchestrator                       │
│                     (activate.py)                            │
│                                                              │
│  ┌────────────────┐  ┌─────────────────┐  ┌──────────────┐ │
│  │   Generator    │  │   Parallel      │  │   Mouse      │ │
│  │   Discovery    │  │   Renderer      │  │   Controller │ │
│  │   & Selector   │  │   (Threading)   │  │  (PyAutoGUI) │ │
│  └────────────────┘  └─────────────────┘  └──────────────┘ │
│           │                   │                    │         │
│           ▼                   ▼                    ▼         │
│  ┌────────────────┐  ┌─────────────────┐  ┌──────────────┐ │
│  │ Auto-discover  │  │  Pre-render     │  │  Move mouse  │ │
│  │ from package   │  │  next generator │  │  randomly    │ │
│  └────────────────┘  └─────────────────┘  └──────────────┘ │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
        ┌───────────────────────────────────────────┐
        │        Generator Layer                    │
        │     (art_generators/*.py)                 │
        │                                           │
        │  ┌──────────────┐  ┌──────────────┐     │
        │  │ Astronomy    │  │ Physics      │     │
        │  │ Generators   │  │ Generators   │ ... │
        │  └──────────────┘  └──────────────┘     │
        └───────────────────────────────────────────┘
                              │
                              ▼
        ┌───────────────────────────────────────────┐
        │         Rendering Layer                   │
        │         (Rich + Color System)             │
        │                                           │
        │  ┌────────────┐  ┌────────────────────┐  │
        │  │  Thematic  │  │   Terminal Output  │  │
        │  │  Coloring  │  │   (Rich Live)      │  │
        │  └────────────┘  └────────────────────┘  │
        └───────────────────────────────────────────┘
```

### Component Responsibilities

#### Main Orchestrator (`activate.py`)

**Responsibilities:**
1. **Generator Discovery**: Auto-discover all modules in `art_generators/` using `pkgutil.iter_modules()`
2. **Mode Selection**: Handle command-line flags and interactive menu
3. **Rendering Loop**: Execute selected mode (cycle, single, benchmark)
4. **Error Handling**: Catch generator exceptions and trigger fallback system
5. **Resource Monitoring**: Enforce timeout and memory limits
6. **Parallel Coordination**: Manage free-threaded tasks for pre-rendering and grid operations

**Key Implementation Points:**
- Uses Python 3.13+ free-threading for parallel operations
- Maintains generator state dictionary: `{generator_name: state_dict}`
- Implements three-level fallback: failing generator → starfield → single dot

#### Generator Discovery System

**Auto-Discovery Algorithm:**
```python
import pkgutil
import art_generators

discovered = []
for importer, modname, ispkg in pkgutil.iter_modules(art_generators.__path__):
    module = importer.find_module(modname).load_module(modname)
    if hasattr(module, 'generate'):
        discovered.append({
            'name': modname,
            'module': module,
            'category': infer_category(modname)  # Based on visuals.md mapping
        })
```

**Naming Requirements:**
- Generator files must be valid Python module names (lowercase, underscores)
- File name becomes generator identifier (e.g., `black_hole.py` → `"black_hole"`)
- No name collision detection needed (Python import system handles this)

#### Parallel Renderer (Free-Threading)

**Parallelization Strategy:**

The tool uses Python 3.13+ free-threading for three concurrent operations:

1. **Generator Internal Computations**
   - Particle systems update positions in parallel threads
   - Physics calculations split across CPU cores
   - Example: `galaxy_collision.py` updates Galaxy A and Galaxy B in parallel

2. **Pre-Rendering Next Generator**
   - While displaying current frame, compute next generator in background
   - Reduces perceived latency between cycles
   - Cached result is ready when cycle timer expires

3. **Parallel Grid Operations**
   - Large grid population operations split into chunks
   - Each thread handles a horizontal slice of the output
   - Particularly beneficial for dense visualizations (stellar nursery, CMB)

**Performance Expectations:**
- 2-3x speedup on particle-heavy generators (500+ particles)
- Reduced cycle transition latency from ~100ms to ~10ms
- Negligible benefit for simple generators (starfield, constellation map)

---

## Generator System

### Generator Contract

All generators implement a minimal, stateless interface:

```python
def generate(width: int, height: int) -> tuple[str, str]:
    """
    Generate ASCII art for given terminal dimensions.

    Args:
        width: Terminal width in characters
        height: Terminal height in characters

    Returns:
        Tuple of (ascii_art: str, palette_name: str)
        - ascii_art: Multi-line string with newline-separated rows
        - palette_name: One of 'astronomy', 'physics', 'chemistry'

    Performance Requirements:
        - Must complete within 100ms (hard timeout)
        - Memory allocation must not exceed 100MB delta
        - Output must be exactly width x height characters

    Example:
        >>> art, palette = generate(80, 24)
        >>> assert len(art.split('\\n')) == 24
        >>> assert all(len(line) <= 80 for line in art.split('\\n'))
        >>> assert palette in ['astronomy', 'physics', 'chemistry']
    """
```

### Generator Categories

Generators are organized into three categories, each with thematic color mapping:

| Category | Generators | Palette Theme |
|----------|------------|---------------|
| **Astronomy** | solar_system, galaxy_collision, black_hole, pulsar, gravitational_lensing, constellation_map, moon_phases, exoplanet_transit, cosmic_microwave_background, stellar_nursery | Cool blues/whites (temperature-based) |
| **Physics** | particle_accelerator, wave_interference, double_pendulum, brownian_motion, spacetime_grid, radioactive_decay, doppler_effect | Energetic reds/yellows (temperature-based) |
| **Chemistry** | molecular_dynamics, crystal_growth, chemical_reaction | Balanced greens/purples (temperature-based) |

### State Persistence Pattern

Generators use function attributes for animation state:

```python
def generate(width, height):
    # Initialize state on first call
    if not hasattr(generate, "particles"):
        generate.particles = initialize_particles(500)
        generate.frame_count = 0

    # Update state
    generate.frame_count += 1
    for particle in generate.particles:
        particle.update(generate.frame_count)

    # Render current state
    grid = create_grid(width, height)
    for particle in generate.particles:
        plot_particle(grid, particle)

    return grid_to_string(grid), "astronomy"
```

**State Lifecycle:**
- State is stored in function attributes (e.g., `generate.particles`)
- State persists across calls in the same Python process
- State persists across generator cycles (galaxy collision continues from where it left off)
- State is reset on program restart (no disk persistence)
- Each generator maintains independent state (no cross-generator communication)

### Animation Techniques

#### 1. Time-Based Animation

For continuous, smooth motion:

```python
import time

def generate(width, height):
    current_time = time.time()
    angle = (current_time * 2.0) % (2 * math.pi)  # 2 rad/sec rotation

    x = center_x + int(radius * math.cos(angle) * 2.5)
    y = center_y + int(radius * math.sin(angle))
    # ... render
```

**Examples:** `pulsar.py` (rotating beams), `solar_system.py` (orbiting planets)

#### 2. State-Based Animation

For discrete particle systems:

```python
def generate(width, height):
    if not hasattr(generate, "particles"):
        generate.particles = [Particle() for _ in range(100)]

    for particle in generate.particles:
        particle.velocity += particle.acceleration * dt
        particle.position += particle.velocity * dt
        # ... boundary checks, collisions

    # ... render particles
```

**Examples:** `black_hole.py` (accretion disk), `brownian_motion.py` (random walk)

#### 3. Hybrid Animation

Combining both for complex effects:

```python
def generate(width, height):
    if not hasattr(generate, "galaxies"):
        generate.galaxies = [Galaxy(), Galaxy()]

    current_time = time.time()

    # Time-based: smooth interpolation
    phase = (current_time % 10.0) / 10.0  # 10-second cycle

    # State-based: update positions
    for galaxy in generate.galaxies:
        galaxy.position += galaxy.velocity * phase
        galaxy.rotate(0.1)

    # ... render
```

**Examples:** `galaxy_collision.py` (Galaxy objects with time-based interpolation)

### Coordinate System & Aspect Ratio

Terminal characters have ~2:1 height/width ratio. Generators compensate:

```python
# Circular shape requires horizontal scaling
angle_rad = math.radians(angle)
x = center_x + int(radius * math.cos(angle_rad) * 2.5)  # 2.5x horizontal
y = center_y + int(radius * math.sin(angle_rad))        # 1x vertical

if 0 <= y < height and 0 <= x < width:
    grid[y][x] = "*"
```

**Standard Scaling Factor:** 2.5 (empirically determined for most terminals)

### Generator Documentation

Generators use simple docstrings:

```python
def generate(width: int, height: int) -> tuple[str, str]:
    """
    Renders a black hole accretion disk with Keplerian rotation.

    Particles orbit at velocities proportional to 1/sqrt(distance),
    creating the characteristic disk shape. Inner particles move faster,
    outer particles slower.
    """
```

**Documentation Requirements:**
- Brief description of what is visualized
- Scientific accuracy notes if applicable
- No structured metadata, author info, or complexity ratings

---

## Color System

### Architecture

The color system separates concerns between generators (semantic intent) and orchestrator (color application):

1. **Generators** return a palette name: `'astronomy'`, `'physics'`, or `'chemistry'`
2. **Orchestrator** maps palette name to color ranges
3. **Rich library** applies colors to each non-space character

### Thematic Color Palettes

Color palettes use temperature-based mapping for scientific intuition:

| Palette Name | Theme | Hue Range (HSV) | Saturation | Value | Visual Description |
|--------------|-------|-----------------|------------|-------|-------------------|
| `astronomy` | Cool (distant, cosmic) | 180-240° (blue-cyan) | 80-100% | 80-100% | Deep blues, cool whites, icy cyans |
| `physics` | Energetic (hot, dynamic) | 0-60° (red-yellow) | 90-100% | 90-100% | Bright reds, oranges, yellows |
| `chemistry` | Balanced (neutral, reactive) | 100-180° (green-cyan) | 70-90% | 70-90% | Vibrant greens, balanced purples |

### Color Application Algorithm

```python
from rich.style import Style
from colorsys import hsv_to_rgb

PALETTE_RANGES = {
    'astronomy': {'hue': (180, 240), 'sat': (0.8, 1.0), 'val': (0.8, 1.0)},
    'physics':   {'hue': (0, 60),    'sat': (0.9, 1.0), 'val': (0.9, 1.0)},
    'chemistry': {'hue': (100, 180), 'sat': (0.7, 0.9), 'val': (0.7, 0.9)},
}

def apply_colors(ascii_art: str, palette_name: str) -> str:
    """Apply thematic colors to ASCII art based on palette."""
    ranges = PALETTE_RANGES[palette_name]
    colored_output = []

    for char in ascii_art:
        if char == ' ' or char == '\n':
            colored_output.append(char)
        else:
            # Random color within palette range
            h = random.uniform(*ranges['hue'])
            s = random.uniform(*ranges['sat'])
            v = random.uniform(*ranges['val'])

            r, g, b = hsv_to_rgb(h/360, s, v)
            style = Style(color=f"rgb({int(r*255)},{int(g*255)},{int(b*255)})")
            colored_output.append(f"[{style}]{char}[/]")

    return "".join(colored_output)
```

### Color System Benefits

1. **Semantic Intent**: Generators declare their thematic category, not RGB values
2. **Consistent Aesthetics**: All astronomy generators have coherent blue palette
3. **Variation**: Random sampling within range prevents monotony
4. **Separation of Concerns**: Generators don't need to know about Rich library or terminal capabilities

---

## Resource Management

### Hard Resource Limits

To prevent runaway generators from degrading system performance or causing crashes:

| Resource | Limit | Enforcement | Consequence |
|----------|-------|-------------|-------------|
| **Execution Time** | 100ms per `generate()` call | Timeout decorator with signal | Fallback to starfield |
| **Memory Allocation** | 100MB delta per generator | Pre/post memory measurement | Fallback to starfield |
| **Output Size** | `width * height` characters | Truncate or reject | Log error, fallback |

### Fallback System (Three Levels)

**Level 1: Primary Generator**
- Execute requested generator
- Apply resource limits
- If successful, render normally

**Level 2: Starfield Fallback**
- If primary generator fails (exception, timeout, memory):
  - Log error to stderr: `"ERROR: <generator_name> failed: <reason>"`
  - Immediately switch to `starfield.py` (known simple, fast generator)
  - Continue operation without interruption

**Level 3: Ultimate Fallback (Single Dot)**
- If even starfield fails (critical system issue):
  - Render a single ASCII dot `.` at screen center
  - Dot moves position each frame: `(width//2 + random.randint(-5, 5), height//2 + random.randint(-3, 3))`
  - This is mathematically impossible to fail (no loops, no allocations, no computations)
  - Logs: `"CRITICAL: Starfield fallback failed, using minimal renderer"`

**Level 4: Graceful Exit**
- If single dot fails (should never happen):
  - Log: `"FATAL: All rendering failed, exiting"`
  - Exit with code 1
  - This indicates OS-level issues (out of memory, terminal corruption)

### Timeout Implementation

```python
import signal
from contextlib import contextmanager

class TimeoutException(Exception):
    pass

@contextmanager
def timeout(seconds):
    def handler(signum, frame):
        raise TimeoutException()

    signal.signal(signal.SIGALRM, handler)
    signal.alarm(seconds)
    try:
        yield
    finally:
        signal.alarm(0)

# Usage
try:
    with timeout(0.1):  # 100ms
        ascii_art, palette = generator.generate(width, height)
except TimeoutException:
    log_error(f"{generator_name} exceeded 100ms timeout")
    ascii_art, palette = starfield.generate(width, height)
```

### Memory Monitoring

```python
import tracemalloc

def measure_generator(generator_func, width, height):
    tracemalloc.start()
    snapshot_before = tracemalloc.take_snapshot()

    result = generator_func(width, height)

    snapshot_after = tracemalloc.take_snapshot()
    tracemalloc.stop()

    stats = snapshot_after.compare_to(snapshot_before, 'lineno')
    total_delta_mb = sum(stat.size_diff for stat in stats) / (1024 * 1024)

    if total_delta_mb > 100:
        raise MemoryError(f"Generator allocated {total_delta_mb:.1f}MB")

    return result
```

---

## Configuration

### Configuration File

**Location:** `.ascii-viz.toml` (project-local, checked into version control)

**Rationale:** Project-local allows teams to share configurations and ensures reproducible behavior across environments.

### Configuration Schema

```toml
# .ascii-viz.toml

[orchestrator]
cycle_duration_seconds = 15        # Time between generator switches
mouse_margin_pixels = 50           # Buffer from screen edges
mouse_movement_duration = 0.5      # Smooth motion time (seconds)
default_palette = "astronomy"      # Fallback if generator doesn't specify

[resource_limits]
timeout_ms = 100                   # Max execution time per generate()
memory_limit_mb = 100              # Max memory delta per generator

[display]
multi_monitor_mode = "primary"     # Options: "primary", "all", "random", or specific index like "1"
preferred_terminal_size = [120, 40]  # Hint for documentation, not enforced

[generator_overrides]
# Per-generator parameter overrides (passed as kwargs if generator supports)
[generator_overrides.galaxy_collision]
particle_count = 500
collision_speed = 1.5

[generator_overrides.black_hole]
particle_count = 750
rotation_speed = 2.0

[categories]
# Optional: disable entire categories
enabled = ["astronomy", "physics", "chemistry"]
```

### Configuration Loading

```python
import tomli
from pathlib import Path

def load_config():
    config_path = Path.cwd() / ".ascii-viz.toml"

    # Default configuration
    default_config = {
        "orchestrator": {
            "cycle_duration_seconds": 15,
            "mouse_margin_pixels": 50,
            "mouse_movement_duration": 0.5,
            "default_palette": "astronomy",
        },
        "resource_limits": {
            "timeout_ms": 100,
            "memory_limit_mb": 100,
        },
        "display": {
            "multi_monitor_mode": "primary",
        },
        "generator_overrides": {},
        "categories": {
            "enabled": ["astronomy", "physics", "chemistry"],
        },
    }

    if config_path.exists():
        with open(config_path, "rb") as f:
            user_config = tomli.load(f)
        # Deep merge user_config into default_config
        return merge_configs(default_config, user_config)

    return default_config
```

### Multi-Monitor Configuration

Configuration determines mouse movement behavior across displays:

- `"primary"`: Constrain to primary display only (default, most predictable)
- `"all"`: Randomly select any connected display each cycle
- `"random"`: Alias for "all"
- `"0"`, `"1"`, etc.: Specific display index

**Implementation with PyAutoGUI:**

```python
from screeninfo import get_monitors

def get_mouse_target(config):
    monitors = get_monitors()
    mode = config["display"]["multi_monitor_mode"]

    if mode == "primary":
        monitor = [m for m in monitors if m.is_primary][0]
    elif mode == "all" or mode == "random":
        monitor = random.choice(monitors)
    else:
        monitor_index = int(mode)
        monitor = monitors[monitor_index]

    margin = config["orchestrator"]["mouse_margin_pixels"]
    x = random.randint(monitor.x + margin, monitor.x + monitor.width - margin)
    y = random.randint(monitor.y + margin, monitor.y + monitor.height - margin)

    return x, y
```

---

## Testing Strategy

### Multi-Layered Testing Approach

The project uses four complementary testing strategies:

#### 1. Linting and Type Checking

**Tools:** Ruff (linting + formatting), mypy (type checking)

**Scope:**
- All Python code in `tools-py/`
- Enforces PEP 8 compliance
- Checks type annotations on all generator functions
- Runs in CI on every commit

**Configuration (`.ruff.toml`):**
```toml
[lint]
select = ["E", "F", "W", "C90", "I", "N", "UP", "ANN"]
ignore = ["ANN101", "ANN102"]  # Ignore self/cls annotations

[format]
quote-style = "double"
indent-style = "space"
```

#### 2. Property-Based Testing

**Tool:** Hypothesis

**Properties Tested:**
- **Output Dimensions**: `len(output.split('\n')) == height`
- **Line Width**: `all(len(line) <= width for line in output.split('\n'))`
- **No Out-of-Bounds**: No IndexError or similar exceptions
- **Valid Palette**: Returned palette in `{'astronomy', 'physics', 'chemistry'}`
- **Deterministic State**: Calling same generator twice with same state yields same output

**Example Test:**

```python
from hypothesis import given, strategies as st
import art_generators.solar_system as solar_system

@given(
    width=st.integers(min_value=40, max_value=200),
    height=st.integers(min_value=20, max_value=60)
)
def test_solar_system_properties(width, height):
    output, palette = solar_system.generate(width, height)

    lines = output.split('\n')
    assert len(lines) == height, "Output height mismatch"
    assert all(len(line) <= width for line in lines), "Line exceeds width"
    assert palette in {'astronomy', 'physics', 'chemistry'}, "Invalid palette"
```

#### 3. Snapshot Testing

**Tool:** pytest-snapshot

**Scope:** Deterministic generators only (those without time-based randomness)

**Process:**
1. Generate output with fixed dimensions (80x24)
2. Store as golden snapshot
3. On subsequent runs, compare output to snapshot
4. Flag any differences as potential regressions

**Example:**

```python
def test_constellation_map_snapshot(snapshot):
    # Seed random for determinism
    random.seed(42)

    output, palette = constellation_map.generate(80, 24)
    snapshot.assert_match(output, "constellation_map_80x24.txt")
```

**Applicable Generators:** `constellation_map`, `gravitational_lensing`, `moon_phases` (with fixed date)

#### 4. Benchmark Threshold Tests

**Purpose:** Ensure no generator regresses in performance

**Thresholds:**
- Execution time: ≤100ms (matches runtime timeout)
- Memory delta: ≤100MB (matches runtime limit)

**Implementation:**

```python
import pytest
import time
import tracemalloc

@pytest.mark.parametrize("generator_name", [
    "solar_system", "galaxy_collision", "black_hole", # ... all 25
])
def test_generator_performance(generator_name):
    module = __import__(f"art_generators.{generator_name}", fromlist=['generate'])

    # Measure time
    start = time.perf_counter()
    output, palette = module.generate(120, 40)
    elapsed_ms = (time.perf_counter() - start) * 1000

    assert elapsed_ms <= 100, f"{generator_name} took {elapsed_ms:.1f}ms (limit: 100ms)"

    # Memory measurement (simplified; actual uses tracemalloc)
    # ... memory assertion
```

**CI Integration:**
- Runs on every PR
- Fails build if any generator exceeds thresholds
- Generates performance report as GitHub Actions artifact

#### 5. Visual Regression Testing

**Tool:** Custom image comparison using Pillow + pixelmatch

**Process:**
1. Render generator to terminal
2. Screenshot terminal using `playwright` or similar
3. Convert to PNG
4. Compare against baseline image using perceptual diff
5. Flag differences above threshold (e.g., >5% pixel difference)

**Challenges:**
- Terminal rendering varies by emulator, font, color support
- Requires containerized environment for consistent screenshots
- Slower than text-based snapshot testing

**Status:** Planned but not initially implemented due to infrastructure complexity

---

## System Requirements

### Runtime Environment

**Required:**
- **Python Version:** 3.13.5 or higher with free-threading support
  - Must be built with `--disable-gil` flag (free-threading build)
  - Check: `python -c "import sys; print(sys.flags.nogil)"`
- **Operating System:** macOS ARM64 (primary), Linux x64 (planned), Windows (future)
- **Package Manager:** Pixi 0.9.3+

**Terminal Requirements:**

| Requirement | Minimum | Recommended | Notes |
|-------------|---------|-------------|-------|
| **Color Support** | 256-color | True color (24-bit) | Degrades gracefully to 256-color |
| **Unicode** | UTF-8 basic | Full Unicode | Some generators use extended Unicode |
| **Dimensions** | 80x24 | 120x40+ | Generators may not display well below minimum |
| **Terminal Emulator** | Any ANSI-compatible | iTerm2, Alacritty, WezTerm | Better color and Unicode support |

**Tested Terminals:**
- ✅ iTerm2 (macOS) - Full support
- ✅ Alacritty (macOS/Linux) - Full support
- ✅ WezTerm (macOS/Linux) - Full support
- ⚠️  Terminal.app (macOS) - Limited Unicode support
- ❓ Windows Terminal - Untested

### Dependencies

Managed via Pixi with conda-forge channel:

| Package | Version | Purpose | Notes |
|---------|---------|---------|-------|
| **python-freethreading** | ≥3.13.5 | Free-threading runtime | Core requirement |
| **pyautogui** | ≥0.9.54 | Mouse automation | Cross-platform, may need permissions |
| **rich** | ≥13.7.1 | Terminal rendering | Provides Live display and color |
| **screeninfo** | ≥0.8.1 | Multi-monitor support | Detect display geometry |
| **tomli** | ≥2.0.1 | TOML config parsing | Python 3.11+ has builtin tomllib |
| **art** | ≥6.2 | ASCII art utilities | Currently unused, future expansion |
| **ruff** | ≥0.14.1 | Linting and formatting | Development only |

### Platform-Specific Notes

**macOS ARM64 (Primary Platform):**
- Fully supported
- PyAutoGUI requires Accessibility permissions
- Enable in System Preferences → Security & Privacy → Accessibility

**Linux x64 (Planned Support):**
- Free-threading Python available via conda-forge
- PyAutoGUI requires X11 or Wayland
- May need `python3-tk` and `python3-dev` packages

**Windows (Future Support):**
- Free-threading Python builds available but less tested
- PyAutoGUI should work out-of-box
- Terminal color support varies (Windows Terminal recommended)

### Installation

1. **Install Pixi** (if not already installed):
   ```bash
   curl -fsSL https://pixi.sh/install.sh | bash
   ```

2. **Clone Repository**:
   ```bash
   git clone <repository-url>
   cd ascii-viz
   ```

3. **Install Dependencies**:
   ```bash
   pixi install
   ```

4. **Grant Permissions** (macOS only):
   - System Preferences → Security & Privacy → Accessibility
   - Add terminal emulator to allowed apps

5. **Run**:
   ```bash
   pixi run sim
   ```

---

## User Experience

### Interactive Menu (Default Mode)

When run without arguments (`pixi run sim`), display a simple numbered menu:

```
╔════════════════════════════════════════════════════════╗
║     ASCII Visualization Tool - Main Menu              ║
╚════════════════════════════════════════════════════════╝

Select a mode:

  1) Random Cycle Mode
     → Cycle through all visualizations with mouse movement

  2) Visualization Only (--no-mouse)
     → Display visualizations without mouse automation

  3) Mouse Only (--headless)
     → Move mouse periodically without visualization

  4) Benchmark Mode (--benchmark)
     → Test performance of all generators

  5) Exit

Enter choice (1-5): _
```

**Implementation:**
- Simple `input()` prompt (no TUI library needed)
- Invalid input shows error and re-prompts
- Ctrl+C exits immediately

### Exit Behavior

On `Ctrl+C` or `SIGTERM`:

1. **Immediate Exit**: Stop rendering loop immediately (no cleanup delay)
2. **Display Statistics** (printed to stdout):

```
╔════════════════════════════════════════════════════════╗
║                Session Statistics                      ║
╚════════════════════════════════════════════════════════╝

Runtime: 5m 23s
Generators Shown: 21
Most Common: galaxy_collision (3 times)
Total Frames Rendered: 323
Average FPS: 15.2

Thank you for using ASCII Visualization Tool!
```

3. **Exit Code**: 0 (success)

**No Cleanup:**
- Terminal state is NOT restored (screen not cleared)
- Cursor position is NOT reset
- Colors are NOT reset
- Rationale: Faster exit, user can see last frame, `clear` command available

### Error Messages

All error messages go to stderr with consistent format:

```
ERROR: <generator_name> - <error_type>: <description>
  ↳ Falling back to starfield

CRITICAL: Starfield failed - <error_type>: <description>
  ↳ Using minimal renderer (single dot)

FATAL: All rendering systems failed
  ↳ Exiting with code 1
```

**Examples:**

```
ERROR: galaxy_collision - TimeoutError: Exceeded 100ms limit (actual: 127ms)
  ↳ Falling back to starfield

ERROR: molecular_dynamics - MemoryError: Allocated 156MB (limit: 100MB)
  ↳ Falling back to starfield

CRITICAL: Starfield failed - AttributeError: 'NoneType' object has no attribute 'generate'
  ↳ Using minimal renderer (single dot)
```

### Logging Strategy

**Minimal Logging:** Only errors to stderr, no log files

**Rationale:**
- Terminal output should be clean (visualization only)
- Errors are rare and should be immediately visible
- No silent failures or hidden log files
- Users can redirect stderr if needed: `pixi run sim 2> errors.log`

**No Logging:**
- Generator switches
- Performance metrics (unless `--benchmark`)
- Mouse movements
- Configuration loading
- State persistence

---

## Security & Quality

### Security Model

**Threat Model:**
- Generators are trusted internal code, not user plugins
- All generators undergo code review before merge
- No remote code execution, no network access
- No file system access (generators are pure computation)

**Defense Layers:**

1. **Code Review:**
   - All PRs require approval
   - Focus on: no `os.system()`, `eval()`, `subprocess`, file I/O

2. **Resource Limits:**
   - 100ms timeout prevents infinite loops
   - 100MB memory cap prevents memory exhaustion
   - Limits catch bugs and malicious code alike

3. **No External Plugins:**
   - Internal-only generator system
   - No pip packages with "ascii-viz-" prefix
   - No `~/.config/ascii-viz/generators/` directory

**CI Security Scanning:**
- Not implemented (reliance on code review + resource limits)
- Rationale: Internal code, limited attack surface, resource limits sufficient

### Code Quality Standards

**Linting:** Ruff with strict settings
- PEP 8 compliance
- No unused imports or variables
- Consistent naming conventions

**Type Checking:** mypy in strict mode
- All function signatures typed
- Generator interface enforced: `def generate(width: int, height: int) -> tuple[str, str]`

**Documentation:** Simple docstrings required
- Explain what is visualized
- Note any scientific accuracy considerations
- No need for parameter docs (width/height are obvious)

**Testing:** All four testing strategies required
- Property-based tests for all generators
- Snapshot tests for deterministic generators
- Benchmark tests prevent performance regressions
- CI runs on every PR, blocks merge on failure

### CI/CD Pipeline

**GitHub Actions Workflow:**

```yaml
name: CI

on: [push, pull_request]

jobs:
  test:
    runs-on: macos-latest  # Free-threading Python available
    steps:
      - uses: actions/checkout@v3

      - name: Install Pixi
        run: curl -fsSL https://pixi.sh/install.sh | bash

      - name: Install Dependencies
        run: pixi install

      - name: Lint
        run: pixi run lint

      - name: Type Check
        run: pixi run typecheck

      - name: Property-Based Tests
        run: pixi run test-properties

      - name: Snapshot Tests
        run: pixi run test-snapshots

      - name: Benchmark Tests
        run: pixi run test-benchmarks

      - name: Upload Benchmark Report
        uses: actions/upload-artifact@v3
        with:
          name: benchmark-report
          path: benchmark-results.json
```

**Required Checks for Merge:**
- ✅ Linting passes
- ✅ Type checking passes
- ✅ All property-based tests pass
- ✅ All benchmark tests pass (no regressions)
- ✅ Snapshot tests pass (or explicitly updated)

---

## Implementation Roadmap

### Current Status: 25 of 25 Visualizations Complete (100%)

All planned visualizations from `visuals.md` have been implemented:

**Astronomy & Cosmology (10/10):**
- ✅ Solar System
- ✅ Galaxy Collision
- ✅ Black Hole
- ✅ Pulsar
- ✅ Gravitational Lensing
- ✅ Constellation Map
- ✅ Moon Phases
- ✅ Exoplanet Transit
- ✅ Cosmic Microwave Background
- ✅ Stellar Nursery

**Space Exploration (5/5):**
- ✅ Rocket Launch
- ✅ Satellite Orbit
- ✅ Mars Rover
- ✅ Voyager's Journey
- ✅ Moon Landing

**Physics & Chemistry (10/10):**
- ✅ Particle Accelerator
- ✅ Molecular Dynamics
- ✅ Wave Interference
- ✅ Double Pendulum
- ✅ Brownian Motion
- ✅ Spacetime Grid
- ✅ Radioactive Decay
- ✅ Doppler Effect
- ✅ Crystal Growth
- ✅ Chemical Reaction

### Pending Architecture Enhancements

Based on this specification, the following changes need implementation:

#### Phase 1: Core Architecture (High Priority)

1. **Color Palette System**
   - Implement thematic palette mapping (`astronomy`, `physics`, `chemistry`)
   - Update generator contract to return `tuple[str, str]` (art, palette)
   - Implement temperature-based color ranges
   - Update all 25 generators to specify palettes

2. **Operation Modes**
   - Add `--no-mouse`, `--headless`, `--benchmark` flags
   - Implement interactive menu for default mode
   - Add mode selection logic in orchestrator

3. **Resource Limits**
   - Implement 100ms timeout with signal
   - Add memory monitoring with tracemalloc
   - Implement three-level fallback system
   - Create minimal "single dot" renderer

4. **Free-Threading Parallelization**
   - Implement pre-rendering of next generator
   - Add parallel grid operations for large grids
   - Optimize generator internal computations for parallelism
   - Benchmark performance improvements

#### Phase 2: Configuration & UX (Medium Priority)

5. **Configuration System**
   - Implement `.ascii-viz.toml` loading
   - Define configuration schema
   - Add per-generator parameter overrides
   - Implement multi-monitor configuration

6. **Interactive Menu**
   - Create simple numbered menu interface
   - Add mode selection logic
   - Implement input validation

7. **Exit Statistics**
   - Track session metrics (runtime, generators shown, FPS)
   - Display on Ctrl+C
   - Implement graceful shutdown

#### Phase 3: Testing Infrastructure (Medium Priority)

8. **Property-Based Tests**
   - Set up Hypothesis
   - Write property tests for all 25 generators
   - Integrate into CI

9. **Snapshot Tests**
   - Set up pytest-snapshot
   - Identify deterministic generators
   - Create golden snapshots

10. **Benchmark Tests**
    - Implement performance measurement
    - Set up threshold assertions
    - Create JSON benchmark report

11. **CI/CD Pipeline**
    - Configure GitHub Actions workflow
    - Add linting, type checking, testing jobs
    - Set up required checks for PR merge

#### Phase 4: Documentation & Polish (Low Priority)

12. **Documentation Updates**
    - Update the project guide with new architecture details
    - Add configuration examples
    - Document testing strategy

13. **Error Handling**
    - Implement consistent error message format
    - Add stderr logging for errors
    - Test fallback system thoroughly

14. **Multi-Platform Support**
    - Test on Linux x64
    - Update `pixi.toml` platforms
    - Document platform-specific requirements

### Future Enhancements (Not Planned)

The following were considered but explicitly rejected for simplicity:

- ❌ Visual regression testing (infrastructure complexity)
- ❌ Disk-based state persistence (added complexity, unclear benefit)
- ❌ External plugin system (security, quality control)
- ❌ Export to GIF/video formats (scope creep)
- ❌ Interactive controls during runtime (complexity, not aligned with screensaver use case)
- ❌ Telemetry or performance logging (keep it simple)
- ❌ Built-in feedback mechanism (prefer manual GitHub issues)
- ❌ Advanced TUI menu with arrow keys (overkill for 5 options)
- ❌ Configurable per-generator state persistence (too complex)
- ❌ Generator lifecycle hooks (init/cleanup) (keep simple contract)

---

## Appendix A: Generator Contract Full Specification

### Function Signature

```python
def generate(width: int, height: int) -> tuple[str, str]:
    """
    Generate ASCII art for given terminal dimensions.

    Args:
        width: Terminal width in characters (range: 40-500)
        height: Terminal height in characters (range: 20-200)

    Returns:
        Tuple containing:
        - ascii_art (str): Multi-line string with newline-separated rows
          - Must have exactly `height` lines
          - Each line must have ≤ `width` characters (padding with spaces is OK)
          - Characters beyond width will be truncated
        - palette_name (str): One of 'astronomy', 'physics', 'chemistry'

    Raises:
        No exceptions should be raised. Handle errors internally and return
        a degraded output rather than crashing.

    Performance Requirements:
        - Execution time: ≤100ms (enforced by timeout)
        - Memory allocation: ≤100MB delta (enforced by monitoring)
        - Must complete deterministically (no network calls, no file I/O)

    State Management:
        - Use function attributes for animation state:
          if not hasattr(generate, "state"):
              generate.state = initialize_state()
        - State persists across calls in the same process
        - No external state (files, databases, global variables)

    Coordinate System:
        - Origin (0, 0) is top-left
        - X increases right, Y increases down
        - Characters are ~2:1 aspect ratio (taller than wide)
        - Apply 2.5x horizontal scaling for circular shapes:
          x = center_x + int(radius * math.cos(angle) * 2.5)
          y = center_y + int(radius * math.sin(angle))

    Example Implementation:
        def generate(width: int, height: int) -> tuple[str, str]:
            # Initialize state
            if not hasattr(generate, "frame"):
                generate.frame = 0
            generate.frame += 1

            # Create grid
            grid = [[" " for _ in range(width)] for _ in range(height)]

            # Draw something (example: moving dot)
            x = (generate.frame % width)
            y = height // 2
            grid[y][x] = "*"

            # Convert to string
            art = "\n".join("".join(row) for row in grid)

            return art, "astronomy"
    """
```

### Validation

Generators are automatically validated in CI:

```python
def validate_generator(generator_func, width=120, height=40):
    """Validate a generator against the contract."""

    # Test execution
    try:
        output, palette = generator_func(width, height)
    except Exception as e:
        raise AssertionError(f"Generator raised exception: {e}")

    # Validate return type
    assert isinstance(output, str), "Output must be string"
    assert isinstance(palette, str), "Palette must be string"

    # Validate palette
    assert palette in {'astronomy', 'physics', 'chemistry'}, \
        f"Invalid palette: {palette}"

    # Validate dimensions
    lines = output.split('\n')
    assert len(lines) == height, \
        f"Expected {height} lines, got {len(lines)}"

    for i, line in enumerate(lines):
        assert len(line) <= width, \
            f"Line {i} has {len(line)} chars (max: {width})"

    # Validate performance (separate test)
    # ... see benchmark tests
```

---

## Appendix B: Configuration File Examples

### Minimal Configuration

```toml
# .ascii-viz.toml (minimal)

[orchestrator]
cycle_duration_seconds = 20  # Override default 15 seconds
```

### Full Configuration with All Options

```toml
# .ascii-viz.toml (comprehensive)

[orchestrator]
cycle_duration_seconds = 15
mouse_margin_pixels = 50
mouse_movement_duration = 0.5
default_palette = "astronomy"  # Fallback if generator doesn't specify

[resource_limits]
timeout_ms = 100
memory_limit_mb = 100

[display]
multi_monitor_mode = "primary"  # Options: "primary", "all", "random", "0", "1", etc.
preferred_terminal_size = [120, 40]

[categories]
enabled = ["astronomy", "physics"]  # Disable chemistry category

[generator_overrides]
# Per-generator parameter overrides

[generator_overrides.galaxy_collision]
particle_count = 750
collision_speed = 2.0
galaxy_a_mass = 1.0
galaxy_b_mass = 0.8

[generator_overrides.black_hole]
particle_count = 1000
rotation_speed = 3.0
accretion_rate = 0.5

[generator_overrides.solar_system]
planet_count = 8
orbit_speed_multiplier = 1.5
show_asteroid_belt = true
```

### Team Configuration for Presentations

```toml
# .ascii-viz.toml (presentation mode)

[orchestrator]
cycle_duration_seconds = 30  # Longer time per visualization for explanations
mouse_margin_pixels = 100    # Keep cursor away from edges on projector

[display]
multi_monitor_mode = "all"   # Move across both presenter display and projector

[categories]
enabled = ["astronomy"]      # Only show astronomy for astronomy talk

[generator_overrides]
# Slower, more visible animations for large screen
[generator_overrides.solar_system]
orbit_speed_multiplier = 0.5

[generator_overrides.galaxy_collision]
collision_speed = 0.75
```

---

## Appendix C: Performance Benchmarks

### Expected Performance (120x40 terminal, Apple M1 MacBook)

| Generator | Execution Time | Memory Delta | Complexity | Notes |
|-----------|----------------|--------------|------------|-------|
| starfield | 2-5ms | <1MB | Low | Baseline, simplest generator |
| constellation_map | 5-10ms | <1MB | Low | Static positions, Bresenham lines |
| moon_phases | 8-12ms | <1MB | Low | Simple circle drawing |
| pulsar | 10-15ms | <1MB | Low | Time-based rotation, minimal state |
| solar_system | 15-25ms | 1-2MB | Medium | 8 planets, Keplerian orbits |
| gravitational_lensing | 20-30ms | 1-2MB | Medium | Ray tracing, but sparse |
| exoplanet_transit | 20-35ms | 1-2MB | Medium | Light curve computation |
| black_hole | 40-60ms | 5-10MB | High | 500-1000 particles, Keplerian rotation |
| galaxy_collision | 50-80ms | 8-15MB | High | Two galaxies, 500+ particles each |
| molecular_dynamics | 60-90ms | 10-20MB | High | Particle physics, collision detection |
| brownian_motion | 30-50ms | 3-5MB | Medium | Random walk particles |
| stellar_nursery | 70-95ms | 12-18MB | High | Dense particle field, gas dynamics |

**Notes:**
- All measurements on Apple M1 MacBook Pro, 16GB RAM
- Times are for single-threaded execution (pre-parallelization)
- Free-threading should reduce high-complexity generators by 2-3x
- Memory delta includes all allocations within `generate()` call

### Performance Optimization Checklist

When a generator exceeds 100ms:

1. **Profile with cProfile**
   ```python
   import cProfile
   cProfile.run('generator.generate(120, 40)', sort='cumulative')
   ```

2. **Common Bottlenecks:**
   - Nested loops over grid (O(width × height))
   - Unnecessary list copies or concatenations
   - Complex math in inner loops (move to numpy if applicable)
   - Excessive random number generation

3. **Optimization Strategies:**
   - Use sparse data structures (only store non-empty cells)
   - Pre-compute expensive values (sin/cos tables)
   - Reduce particle count or grid resolution
   - Parallelize with free-threading (split grid into chunks)

4. **Acceptable Tradeoffs:**
   - Lower particle counts (500 → 300)
   - Coarser timesteps in physics simulations
   - Simplified rendering (fewer characters, simpler shapes)

---

## Appendix D: Troubleshooting Guide

### Common Issues

#### Generator Exceeds Timeout

**Symptom:** `ERROR: <generator> - TimeoutError: Exceeded 100ms limit`

**Solutions:**
1. Reduce particle count in generator
2. Optimize inner loops (use list comprehensions, avoid nested loops)
3. Pre-compute expensive calculations
4. Use sparse data structures

#### Memory Limit Exceeded

**Symptom:** `ERROR: <generator> - MemoryError: Allocated XMB (limit: 100MB)`

**Solutions:**
1. Reduce particle count or grid density
2. Check for memory leaks (state not being cleaned up)
3. Use generators/iterators instead of lists for large datasets
4. Clear temporary data structures after use

#### PyAutoGUI Permission Denied (macOS)

**Symptom:** `RuntimeError: pyautogui.moveTo() failed: accessibility permissions not granted`

**Solution:**
1. System Preferences → Security & Privacy → Privacy → Accessibility
2. Add your terminal emulator to the list
3. Toggle the checkbox off and on again if it's already there
4. Restart terminal

#### Colors Not Displaying Correctly

**Symptom:** Colors are wrong, washed out, or monochrome

**Solutions:**
1. Check terminal supports true color: `echo $COLORTERM` should be `truecolor`
2. Try a different terminal emulator (iTerm2, Alacritty, WezTerm)
3. Update Rich library: `pixi update rich`

#### Unicode Characters Not Rendering

**Symptom:** Boxes, question marks, or missing characters

**Solutions:**
1. Ensure terminal uses UTF-8: `locale` should show `UTF-8`
2. Install a font with broad Unicode support (Nerd Fonts, Fira Code)
3. Check terminal emulator settings for UTF-8 encoding

#### Free-Threading Not Available

**Symptom:** `AttributeError: module 'sys' has no attribute 'nogil'`

**Solution:**
1. Verify Python version: `python --version` should be ≥3.13.5
2. Check free-threading build: `python -c "import sys; print(sys.flags.nogil)"`
3. Reinstall with Pixi: `pixi clean && pixi install`

---

## Document Revision History

| Version | Date | Author | Changes |
|---------|------|--------|---------|
| 1.0 | 2026-01-18 | Initial | Original specification from project |
| 2.0 | 2026-01-18 | Interview | Complete rewrite based on in-depth interview: added color palette system, operation modes, resource limits, configuration system, testing strategy, full technical architecture |

---

**End of Specification**
