#!/usr/bin/env python3
"""Reads a macOS crash report of FinchGram: the thread that crashed, frame by frame, with the names
and lines the stripped app does not carry, from the symbols (finchgram.dSYM) every release keeps
since v0.5.2 (docs/crash-reports.md). The app's own panic.log is shown alongside when it has lines
from the minute before the crash.

    scripts/crash-report.py                         the newest report of the app on this Mac
    scripts/crash-report.py REPORT.ips              that one (a report sent by someone else, too)
    scripts/crash-report.py --list                  every report of the app on this Mac
    scripts/crash-report.py --symbols DSYM REPORT   these symbols, for a local build
                                                    (target/release/finchgram.dSYM)

Needs the Xcode command line tools (atos, dwarfdump, c++filt) and, for a release's symbols, gh
logged in. Symbols downloaded stay in ~/Library/Caches/FinchGram/symbols/<version>/.
"""

import argparse
import json
import re
import shutil
import subprocess
import sys
import tempfile
from datetime import datetime, timedelta
from pathlib import Path

REPOSITORY = "FinchGram/FinchGram"
REPORT_FOLDERS = [Path.home() / "Library/Logs/DiagnosticReports", Path.home() / "Library/Logs/DiagnosticReports/Retired"]
PANIC_LOG = Path.home() / "Library/Application Support/FinchGram/panic.log"
SYMBOLS_CACHE = Path.home() / "Library/Caches/FinchGram/symbols"
ROOT = Path(__file__).resolve().parent.parent
FIRST_RELEASE_WITH_SYMBOLS = "0.5.2"


def reports_on_this_mac():
    found = [path for folder in REPORT_FOLDERS if folder.is_dir() for path in folder.glob("finchgram-*.ips")]
    return sorted(found, key=lambda path: path.stat().st_mtime, reverse=True)


def read_report(path):
    """An .ips file is a line of JSON (the summary) and then the report, JSON too."""
    header, body = path.read_text(errors="replace").split("\n", 1)
    return json.loads(header), json.loads(body)


def capture_time(body):
    return datetime.strptime(body["captureTime"], "%Y-%m-%d %H:%M:%S.%f %z")


def what_happened(body):
    exception = body.get("exception", {})
    termination = body.get("termination", {})
    parts = [exception.get("type", ""), exception.get("signal", ""), termination.get("indicator", "")]
    for messages in body.get("asi", {}).values():
        parts.extend(messages)
    return ", ".join(part for part in parts if part)


def app_image(body):
    for index, image in enumerate(body.get("usedImages", [])):
        if image.get("name") == "finchgram" or str(image.get("path", "")).endswith("/MacOS/finchgram"):
            return index, image
    sys.exit("this report has no finchgram image: not a crash of the app?")


def dwarf_file(dsym):
    """The one file inside a dSYM: named after the binary, finchgram-<hash> in a local build."""
    files = [path for path in (dsym / "Contents/Resources/DWARF").glob("*") if path.is_file()] if dsym.is_dir() else []
    return files[0] if len(files) == 1 else None


def uuid_of(dsym):
    dwarf = dwarf_file(dsym)
    if dwarf is None:
        return None
    out = subprocess.run(["dwarfdump", "--uuid", str(dwarf)], capture_output=True, text=True, check=False).stdout
    match = re.search(r"UUID: ([0-9A-Fa-f-]+)", out)
    return match.group(1).lower() if match else None


def download_symbols(version):
    """The release's finchgram-<version>-macos-arm64.dSYM.zip, unpacked into the cache."""
    asset = f"finchgram-{version}-macos-arm64.dSYM.zip"
    target = SYMBOLS_CACHE / version
    with tempfile.TemporaryDirectory() as work:
        got = subprocess.run(
            ["gh", "release", "download", f"v{version}", "--repo", REPOSITORY, "--pattern", asset, "--dir", work],
            capture_output=True, text=True, check=False,
        )
        if got.returncode != 0:
            return None, got.stderr.strip() or f"no {asset} in release v{version}"
        subprocess.run(["ditto", "-x", "-k", str(Path(work) / asset), work], check=True)
        unpacked = next(Path(work).glob("**/finchgram.dSYM"), None)
        if unpacked is None:
            return None, f"{asset} holds no finchgram.dSYM"
        target.mkdir(parents=True, exist_ok=True)
        shutil.rmtree(target / "finchgram.dSYM", ignore_errors=True)
        shutil.move(str(unpacked), str(target / "finchgram.dSYM"))
    return target / "finchgram.dSYM", None


def version_numbers(version):
    return tuple(int(part) if part.isdigit() else 0 for part in version.split("."))


def find_symbols(version, uuid, chosen):
    """The dSYM of exactly this binary (same UUID): the one given, a local build's, the cache's, or
    the release's. Says why when there is none."""
    if chosen is not None:
        found = uuid_of(chosen)
        if found is None:
            sys.exit(f"{chosen} is no dSYM (nothing under Contents/Resources/DWARF/)")
        note = "" if found == uuid else f"WARNING: {chosen} is for another build (UUID {found}, the report's {uuid}): names below may be wrong"
        return chosen, note
    for candidate in [ROOT / "target/release/finchgram.dSYM", SYMBOLS_CACHE / version / "finchgram.dSYM"]:
        if uuid_of(candidate) == uuid:
            return candidate, ""
    if version_numbers(version) < version_numbers(FIRST_RELEASE_WITH_SYMBOLS):
        return None, f"none: the v{version} release keeps no symbols (releases from v{FIRST_RELEASE_WITH_SYMBOLS} on do); finchgram frames are offsets"
    downloaded, problem = download_symbols(version)
    if downloaded is None:
        return None, f"none: {problem}"
    if uuid_of(downloaded) != uuid:
        return None, f"none: the v{version} release's symbols are for another build (UUID {uuid_of(downloaded)}, the report's {uuid}): not the app the release shipped?"
    return downloaded, ""


def symbolicate(dsym, base, offsets):
    """atos names each address, with the file and line the symbols carry; c++filt turns Rust's
    mangled names into paths."""
    addresses = [hex(base + offset) for offset in offsets]
    named = subprocess.run(["atos", "-o", str(dwarf_file(dsym)), "-l", hex(base), *addresses], capture_output=True, text=True, check=True).stdout
    demangled = subprocess.run(["c++filt"], input=named, capture_output=True, text=True, check=False).stdout or named
    lines = demangled.rstrip("\n").split("\n")
    return [re.sub(r" \(in finchgram[^)]*\)", "", line) for line in lines]


def frame_lines(body, app_index, dsym, base):
    thread = next((thread for thread in body.get("threads", []) if thread.get("triggered")), None)
    if thread is None:
        return "no thread is marked as the one that crashed", []
    images = body.get("usedImages", [])
    frames = thread.get("frames", [])
    app_frames = [index for index, frame in enumerate(frames) if frame.get("imageIndex") == app_index]
    names = {}
    if dsym is not None and app_frames:
        for index, name in zip(app_frames, symbolicate(dsym, base, [frames[index]["imageOffset"] for index in app_frames])):
            names[index] = name
    lines = []
    for index, frame in enumerate(frames):
        image = images[frame["imageIndex"]].get("name", "?") if frame.get("imageIndex", -1) < len(images) else "?"
        if index in names:
            where = names[index]
        elif "symbol" in frame:
            where = f"{frame['symbol']} + {frame.get('symbolLocation', 0)}"
        else:
            where = f"+ {frame['imageOffset']}"
        lines.append(f"  {index:3}  {image:<28} {where}")
    title = f"thread {body.get('faultingThread', '?')}" + (f" ({thread['name']})" if thread.get("name") else "") + ", crashed:"
    return title, lines


def panic_log_lines(when):
    """What the app itself wrote (src/main.rs, log_panics) in the two minutes up to the crash."""
    if not PANIC_LOG.is_file():
        return []
    local = when.astimezone().replace(tzinfo=None)
    lines = []
    for line in PANIC_LOG.read_text(errors="replace").splitlines():
        try:
            at = datetime.strptime(line[:19], "%Y-%m-%d %H:%M:%S")
        except ValueError:
            continue
        if local - timedelta(minutes=2) <= at <= local + timedelta(seconds=5):
            lines.append(line)
    return lines


def list_reports():
    found = reports_on_this_mac()
    if not found:
        print("no crash report of finchgram on this Mac")
        return
    for path in found:
        header, body = read_report(path)
        print(f"{capture_time(body):%Y-%m-%d %H:%M:%S}  {header.get('app_version', '?'):<8} {what_happened(body)}")
        print(f"    {path}")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("report", nargs="?", type=Path, help="the .ips file; the newest on this Mac when left out")
    parser.add_argument("--list", action="store_true", help="list the app's reports on this Mac")
    parser.add_argument("--symbols", type=Path, metavar="DSYM", help="a finchgram.dSYM to use instead of the release's")
    args = parser.parse_args()
    if args.list:
        list_reports()
        return
    report = args.report
    if report is None:
        found = reports_on_this_mac()
        if not found:
            sys.exit("no crash report of finchgram on this Mac (~/Library/Logs/DiagnosticReports/)")
        report = found[0]
    header, body = read_report(report)
    version = header.get("app_version") or body.get("bundleInfo", {}).get("CFBundleShortVersionString", "?")
    when = capture_time(body)
    app_index, image = app_image(body)
    base = int(image["base"])
    dsym, note = find_symbols(version, str(image.get("uuid", "")).lower(), args.symbols)

    print(f"finchgram {version} crashed on {when:%Y-%m-%d %H:%M:%S %z}: {what_happened(body)}")
    print(f"report:  {report}")
    print(f"symbols: {dsym if dsym else ''}{(' ' if dsym and note else '') + note}")
    print()
    title, lines = frame_lines(body, app_index, dsym, base)
    print(title)
    print("\n".join(lines))
    print()
    print("panic.log, the two minutes before:")
    panics = panic_log_lines(when)
    print("\n".join(f"  {line}" for line in panics) if panics else f"  nothing ({PANIC_LOG} {'has no line from then' if PANIC_LOG.is_file() else 'does not exist; the app writes it since v0.5.1'})")


if __name__ == "__main__":
    main()
