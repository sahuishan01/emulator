#!/usr/bin/env python3
"""Terminal uploader for the private dump inbox.

Use this for multi-gigabyte dumps instead of the browser page: it keeps running
when windows are minimised, retries forever, and resumes from the bytes the
server already has if it is stopped and started again.

    python3 dump_upload.py PokemonZ-A.nsp

The password is read from DUMP_INBOX_PASSWORD or prompted for; it is never
echoed or written anywhere. Only the Python standard library is needed.
"""

from __future__ import annotations

import argparse
import base64
import getpass
import hashlib
import json
import os
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

DEFAULT_URL = "https://dumps.algosculptor.com"
TIMEOUT = 180
MAX_BACKOFF = 60


class Fatal(Exception):
    pass


class Client:
    def __init__(self, base: str, user: str | None, password: str | None) -> None:
        self.base = base.rstrip("/")
        self.headers = {"X-Dump-Inbox": "1", "User-Agent": "dump-upload/1"}
        if password is not None:
            token = base64.b64encode(f"{user}:{password}".encode()).decode()
            self.headers["Authorization"] = f"Basic {token}"

    def call(self, method: str, path: str, body: bytes | None = None, headers: dict | None = None) -> tuple[int, dict]:
        """One request. Network failures raise OSError; HTTP errors return their status."""
        request = urllib.request.Request(f"{self.base}/{path}", data=body, method=method,
                                         headers={**self.headers, **(headers or {})})
        try:
            with urllib.request.urlopen(request, timeout=TIMEOUT) as response:
                status, raw = response.status, response.read()
        except urllib.error.HTTPError as error:
            status, raw = error.code, error.read()
        try:
            payload = json.loads(raw)
        except ValueError:
            payload = {}
        return status, payload if isinstance(payload, dict) else {}


def human(count: float) -> str:
    for unit in ("B", "KiB", "MiB", "GiB"):
        if count < 1024 or unit == "GiB":
            return f"{count:.0f} {unit}" if unit == "B" else f"{count:.1f} {unit}"
        count /= 1024
    return ""


def retrying(action, what: str):
    """Run `action` until the network lets it through; only Fatal stops it."""
    attempt = 0
    while True:
        try:
            status, payload = action()
            if status == 401:
                raise Fatal("wrong user name or password")
            if status < 500 and status != 429:
                return status, payload
            reason = payload.get("error") or f"HTTP {status}"
        except (OSError, urllib.error.URLError) as error:
            reason = str(getattr(error, "reason", error)) or type(error).__name__
        attempt += 1
        delay = min(MAX_BACKOFF, 2 ** min(attempt, 6))
        print(f"\n  {what}: {reason}; retrying in {delay}s", file=sys.stderr, flush=True)
        time.sleep(delay)


def upload(client: Client, path: Path, quiet: bool = False) -> None:
    size = path.stat().st_size
    begin_body = json.dumps({"name": path.name, "size": size}).encode()
    status, begun = retrying(lambda: client.call("POST", "api/upload/begin", begin_body,
                                                 {"Content-Type": "application/json"}), "begin")
    if status != 200:
        raise Fatal(begun.get("error") or f"begin failed: HTTP {status}")
    upload_id, offset, chunk = begun["id"], begun["received"], begun["chunk"]
    if offset and not quiet:
        print(f"  resuming at {human(offset)}", flush=True)
    started, base = time.monotonic(), offset
    with path.open("rb") as handle:
        while offset < size:
            handle.seek(offset)
            data = handle.read(min(chunk, size - offset))
            if not data:
                raise Fatal("file shrank while uploading")
            headers = {"Content-Type": "application/octet-stream", "X-Chunk-Sha256": hashlib.sha256(data).hexdigest()}
            status, reply = retrying(lambda: client.call("PUT", f"api/upload/{upload_id}?offset={offset}", data, headers),
                                     "chunk")
            if status in (200, 409) and "received" in reply:
                offset = reply["received"]  # 409: the server already has more or less than we thought
            elif status == 422:
                continue  # corrupted in transit; send it again
            else:
                raise Fatal(reply.get("error") or f"upload failed: HTTP {status}")
            if not quiet:
                rate = (offset - base) / max(1e-3, time.monotonic() - started)
                eta = (size - offset) / rate if rate > 0 else 0
                print(f"\r  {human(offset)} of {human(size)} · {100 * offset / size:5.1f}% · {human(rate)}/s · "
                      f"{int(eta // 60)}m{int(eta % 60):02d}s left   ", end="", flush=True)
    status, done = retrying(lambda: client.call("POST", f"api/upload/{upload_id}/finish"), "finish")
    if status != 200:
        raise Fatal(done.get("error") or f"finish failed: HTTP {status}")
    if not quiet:
        print(f"\n  stored as {done['stored']}", flush=True)


def main() -> int:
    parser = argparse.ArgumentParser(description="Resumable terminal uploader for the private dump inbox.")
    parser.add_argument("files", nargs="+", type=Path, help=".nsp/.xci dumps, prod.keys, title.keys or firmware <contentid>.nca files")
    parser.add_argument("--url", default=os.environ.get("DUMP_INBOX_URL", DEFAULT_URL))
    parser.add_argument("--user", default=os.environ.get("DUMP_INBOX_USER", "dump"))
    parser.add_argument("--no-auth", action="store_true", help="skip basic auth (loopback testing only)")
    arguments = parser.parse_args()
    for path in arguments.files:
        if not path.is_file():
            print(f"not a file: {path}", file=sys.stderr)
            return 2
    password = None
    if not arguments.no_auth:
        password = os.environ.get("DUMP_INBOX_PASSWORD") or getpass.getpass(f"Password for {arguments.user}: ")
    client = Client(arguments.url, arguments.user, password)
    try:
        for path in arguments.files:
            print(f"{path.name} ({human(path.stat().st_size)})", flush=True)
            upload(client, path)
    except Fatal as error:
        print(f"\nerror: {error}", file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        print("\nstopped; run the same command again to resume", file=sys.stderr)
        return 130
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
