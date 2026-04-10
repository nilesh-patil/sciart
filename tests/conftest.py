import sys
from pathlib import Path

TOOLS_PY = Path(__file__).resolve().parent.parent / "tools-py"
if str(TOOLS_PY) not in sys.path:
    sys.path.insert(0, str(TOOLS_PY))
