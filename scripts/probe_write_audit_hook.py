#!/usr/bin/env python3
"""Standalone probe for the kernel write-audit hook.

Registers the same observer used by python/kernel_worker.py, runs a table of
representative cells, and times a compute-bound cell and a 2000-file write
loop with and without the hook. Optional numpy/matplotlib are used when
importable; stdlib-only runs skip those rows.
"""

from __future__ import annotations

import os
import sys
import tempfile
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "python"))
from kernel_worker import _WriteObserver  # noqa: E402


def _run_cell(observer: _WriteObserver | None, cwd: str, source: str) -> tuple[list[str] | None, str | None]:
    old = os.getcwd()
    os.chdir(cwd)
    try:
        if observer is not None:
            observer.begin()
        error = None
        try:
            exec(compile(source, "<probe>", "exec"), {})
        except Exception as exc:  # noqa: BLE001 — probe must survive the raise cell
            error = f"{type(exc).__name__}: {exc}"
        reported = observer.finish() if observer is not None else None
        return reported, error
    finally:
        os.chdir(old)


def _rel(root: str, paths: list[str] | None) -> str:
    if paths is None:
        return "(omitted)"
    # tempfile roots can sit behind a symlink; abspath (what the observer
    # stores) and realpath then disagree. Match either spelling.
    roots = {os.path.abspath(root), os.path.realpath(root)}
    out = []
    for path in paths:
        labeled = path
        for candidate in roots:
            prefix = candidate.rstrip(os.sep) + os.sep
            abs_path = os.path.abspath(path)
            if abs_path.startswith(prefix):
                labeled = abs_path[len(prefix) :]
                break
            real_path = os.path.realpath(path)
            real_prefix = os.path.realpath(candidate).rstrip(os.sep) + os.sep
            if real_path.startswith(real_prefix):
                labeled = real_path[len(real_prefix) :]
                break
        out.append(labeled.replace("\\", "/"))
    return ", ".join(out) if out else "(none)"


def main() -> int:
    observer = _WriteObserver()
    sys.addaudithook(observer.hook)

    with tempfile.TemporaryDirectory() as tmp:
        print("What the hook observes:")
        print()
        cells = [
            ("open('a.txt','w')", "open('a.txt', 'w').write('x')"),
            ("open('a.txt') (read)", "open('a.txt').read()"),
            ("open('a.txt','a') (append)", "open('a.txt', 'a').write('y')"),
            ("print('hello')", "print('hello')"),
        ]
        try:
            import matplotlib

            matplotlib.use("Agg")
            import matplotlib.pyplot as plt  # noqa: F401

            cells.append(
                (
                    "plt.savefig('plot.png') (C-level write)",
                    "import matplotlib\nmatplotlib.use('Agg')\n"
                    "import matplotlib.pyplot as plt\nplt.plot([1, 2, 3])\nplt.savefig('plot.png')",
                )
            )
        except Exception:
            print("matplotlib not importable; skipping savefig row")
        try:
            import numpy as np  # noqa: F401

            cells.append(
                (
                    "np.save('arr.npy', ...)",
                    "import numpy as np\nnp.save('arr.npy', np.arange(3))",
                )
            )
        except Exception:
            print("numpy not importable; skipping np.save row")
        cells.extend(
            [
                (
                    "write then raise",
                    "open('raised.txt', 'w').write('x')\nraise RuntimeError('nope')",
                ),
                (
                    "subprocess.run([...open(...,'w')...])",
                    "import subprocess, sys\n"
                    "subprocess.run([sys.executable, '-c', \"open('sub.txt','w').write('x')\"], check=True)",
                ),
            ]
        )
        for label, source in cells:
            reported, error = _run_cell(observer, tmp, source)
            suffix = f" error={error}" if error else ""
            print(f"  {label}: {_rel(tmp, reported)}{suffix}")

        print()
        print("Overhead (two runs each, alternating):")

        def compute():
            total = 0
            for i in range(2_000_000):
                total += i
            return total

        def write_many():
            for i in range(2000):
                path = os.path.join(tmp, f"bulk_{i}.txt")
                with open(path, "w", encoding="utf-8") as handle:
                    handle.write("x")

        def time_once(fn, with_hook: bool) -> float:
            if with_hook:
                observer.begin()
            t0 = time.perf_counter()
            fn()
            elapsed = time.perf_counter() - t0
            if with_hook:
                observer.finish()
            return elapsed

        compute_off = [time_once(compute, False) for _ in range(2)]
        compute_on = [time_once(compute, True) for _ in range(2)]
        writes_off = [time_once(write_many, False) for _ in range(2)]
        writes_on = [time_once(write_many, True) for _ in range(2)]
        print(
            f"  compute baseline: {compute_off[0]:.3f} s / {compute_off[1]:.3f} s"
        )
        print(f"  compute with hook: {compute_on[0]:.3f} s / {compute_on[1]:.3f} s")
        print(f"  2000 writes baseline: {writes_off[0]:.3f} s / {writes_off[1]:.3f} s")
        print(f"  2000 writes with hook: {writes_on[0]:.3f} s / {writes_on[1]:.3f} s")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
