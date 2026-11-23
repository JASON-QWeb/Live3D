"""Model downloads that report progress to the Vorela console."""

import json
import os
import sys
import threading

from huggingface_hub import snapshot_download
from tqdm import tqdm


def report(stage, **fields):
    """Replace the progress file the console polls. Progress is best effort."""
    path = os.environ.get("VORELA_PROGRESS")
    if not path:
        return
    try:
        partial = f"{path}.tmp"
        with open(partial, "w", encoding="utf-8") as f:
            json.dump({"stage": stage, **fields}, f)
        os.replace(partial, path)
    except OSError:
        pass


def fetch(repo, **options):
    """snapshot_download that reports received bytes while it runs."""
    bars, counters = [], []

    class Meter(tqdm):
        def __init__(self, *args, **kwargs):
            kwargs.setdefault("mininterval", 5)
            super().__init__(*args, **kwargs)
            (bars if getattr(self, "unit", None) == "B" else counters).append(self)

    # snapshot_download opens one byte bar for network transfer and one for
    # bytes written. Xet writes a file only after receiving all of it, so the
    # larger count is the one that moves. The smaller total is the file sizes;
    # the transfer total can be padded once it overshoots. Totals grow as each
    # file starts, so hold below 100% until every file is done.
    def measure():
        totals = [int(bar.total) for bar in bars if bar.total]
        total = min(totals, default=0)
        done = max((int(bar.n) for bar in bars), default=0)
        files = next(((int(c.n), int(c.total or 0)) for c in counters), (0, 0))
        if total:
            done = min(done, total if files[0] >= files[1] else total * 99 // 100)
        return done, total, files

    finished = threading.Event()

    def watch():
        # Small config files register first. Leave the total unknown until it
        # has held for 3 seconds so the bar does not flash full at the start.
        seen, steady = 0, 0
        while True:
            done, total, (files, files_total) = measure()
            steady = steady + 1 if total == seen else 0
            seen = total
            report(
                "download", model=repo, done=done, total=total if steady >= 6 else 0,
                files=files, files_total=files_total,
            )
            if finished.wait(0.5):
                return

    watcher = threading.Thread(target=watch, daemon=True)
    watcher.start()
    try:
        return snapshot_download(repo_id=repo, tqdm_class=Meter, **options)
    finally:
        finished.set()
        watcher.join()


if __name__ == "__main__":
    # Fetch a chat model with the files mlx_lm.server loads, before the server starts.
    from mlx_lm.utils import DEFAULT_ALLOW_PATTERNS

    model = sys.argv[1]
    try:
        snapshot_download(repo_id=model, allow_patterns=DEFAULT_ALLOW_PATTERNS, local_files_only=True)
    except Exception:
        fetch(model, allow_patterns=DEFAULT_ALLOW_PATTERNS)
