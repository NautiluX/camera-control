#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 NautiluX
"""Prepare, package and publish versioned releases without changing main."""
import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tarfile
import tempfile
import tomllib

VERSION = re.compile(r"v?(\d+)\.(\d+)\.(\d+)")


def run(*args, **kwargs):
    return subprocess.check_output(args, text=True, **kwargs).strip()


def parse_version(value):
    match = VERSION.fullmatch(value)
    if not match:
        raise ValueError(f"Expected a stable major.minor.patch version, got {value!r}")
    return tuple(map(int, match.groups()))


def next_version(baseline, previous):
    baseline = parse_version(baseline)
    if previous is None or baseline > parse_version(previous):
        return ".".join(map(str, baseline))
    major, minor, patch = parse_version(previous)
    return f"{major}.{minor}.{patch + 1}"


def latest_release(repo):
    result = subprocess.run(["gh", "api", f"repos/{repo}/releases/latest"], capture_output=True, text=True)
    if result.returncode == 0:
        return json.loads(result.stdout)
    try:
        not_found = str(json.loads(result.stdout).get("status")) == "404"
    except (ValueError, AttributeError):
        not_found = False
    if not_found or "HTTP 404" in result.stderr:
        return None
    raise RuntimeError(result.stderr or result.stdout)


def prepare():
    repo = os.environ["GITHUB_REPOSITORY"]
    source = run("git", "rev-parse", "HEAD")
    baseline = tomllib.loads(Path("Cargo.toml").read_text())["package"]["version"]
    previous = latest_release(repo)
    publish = True
    if previous:
        tag = previous["tag_name"]
        parse_version(tag)
        # Release tags contain only a version-stamping commit on top of main.
        # Its merge base is the exact source revision used for that release.
        old_source = run("git", "merge-base", source, tag)
        diff = subprocess.run(["git", "diff", "--quiet", old_source, source])
        if diff.returncode not in (0, 1):
            raise RuntimeError("Could not compare release source revisions")
        publish = diff.returncode == 1
    version = next_version(baseline, previous["tag_name"] if previous else None)
    values = {"publish": str(publish).lower(), "version": version, "tag": f"v{version}", "source": source}
    with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as output:
        output.write("".join(f"{key}={value}\n" for key, value in values.items()))
    print(f"{'Release' if publish else 'No changes; skip'} v{version} from {source}")


def stamp(version):
    parse_version(version)
    manifest = Path("Cargo.toml")
    text = manifest.read_text()
    package_start = text.index("[package]")
    package_end = text.find("\n[", package_start + len("[package]"))
    if package_end == -1:
        package_end = len(text)
    package, count = re.subn(r'^version = "[^"]+"$', f'version = "{version}"', text[package_start:package_end], count=1, flags=re.M)
    if count != 1:
        raise RuntimeError("Could not locate the package version")
    manifest.write_text(text[:package_start] + package + text[package_end:])
    lock = Path("Cargo.lock")
    content, count = re.subn(r'(\[\[package\]\]\nname = "camera-control"\nversion = ")[^"]+("\n)', lambda m: m[1] + version + m[2], lock.read_text(), count=1)
    if count != 1:
        raise RuntimeError("Could not locate the root package in Cargo.lock")
    lock.write_text(content)


def bundle(version):
    parse_version(version)
    name = f"camera-control-v{version}-linux-x86_64"
    output = Path(".release")
    output.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory() as temporary:
        root = Path(temporary) / name
        root.mkdir()
        for filename in ["README.md", "LICENSE", "NOTICE", "install.py"]:
            shutil.copy2(filename, root / filename)
        shutil.copytree("resources", root / "resources")
        shutil.copy2("target/release/camera-control", root / "camera-control")
        (root / "VERSION").write_text(version + "\n")
        (root / "INSTALL.md").write_text(
            "# Install Camera Control\n\n"
            "Linux x86_64 build, compiled on Ubuntu 24.04 (glibc 2.39).\n"
            "Requires a Wayland/X11 desktop and graphics drivers. A StatusNotifierItem\n"
            "tray host is needed for the tray icon.\n\n"
            "Run directly: `./camera-control`\n\n"
            "Install for your user: `python3 install.py ./camera-control --autostart`\n\n"
            "Preview starts off. Enable watching separately for each camera.\n"
        )
        metadata = json.loads(run("cargo", "metadata", "--locked", "--format-version=1", "--filter-platform", "x86_64-unknown-linux-gnu"))
        packages = []
        notices = root / "THIRD-PARTY-LICENSES"
        for package in metadata["packages"]:
            if package["name"] == "camera-control" and package["source"] is None:
                continue
            package_root = Path(package["manifest_path"]).parent.resolve()
            destination = notices / f'{package["name"]}-{package["version"]}'
            files = set()
            for pattern in ["LICEN[CS]E*", "COPYING*", "NOTICE*", "COPYRIGHT*", "licenses/*", "LICENSES/*"]:
                files.update(package_root.glob(pattern))
            if package.get("license_file"):
                files.add(package_root / package["license_file"])
            for path in sorted(files):
                if path.is_file() and path.resolve().is_relative_to(package_root):
                    relative = path.relative_to(package_root)
                    target = destination / relative
                    target.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copy2(path, target)
            packages.append({key: package[key] for key in ["name", "version", "license", "repository"]})
        notices.mkdir(exist_ok=True)
        (notices / "dependencies.json").write_text(json.dumps(packages, indent=2) + "\n")
        archive = output / f"{name}.tar.gz"
        with tarfile.open(archive, "w:gz") as tar:
            tar.add(root, arcname=name)
    print(archive)


def publish(version, source):
    parse_version(version)
    if not re.fullmatch(r"[a-f0-9]{40}", source):
        raise ValueError("Invalid source revision")
    tag = f"v{version}"
    stamp(version)
    subprocess.run(["git", "config", "user.name", "github-actions[bot]"], check=True)
    subprocess.run(["git", "config", "user.email", "41898282+github-actions[bot]@users.noreply.github.com"], check=True)
    existing = subprocess.run(["git", "rev-parse", "--verify", f"{tag}^{{commit}}"], capture_output=True, text=True)
    if existing.returncode == 0:
        # Resume a failed publishing run without moving an existing tag.
        if run("git", "rev-parse", f"{tag}^") != source:
            raise RuntimeError("Existing tag belongs to a different source revision")
        if run("git", "diff", tag, "--", "Cargo.toml", "Cargo.lock"):
            raise RuntimeError("Existing tag has different version metadata")
    else:
        subprocess.run(["git", "add", "Cargo.toml", "Cargo.lock"], check=True)
        subprocess.run(["git", "-c", "commit.gpgsign=false", "commit", "--allow-empty", "-m", f"Release {tag}"], check=True)
        subprocess.run(["git", "tag", "-a", tag, "-m", f"Camera Control {tag}\nSource: {source}"], check=True)
        subprocess.run(["gh", "auth", "setup-git"], check=True)
        subprocess.run(["git", "push", "origin", f"refs/tags/{tag}"], check=True)
    notes = Path(".release/notes.md")
    notes.write_text(
        "- Linux camera color controls, white-balance presets and optional settings watcher.\n"
        "- Optional live preview with resolution, requested frame rate and mirror controls.\n"
        "- Tray operation and configurable startup at login.\n\n"
        "Download the Linux x86_64 archive, extract it, and follow `INSTALL.md`.\n"
        "The archive includes the installer, Apache-2.0 license and third-party notices.\n\n"
        f"Source revision: https://github.com/{os.environ['GITHUB_REPOSITORY']}/commit/{source}\n"
    )
    assets = sorted(str(path) for path in Path(".release").glob("*.tar.gz*"))
    if len(assets) != 2:
        raise RuntimeError("Expected an archive and its checksum")
    release = subprocess.run(["gh", "release", "view", tag, "--json", "isDraft"], capture_output=True, text=True)
    if release.returncode == 0:
        if not json.loads(release.stdout)["isDraft"]:
            raise RuntimeError("Release is already published; refusing to replace its assets")
        subprocess.run(["gh", "release", "upload", tag, *assets, "--clobber"], check=True)
        subprocess.run(["gh", "release", "edit", tag, "--draft=false", "--latest", "--title", f"Camera Control {tag}", "--notes-file", str(notes)], check=True)
    else:
        subprocess.run(["gh", "release", "create", tag, *assets, "--verify-tag", "--latest", "--title", f"Camera Control {tag}", "--notes-file", str(notes)], check=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["prepare", "stamp", "bundle", "publish"])
    parser.add_argument("version", nargs="?")
    parser.add_argument("source", nargs="?")
    args = parser.parse_args()
    if args.command == "prepare":
        prepare()
    elif args.command == "stamp":
        stamp(args.version)
    elif args.command == "bundle":
        bundle(args.version)
    else:
        publish(args.version, args.source)
