import math


def draw_line(grid, x1, y1, x2, y2, char_func):
    """Bresenham's line algorithm for drawing pendulum arms."""
    dx = abs(x2 - x1)
    dy = abs(y2 - y1)
    sx = 1 if x1 < x2 else -1
    sy = 1 if y1 < y2 else -1
    err = dx - dy

    while True:
        if 0 <= y1 < len(grid) and 0 <= x1 < len(grid[0]):
            # Use different characters based on angle
            angle = math.atan2(y2 - y1, x2 - x1)
            grid[y1][x1] = char_func(angle)

        if x1 == x2 and y1 == y2:
            break

        e2 = 2 * err
        if e2 > -dy:
            err -= dy
            x1 += sx
        if e2 < dx:
            err += dx
            y1 += sy


def get_arm_char(angle):
    """Get arm character based on angle."""
    # Convert to degrees
    deg = math.degrees(angle) % 180
    if 45 <= deg < 135:
        return "│"  # Vertical
    elif deg < 45 or deg >= 135:
        if -0.4 < angle < 0.4 or 2.7 < angle or angle < -2.7:
            return "─"  # Horizontal
        elif angle > 0:
            return "╲"  # Down-right
        else:
            return "╱"  # Down-left
    else:
        return "│"


def double_pendulum_derivatives(state, L1, L2, m1, m2, g):
    """Calculate derivatives for double pendulum using Lagrangian mechanics.

    Returns [dθ1/dt, dθ2/dt, dω1/dt, dω2/dt]
    """
    theta1, theta2, omega1, omega2 = state

    delta = theta2 - theta1

    # Denominators
    den1 = (m1 + m2) * L1 - m2 * L1 * math.cos(delta) * math.cos(delta)
    den2 = (L2 / L1) * den1

    # dω1/dt
    domega1 = (
        m2 * L1 * omega1 * omega1 * math.sin(delta) * math.cos(delta)
        + m2 * g * math.sin(theta2) * math.cos(delta)
        + m2 * L2 * omega2 * omega2 * math.sin(delta)
        - (m1 + m2) * g * math.sin(theta1)
    ) / den1

    # dω2/dt
    domega2 = (
        -m2 * L2 * omega2 * omega2 * math.sin(delta) * math.cos(delta)
        + (m1 + m2)
        * (
            g * math.sin(theta1) * math.cos(delta)
            - L1 * omega1 * omega1 * math.sin(delta)
            - g * math.sin(theta2)
        )
    ) / den2

    return [omega1, omega2, domega1, domega2]


def rk4_step(state, dt, L1, L2, m1, m2, g):
    """Runge-Kutta 4th order integration step."""
    k1 = double_pendulum_derivatives(state, L1, L2, m1, m2, g)

    state2 = [state[i] + 0.5 * dt * k1[i] for i in range(4)]
    k2 = double_pendulum_derivatives(state2, L1, L2, m1, m2, g)

    state3 = [state[i] + 0.5 * dt * k2[i] for i in range(4)]
    k3 = double_pendulum_derivatives(state3, L1, L2, m1, m2, g)

    state4 = [state[i] + dt * k3[i] for i in range(4)]
    k4 = double_pendulum_derivatives(state4, L1, L2, m1, m2, g)

    # Weighted average
    new_state = [
        state[i] + (dt / 6.0) * (k1[i] + 2 * k2[i] + 2 * k3[i] + k4[i])
        for i in range(4)
    ]

    return new_state


def generate(width, height):
    """Generates a chaotic double pendulum simulation.

    Shows two connected pendulum arms with realistic physics and a
    fading trail showing the complex, unpredictable motion.
    """
    # Initialize pendulum state
    if not hasattr(generate, "state"):
        # State: [theta1, theta2, omega1, omega2]
        # Start with small displacement
        generate.state = [math.pi / 2 + 0.5, math.pi / 2 + 0.3, 0.0, 0.0]
        generate.trail = []

    # Physical parameters
    L1 = min(width // 4, height // 3)  # Length of first arm
    L2 = min(width // 4, height // 3)  # Length of second arm
    m1 = 1.0  # Mass 1
    m2 = 1.0  # Mass 2
    g = 9.81  # Gravity

    # Time step
    dt = 0.05

    # Update pendulum state using RK4
    generate.state = rk4_step(generate.state, dt, L1, L2, m1, m2, g)

    theta1, theta2, omega1, omega2 = generate.state

    grid = [[" " for _ in range(width)] for _ in range(height)]

    # Pivot point
    pivot_x = width // 2
    pivot_y = height // 4

    # Position of first mass
    x1 = pivot_x + int(L1 * math.sin(theta1) * 2.5)
    y1 = pivot_y + int(L1 * math.cos(theta1))

    # Position of second mass
    x2 = x1 + int(L2 * math.sin(theta2) * 2.5)
    y2 = y1 + int(L2 * math.cos(theta2))

    # Add to trail
    if 0 <= y2 < height and 0 <= x2 < width:
        generate.trail.append((x2, y2))

    # Limit trail length
    max_trail = 200
    if len(generate.trail) > max_trail:
        generate.trail.pop(0)

    # Draw trail with fading using ·
    trail_len = len(generate.trail)
    for idx, (tx, ty) in enumerate(generate.trail):
        if 0 <= ty < height and 0 <= tx < width:
            # Only draw fading trail (sparse)
            fade_factor = idx / trail_len
            # Only draw every few trail points for sparsity
            if fade_factor > 0.3 and idx % 2 == 0:
                if grid[ty][tx] == " ":
                    grid[ty][tx] = "·"

    # Draw first arm using │╲╱─
    if 0 <= y1 < height and 0 <= x1 < width:
        draw_line(grid, pivot_x, pivot_y, x1, y1, get_arm_char)

    # Draw second arm using │╲╱─
    if 0 <= y1 < height and 0 <= x1 < width and 0 <= y2 < height and 0 <= x2 < width:
        draw_line(grid, x1, y1, x2, y2, get_arm_char)

    # Draw pivot as ※
    if 0 <= pivot_y < height and 0 <= pivot_x < width:
        grid[pivot_y][pivot_x] = "※"

    # Draw first mass (joint) as ●
    if 0 <= y1 < height and 0 <= x1 < width:
        grid[y1][x1] = "●"

    # Draw second mass (end mass) as ●
    if 0 <= y2 < height and 0 <= x2 < width:
        grid[y2][x2] = "●"

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
