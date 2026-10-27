#!/usr/bin/env python3
"""Install or update this project's launcher, desktop entry, and MCHOSE icon."""
import configparser
import os
from pathlib import Path
import re
import stat
import tempfile

project = Path(__file__).resolve().parent.parent
binary = project / "target/release/mchose-hub-rust"
if not binary.is_file():
    binary = project / "mchose-hub-rust"
icon_source = project / "assets/mchose-icon.png"
if not binary.is_file() or not os.access(binary, os.X_OK):
    raise SystemExit("Build the release first: cargo build --release -j1")
if not icon_source.is_file():
    raise SystemExit(f"Missing application icon: {icon_source}")

launcher = Path.home() / ".local/bin/mchose-native"
alias = Path.home() / ".local/bin/mchose-hub-rust"
data_root = Path(os.environ.get("XDG_DATA_HOME", Path.home() / ".local/share"))
desktop = data_root / "applications/mchose-native.desktop"
icon = data_root / "icons/hicolor/512x512/apps/mchose-native.png"

previous_launchers = {}
for path, allowed in (
    (launcher, {binary, project / "target/release/mchose-native", project / "mchose-native"}),
    (alias, {binary}),
):
    previous = os.readlink(path) if path.is_symlink() else None
    if path.exists() or path.is_symlink():
        target = Path(os.path.abspath(path.parent / previous)) if previous is not None else None
        if target not in allowed:
            raise SystemExit(f"Refusing to replace an unrelated launcher: {path}")
    previous_launchers[path] = previous

for path in (desktop, icon):
    if path.is_symlink() or (path.exists() and not path.is_file()):
        raise SystemExit(f"Refusing to replace an unrelated path: {path}")

previous_desktop = desktop.read_bytes() if desktop.exists() else None
managed_icon = False
if previous_desktop is not None:
    config = configparser.ConfigParser(interpolation=None)
    config.optionxform = str
    try:
        config.read_string(previous_desktop.decode("utf-8"))
        entry = config["Desktop Entry"]
        managed = (
            entry.get("Type") == "Application"
            and entry.get("Name") in {"MCHOSE Native", "MCHOSE Hub Rust"}
            and entry.get("Exec") == f'"{launcher}"'
            and entry.get("StartupWMClass") == "mchose-native"
        )
        managed_icon = (
            managed
            and entry.get("X-MCHOSE-Native-Managed") == "true"
            and entry.get("Icon") == "mchose-native"
        )
    except (UnicodeError, configparser.Error, KeyError):
        managed = False
    if not managed:
        raise SystemExit(f"Refusing to replace an unrelated desktop entry: {desktop}")

icon_bytes = icon_source.read_bytes()
previous_icon = icon.read_bytes() if icon.exists() else None
if previous_icon is not None and previous_icon != icon_bytes and not managed_icon:
    raise SystemExit(f"Refusing to replace an unrelated icon: {icon}")

if previous_desktop is None:
    content = (
        "[Desktop Entry]\n"
        "Type=Application\n"
        "Name=MCHOSE Hub Rust\n"
        f'Exec="{launcher}"\n'
        "Terminal=false\n"
        "Categories=Settings;HardwareSettings;\n"
        "StartupWMClass=mchose-native\n"
        "Keywords=MCHOSE;A7;MagDock;Ace75;keyboard;mouse;клавиатура;мышь;\n"
    )
else:
    content = previous_desktop.decode("utf-8")

section = re.search(r"(?ms)^\[Desktop Entry\]\r?\n.*?(?=^\[|\Z)", content)
if section is None:
    raise SystemExit(f"Invalid desktop entry: {desktop}")
updated = section.group()
for key, value in {
    "Name": "MCHOSE Hub Rust",
    "Icon": "mchose-native",
    "Comment": "Control MCHOSE A7 V3 Ultra+, MagDock and Ace75",
    "Comment[ru]": "Управление A7 V3 Ultra+, MagDock и Ace75",
    "StartupWMClass": "mchose-native",
    "X-MCHOSE-Native-Managed": "true",
}.items():
    pattern = rf"(?m)^[ \t]*{re.escape(key)}[ \t]*=.*$"
    if re.search(pattern, updated):
        updated = re.sub(pattern, lambda _: f"{key}={value}", updated, count=1)
    else:
        updated = updated.rstrip("\n") + f"\n{key}={value}\n"
content = content[:section.start()] + updated + content[section.end():]


def write_checked(path, payload, previous):
    if payload == previous:
        return
    path.parent.mkdir(parents=True, exist_ok=True)
    mode = stat.S_IMODE(path.stat().st_mode) if previous is not None else 0o644
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(dir=path.parent, prefix=f".{path.name}.", delete=False) as file:
            temporary = Path(file.name)
            file.write(payload)
            file.flush()
            os.fsync(file.fileno())
        temporary.chmod(mode)
        current = path.read_bytes() if path.exists() else None
        if path.is_symlink() or current != previous:
            raise SystemExit(f"Path changed during installation; preserved: {path}")
        if previous is None:
            os.link(temporary, path)
        else:
            os.replace(temporary, path)
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)


def update_launcher(path, previous):
    if previous == str(binary):
        return
    path.parent.mkdir(parents=True, exist_ok=True)
    if previous is None:
        path.symlink_to(binary)
        return
    with tempfile.TemporaryDirectory(dir=path.parent, prefix=f".{path.name}.") as directory:
        temporary = Path(directory) / "launcher"
        temporary.symlink_to(binary)
        if not path.is_symlink() or os.readlink(path) != previous:
            raise SystemExit(f"Launcher changed during installation; preserved: {path}")
        os.replace(temporary, path)


write_checked(icon, icon_bytes, previous_icon)
for path, previous in previous_launchers.items():
    update_launcher(path, previous)
write_checked(desktop, content.encode("utf-8"), previous_desktop)
print(f"Command: {alias}\nCompatible command: {launcher}\nMenu: {desktop}\nIcon: {icon}")
