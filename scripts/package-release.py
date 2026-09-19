"""Package a native Mirador AIU executable with documentation and example config."""
import argparse
import os
from pathlib import Path
import tarfile
import tomllib
import zipfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check-tag", action="store_true")
    parser.add_argument("--target")
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    version = tomllib.loads((root / "Cargo.toml").read_text())["package"]["version"]
    if args.check_tag:
        if os.environ.get("RELEASE_REF") != "tag" or os.environ.get("RELEASE_TAG") != f"v{version}":
            raise SystemExit(f"Run from the v{version} tag.")
        return
    supported = {
        "x86_64-pc-windows-msvc", "x86_64-unknown-linux-gnu",
        "aarch64-apple-darwin", "x86_64-apple-darwin",
    }
    if args.target not in supported:
        parser.error("a supported --target is required")
    windows = "windows" in args.target
    executable = "mirador-aiu.exe" if windows else "mirador-aiu"
    files = [(root / "target" / args.target / "release" / executable, executable)]
    for name in ("README.md", "LICENSE", "examples/mirador.toml", "examples/demo.toml"):
        files.append((root / name, name))
    if any(not path.is_file() for path, _ in files):
        raise SystemExit("Missing native executable or release documentation")
    destination = root / "dist"
    destination.mkdir(exist_ok=True)
    suffix = "zip" if windows else "tar.gz"
    archive = destination / f"mirador-aiu-v{version}-{args.target}.{suffix}"
    if windows:
        with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED) as package:
            for path, name in files:
                package.write(path, name)
    else:
        with tarfile.open(archive, "w:gz") as package:
            for path, name in files:
                package.add(path, arcname=name)
    print(archive)


if __name__ == "__main__":
    main()

