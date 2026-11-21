"""Collect the license texts of the Cargo packages used for this Mac build."""
from collections import defaultdict
import hashlib
import json
from pathlib import Path
import subprocess
import sys


def collect(destination: Path) -> None:
    target = subprocess.check_output(["rustc", "-vV"], text=True)
    host = next(line.split(": ", 1)[1] for line in target.splitlines() if line.startswith("host: "))
    metadata = json.loads(subprocess.check_output([
        "cargo", "metadata", "--locked", "--format-version", "1", "--filter-platform", host
    ]))
    names = defaultdict(list)
    bodies = {}
    for package in sorted(metadata["packages"], key=lambda entry: entry["name"]):
        if package["source"] is None:
            continue
        root = Path(package["manifest_path"]).parent
        files = [f for f in root.iterdir() if f.is_file() and
                 f.name.upper().startswith(("LICENSE", "LICENCE", "COPYING", "NOTICE", "COPYRIGHT"))]
        if package["license_file"]:
            files.append(root / package["license_file"])
        if not files:
            raise ValueError(f"Missing license text for {package['name']}")
        for file in sorted(set(files)):
            body = file.read_text(encoding="utf-8", errors="replace").strip()
            digest = hashlib.sha256(body.encode()).digest()
            bodies[digest] = body
            names[digest].append(f"{package['name']} {package['version']} / {file.name}")
    with destination.open("w", encoding="utf-8") as output:
        for digest, packages in names.items():
            output.write("\n".join(packages) + "\n\n" + bodies[digest] + "\n\n")


if __name__ == "__main__":
    collect(Path(sys.argv[1]))
