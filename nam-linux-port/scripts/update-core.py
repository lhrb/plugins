#!/usr/bin/env python3
"""Refresh the unchanged vendor snapshot at an explicit upstream commit.

Run on a review branch, then run the complete verification suite before merging.
--check only reports whether upstream main differs from the pinned revision.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
CORE_URL = "https://github.com/sdatkinson/NeuralAmpModelerCore.git"
EIGEN_URL = "https://gitlab.com/libeigen/eigen.git"


def git(*args, cwd=None):
    return subprocess.check_output(["git", *args], cwd=cwd, text=True).strip()


def checkout(url, revision, directory):
    git("init", str(directory))
    git("remote", "add", "origin", url, cwd=directory)
    git("fetch", "--depth=1", "origin", revision, cwd=directory)
    git("checkout", "--detach", "FETCH_HEAD", cwd=directory)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("revision", nargs="?", help="full upstream core commit SHA")
    parser.add_argument("--check", action="store_true", help="read-only upstream drift check")
    args = parser.parse_args()
    lock_path = ROOT / "upstream.lock.json"
    old = json.loads(lock_path.read_text())
    if args.check:
        latest = git("ls-remote", CORE_URL, "refs/heads/main").split()[0]
        print(f"Pinned:   {old['core']}\nUpstream: {latest}")
        if latest != old["core"]:
            print("Update available. Run scripts/update-core.py <full-SHA> on a review branch.")
            return 1
        return 0
    if not args.revision or not re.fullmatch(r"[0-9a-f]{40}", args.revision):
        parser.error("supply an explicit full 40-character commit SHA")
    with tempfile.TemporaryDirectory(prefix="nam-update-") as temp:
        temp = Path(temp)
        source, eigen, staged = temp / "core", temp / "eigen", temp / "snapshot"
        checkout(CORE_URL, args.revision, source)
        eigen_pin = git("ls-tree", "HEAD", "Dependencies/eigen", cwd=source).split()[2]
        checkout(EIGEN_URL, eigen_pin, eigen)
        staged.mkdir()
        for path in ["NAM", "LICENSE", "README.md", "CMakeLists.txt", "tools/CMakeLists.txt"]:
            src, dst = source / path, staged / path
            dst.parent.mkdir(parents=True, exist_ok=True)
            if src.is_dir():
                shutil.copytree(src, dst)
            else:
                shutil.copy2(src, dst)
        dependency = staged / "Dependencies/eigen"
        dependency.mkdir(parents=True)
        for path in [eigen / "Eigen", eigen / "unsupported", *eigen.glob("COPYING*")]:
            if path.is_dir():
                shutil.copytree(path, dependency / path.name)
            else:
                shutil.copy2(path, dependency / path.name)
        json_dir = staged / "Dependencies/nlohmann"
        json_dir.mkdir(parents=True)
        header = (source / "Dependencies/nlohmann/json.hpp").read_bytes()
        (json_dir / "json.hpp").write_bytes(header)
        version = ".".join(re.search(rb"#define NLOHMANN_JSON_VERSION_" + part + rb"\s+(\d+)", header)[1].decode()
                           for part in [b"MAJOR", b"MINOR", b"PATCH"])
        license_url = f"https://raw.githubusercontent.com/nlohmann/json/v{version}/LICENSE.MIT"
        with urllib.request.urlopen(license_url, timeout=30) as response:
            (json_dir / "LICENSE.MIT").write_bytes(response.read())
        pins = {"core": args.revision, "eigen": eigen_pin, "json_version": version,
                "json_sha256": hashlib.sha256(header).hexdigest()}
        # Downloads and staging must complete before touching the working snapshot.
        destination = ROOT / "vendor/NeuralAmpModelerCore"
        shutil.rmtree(destination)
        shutil.copytree(staged, destination)
        lock_path.write_text(json.dumps(pins, indent=2) + "\n")
        notes = (ROOT / "UPSTREAM.md").read_text()
        for key in ["core", "eigen", "json_sha256", "json_version"]:
            notes = notes.replace(old[key], pins[key])
        (ROOT / "UPSTREAM.md").write_text(notes)
    print("Snapshot refreshed. Review UPSTREAM.md, dependency licenses and native/CMakeLists.txt.")
    print("Run cargo test --locked --all-targets, native allocation tests and DAW smoke tests.")
    print("Commit vendor/, upstream.lock.json and UPSTREAM.md together. Never auto-merge a core update.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
