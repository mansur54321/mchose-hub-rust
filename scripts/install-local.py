#!/usr/bin/env python3
"""Install or update this project's launcher, desktop entry, and MCHOSE icon."""
import configparser
import os
from pathlib import Path
import re
import stat
import tempfile

project = Path(__file__).resolve().parent.parent
binary = project / "target/release/mchose-native"
if not binary.is_file():
    binary = project / "mchose-native"
icon_source = project / "assets/mchose-icon.png"
if not binary.is_file() or not os.access(binary, os.X_OK):
    raise SystemExit("Build the release first: cargo build --release -j1")
if not icon_source.is_file():
    raise SystemExit(f"Missing application icon: {icon_source}")

launcher = Path.home() / ".local/bin/mchose-native"
data_root = Path(os.environ.get("XDG_DATA_HOME", Path.home() / ".local/share"))
desktop = data_root / "applications/mchose-native.desktop"
icon = data_root / "icons/hicolor/512x512/apps/mchose-native.png"

if launcher.exists() or launcher.is_symlink():
    if not launcher.is_symlink() or launcher.resolve() != binary.resolve():
        raise SystemExit(f"Refusing to replace an unrelated launcher: {launcher}")

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
            and entry.get("Name") == "MCHOSE Native"
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
        "Name=MCHOSE Native\n"
        f'Exec="{launcher}"\n'
        "Terminal=false\n"
        "Categories=Settings;HardwareSettings;\n"
        "StartupWMClass=mchose-native\n"
        "Keywords=MCHOSE;A7;MagDock;mouse;мышь;\n"
    )
else:
    content = previous_desktop.decode("utf-8")

section = re.search(r"(?ms)^\[Desktop Entry\]\r?\n.*?(?=^\[|\Z)", content)
if section is None:
    raise SystemExit(f"Invalid desktop entry: {desktop}")
updated = section.group()
for key, value in {
    "Icon": "mchose-native",
    "Comment": "Control MCHOSE A7 V3 Ultra+ and MagDock",
    "Comment[ru]": "Управление A7 V3 Ultra+ и MagDock",
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


write_checked(icon, icon_bytes, previous_icon)
if not launcher.is_symlink():
    launcher.parent.mkdir(parents=True, exist_ok=True)
    launcher.symlink_to(binary)
write_checked(desktop, content.encode("utf-8"), previous_desktop)
print(f"Command: {launcher}\nMenu: {desktop}\nIcon: {icon}")
