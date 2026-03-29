"""Entry point for `remit` command installed via pip."""

from __future__ import annotations

import os
import platform
import shutil
import stat
import subprocess
import sys
import tarfile
import tempfile
import urllib.request
import zipfile
from io import BytesIO
from pathlib import Path

from remit_cli import __version__

TARGETS = {
    ("Linux", "x86_64"): "x86_64-unknown-linux-musl",
    ("Linux", "aarch64"): "aarch64-unknown-linux-musl",
    ("Darwin", "x86_64"): "x86_64-apple-darwin",
    ("Darwin", "arm64"): "aarch64-apple-darwin",
    ("Windows", "AMD64"): "x86_64-pc-windows-msvc",
}


def _bin_dir() -> Path:
    """Directory where the remit binary is cached."""
    return Path.home() / ".remit" / "bin"


def _bin_path() -> Path:
    name = "remit.exe" if sys.platform == "win32" else "remit"
    return _bin_dir() / name


def _get_target() -> str:
    key = (platform.system(), platform.machine())
    target = TARGETS.get(key)
    if not target:
        print(
            f"Unsupported platform: {key[0]}-{key[1]}\n"
            "Download manually from https://github.com/remit-md/remit-cli/releases",
            file=sys.stderr,
        )
        sys.exit(1)
    return target


def _download(url: str) -> bytes:
    """Download with redirect following."""
    req = urllib.request.Request(url, headers={"User-Agent": "remit-cli-pypi"})
    with urllib.request.urlopen(req, timeout=60) as resp:
        return resp.read()


def _ensure_binary() -> Path:
    bin_path = _bin_path()

    # Check if cached binary matches current version
    if bin_path.exists():
        try:
            result = subprocess.run(
                [str(bin_path), "--version"],
                capture_output=True,
                text=True,
                timeout=5,
            )
            if __version__ in result.stdout:
                return bin_path
        except (subprocess.TimeoutExpired, OSError):
            pass  # Binary broken — re-download

    target = _get_target()
    ext = "zip" if sys.platform == "win32" else "tar.gz"
    url = (
        f"https://github.com/remit-md/remit-cli/releases/download/"
        f"v{__version__}/remit-{target}.{ext}"
    )

    print(
        f"Downloading remit v{__version__} for {platform.system()}-{platform.machine()}...",
        file=sys.stderr,
    )
    data = _download(url)

    bin_dir = _bin_dir()
    bin_dir.mkdir(parents=True, exist_ok=True)

    if ext == "tar.gz":
        with tarfile.open(fileobj=BytesIO(data), mode="r:gz") as tar:
            for member in tar.getmembers():
                if Path(member.name).name == "remit":
                    f = tar.extractfile(member)
                    if f is None:
                        raise RuntimeError("Failed to extract remit from tar.gz")
                    bin_path.write_bytes(f.read())
                    break
            else:
                raise RuntimeError("Archive does not contain 'remit' binary")
    else:
        with zipfile.ZipFile(BytesIO(data)) as zf:
            for name in zf.namelist():
                if Path(name).name in ("remit.exe", "remit"):
                    bin_path.write_bytes(zf.read(name))
                    break
            else:
                raise RuntimeError("Archive does not contain 'remit.exe' binary")

    # Set executable on Unix
    if sys.platform != "win32":
        bin_path.chmod(bin_path.stat().st_mode | stat.S_IEXEC | stat.S_IXGRP | stat.S_IXOTH)

    print(f"Installed remit v{__version__} to {bin_path}", file=sys.stderr)
    return bin_path


def main() -> None:
    binary = _ensure_binary()
    try:
        result = subprocess.run([str(binary)] + sys.argv[1:])
        sys.exit(result.returncode)
    except KeyboardInterrupt:
        sys.exit(130)


if __name__ == "__main__":
    main()
