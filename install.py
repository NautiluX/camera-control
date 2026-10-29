#!/usr/bin/env python3
"""Install a locally built executable, launcher and optional login startup entry."""
import argparse
import os
from pathlib import Path
import shutil


def quoted_exec(path):
    value = str(path).replace('\\', '\\\\\\\\').replace('"', '\\\\\\"')
    value = value.replace('$', '\\\\$').replace('`', '\\\\`').replace('%', '%%')
    return '"' + value + '"'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('binary', type=Path, help='Path to the executable produced by cargo build')
    parser.add_argument('--autostart', action='store_true', help='Start in the tray at login')
    args = parser.parse_args()
    if not args.binary.is_file():
        parser.error('The executable does not exist; build it first.')
    data = Path(os.environ.get('XDG_DATA_HOME') or Path.home() / '.local/share')
    config = Path(os.environ.get('XDG_CONFIG_HOME') or Path.home() / '.config')
    install_dir = data / 'camera-control-iced'
    install_dir.mkdir(parents=True, exist_ok=True)
    destination = install_dir / 'camera-control'
    temporary = destination.with_suffix('.new')
    shutil.copy2(args.binary, temporary)
    temporary.chmod(0o755)
    temporary.replace(destination)
    bin_dir = Path.home() / '.local/bin'
    bin_dir.mkdir(parents=True, exist_ok=True)
    shortcut = bin_dir / 'camera-control-iced'
    if shortcut.is_symlink():
        shortcut.unlink()
    if not shortcut.exists():
        shortcut.symlink_to(destination)
    icon_dir = data / 'icons/hicolor/scalable/apps'
    icon_dir.mkdir(parents=True, exist_ok=True)
    shutil.copy2(Path(__file__).parent / 'resources/camera-control.svg', icon_dir / 'camera-control-iced.svg')
    entry = ('[Desktop Entry]\nType=Application\nName=Camera Control\n'
             'Comment=Camera color controls and optional settings watcher\n'
             f'Exec={quoted_exec(destination)}\nIcon=camera-control-iced\nTerminal=false\n'
             'Categories=AudioVideo;Video;\nStartupWMClass=camera-control\n')
    applications = data / 'applications'
    applications.mkdir(parents=True, exist_ok=True)
    (applications / 'camera-control-iced.desktop').write_text(entry)
    startup = config / 'autostart'
    if args.autostart:
        startup.mkdir(parents=True, exist_ok=True)
    if args.autostart:
        entry = entry.replace(f'Exec={quoted_exec(destination)}\n', f'Exec={quoted_exec(destination)} --background\n')
        (startup / 'camera-control-iced.desktop').write_text(entry + 'X-GNOME-Autostart-enabled=true\n')
    print(f'Installed: {destination}')
    print('Open Camera Control from your application menu.')


if __name__ == '__main__':
    main()
