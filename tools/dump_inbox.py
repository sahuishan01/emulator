#!/usr/bin/env python3
"""Private inbox for the user's own game dumps and console key files.

`serve` runs a loopback-only HTTP service that Caddy publishes behind basic
authentication. Uploads are chunked and resumable because the public route is
proxied by Cloudflare, which caps a single request body at 100 MB.

`status` and `smoke` are the agent-side entry points: list what has arrived and
run the host CLI against it. Nothing in this file ever prints or logs key
material; only names, sizes and SHA-256 digests of whole files are reported.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, unquote, urlparse

DEFAULT_ROOT = Path("/srv/emulator-dumps")
DEFAULT_PORT = 8791
CLIENT_CHUNK = 8 << 20  # small enough that an interrupted chunk costs seconds, not minutes
MAX_CHUNK = 64 << 20
MAX_FILE = 48 << 30
MAX_KEY_FILE = 1 << 20
DISK_RESERVE = 6 << 30
GAME_EXTENSIONS = (".nsp", ".xci")
KEY_NAMES = ("prod.keys", "title.keys")
# Firmware content: one NCA per file, named by content ID as dumped from the
# console's system storage (the shared fonts live in system archives).
FIRMWARE_RE = re.compile(r"^[0-9a-f]{32}(\.cnmt)?\.nca$")
NCA_HEADER_SIZE = 0xC00
MAX_FIRMWARE_FILE = 1 << 30
NAME_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._ ()\[\]+,'-]{0,199}$")
ID_RE = re.compile(r"^[0-9a-f]{32}$")
KEY_LINE_RE = re.compile(r"^\s*[A-Za-z0-9_]+\s*=\s*[0-9A-Fa-f]+\s*$")
CSRF_HEADER = "X-Dump-Inbox"
CLIENT_SCRIPT = Path(__file__).resolve().with_name("dump_upload.py")


class InboxError(Exception):
    def __init__(self, status: int, message: str, **extra: object) -> None:
        super().__init__(message)
        self.status = status
        self.message = message
        self.extra = extra


def classify(name: str) -> tuple[str, str]:
    """Return (kind, stored_name) or raise for anything that is not a dump."""
    if not NAME_RE.fullmatch(name) or name.endswith(".") or ".." in name:
        raise InboxError(400, "file name contains unsupported characters")
    lowered = name.lower()
    if lowered in KEY_NAMES:
        return "keys", lowered
    if lowered.endswith(GAME_EXTENSIONS):
        return "games", name
    if FIRMWARE_RE.fullmatch(lowered):
        return "firmware", lowered
    raise InboxError(400, "only .nsp, .xci, prod.keys, title.keys and firmware <contentid>.nca are accepted")


def validate_content(kind: str, name: str, path: Path) -> None:
    if kind == "keys":
        try:
            text = path.read_bytes().decode("utf-8")
        except UnicodeDecodeError:
            raise InboxError(422, "key file is not text") from None
        entries = 0
        for line in text.splitlines():
            stripped = line.strip()
            if not stripped or stripped.startswith((";", "#")):
                continue
            if not KEY_LINE_RE.fullmatch(line):
                raise InboxError(422, "key file has a line that is not `name = hex`")
            entries += 1
        if entries == 0:
            raise InboxError(422, "key file has no entries")
        return
    if kind == "firmware":
        # NCA headers are encrypted with the header key, so only the shape is
        # checkable here; the emulator validates contents with the user's keys.
        if path.stat().st_size < NCA_HEADER_SIZE:
            raise InboxError(422, "not an NCA: smaller than its 0xC00-byte header")
        return
    with path.open("rb") as handle:
        if name.lower().endswith(".nsp"):
            if handle.read(4) != b"PFS0":
                raise InboxError(422, "not an NSP: missing PFS0 magic")
        else:
            handle.seek(0x100)
            if handle.read(4) != b"HEAD":
                raise InboxError(422, "not an XCI: missing HEAD magic")


def cpu_idle_percent(interval: float = 1.0) -> float:
    def sample() -> tuple[int, int]:
        fields = [int(value) for value in Path("/proc/stat").read_text().splitlines()[0].split()[1:]]
        return fields[3] + fields[4], sum(fields)

    idle_a, total_a = sample()
    time.sleep(interval)
    idle_b, total_b = sample()
    total = total_b - total_a
    return 100.0 if total <= 0 else 100.0 * (idle_b - idle_a) / total


def wait_for_idle_cpu(max_wait: float = 600.0) -> None:
    """Heavy background work waits for >= 80% idle CPU, bounded so it cannot starve."""
    deadline = time.monotonic() + max_wait
    while time.monotonic() < deadline:
        try:
            if cpu_idle_percent() >= 80.0:
                return
        except (OSError, ValueError, IndexError):
            return
        time.sleep(5)


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        while block := handle.read(4 << 20):
            digest.update(block)
    return digest.hexdigest()


class Inbox:
    def __init__(self, root: Path, gate_cpu: bool = True) -> None:
        self.root = root
        self.gate_cpu = gate_cpu
        self.incoming = root / "incoming"
        self.dirs = {"games": root / "games", "keys": root / "keys", "firmware": root / "firmware"}
        for directory in (root, self.incoming, *self.dirs.values()):
            directory.mkdir(mode=0o700, parents=True, exist_ok=True)
        self.hash_path = root / "hashes.json"
        self._meta_lock = threading.Lock()
        self._upload_locks: dict[str, threading.Lock] = {}
        self._hash_wake = threading.Event()
        self._hash_idle = threading.Event()

    # ---- uploads -------------------------------------------------------

    def _lock_for(self, upload_id: str) -> threading.Lock:
        with self._meta_lock:
            return self._upload_locks.setdefault(upload_id, threading.Lock())

    def _paths(self, upload_id: str) -> tuple[Path, Path]:
        if not ID_RE.fullmatch(upload_id):
            raise InboxError(400, "malformed upload id")
        return self.incoming / f"{upload_id}.part", self.incoming / f"{upload_id}.json"

    def _meta(self, upload_id: str) -> dict:
        _, meta_path = self._paths(upload_id)
        try:
            return json.loads(meta_path.read_text())
        except (OSError, ValueError):
            raise InboxError(404, "unknown upload; call begin again") from None

    def begin(self, name: object, size: object) -> dict:
        if not isinstance(name, str) or not isinstance(size, int) or isinstance(size, bool):
            raise InboxError(400, "begin needs a string name and an integer size")
        kind, stored = classify(name)
        limit = {"keys": MAX_KEY_FILE, "firmware": MAX_FIRMWARE_FILE}.get(kind, MAX_FILE)
        if size <= 0 or size > limit:
            raise InboxError(413, f"size must be between 1 and {limit} bytes for this file type")
        upload_id = hashlib.sha256(f"{stored}\0{size}".encode()).hexdigest()[:32]
        part, meta_path = self._paths(upload_id)
        with self._lock_for(upload_id):
            if not (part.exists() and meta_path.exists()):
                free = shutil.disk_usage(self.root).free
                if size + DISK_RESERVE > free:
                    raise InboxError(507, "not enough free disk space on the server")
                part.write_bytes(b"")
                part.chmod(0o600)
                meta_path.write_text(json.dumps({"name": stored, "kind": kind, "size": size, "started": int(time.time())}))
                meta_path.chmod(0o600)
            received = part.stat().st_size
        return {"id": upload_id, "name": stored, "kind": kind, "size": size, "received": received, "chunk": CLIENT_CHUNK}

    def append(self, upload_id: str, offset: int, data: bytes, expected_sha256: str | None) -> dict:
        part, _ = self._paths(upload_id)
        with self._lock_for(upload_id):
            meta = self._meta(upload_id)
            received = part.stat().st_size
            if offset != received:
                raise InboxError(409, "offset does not match bytes already stored", received=received)
            if not data or received + len(data) > meta["size"]:
                raise InboxError(400, "chunk is empty or runs past the declared size", received=received)
            if expected_sha256 is not None and hashlib.sha256(data).hexdigest() != expected_sha256.lower():
                raise InboxError(422, "chunk SHA-256 mismatch; resend it", received=received)
            with part.open("ab") as handle:
                handle.write(data)
                handle.flush()
                os.fsync(handle.fileno())
            return {"received": received + len(data), "size": meta["size"]}

    def finish(self, upload_id: str) -> dict:
        part, meta_path = self._paths(upload_id)
        with self._lock_for(upload_id):
            meta = self._meta(upload_id)
            received = part.stat().st_size
            if received != meta["size"]:
                raise InboxError(409, "upload is incomplete", received=received)
            try:
                validate_content(meta["kind"], meta["name"], part)
            except InboxError:
                part.unlink(missing_ok=True)
                meta_path.unlink(missing_ok=True)
                raise
            destination = self.dirs[meta["kind"]] / meta["name"]
            os.replace(part, destination)
            meta_path.unlink(missing_ok=True)
        log(f"stored {meta['kind']}/{meta['name']} ({meta['size']} bytes)")
        self._hash_wake.set()
        return {"stored": f"{meta['kind']}/{meta['name']}", "size": meta["size"]}

    def abort(self, upload_id: str) -> dict:
        part, meta_path = self._paths(upload_id)
        with self._lock_for(upload_id):
            self._meta(upload_id)
            part.unlink(missing_ok=True)
            meta_path.unlink(missing_ok=True)
        return {"aborted": upload_id}

    def delete(self, kind: str, name: str) -> dict:
        if kind not in self.dirs:
            raise InboxError(404, "unknown file kind")
        _, stored = classify(name)
        target = self.dirs[kind] / stored
        if not target.is_file():
            raise InboxError(404, "no such file")
        target.unlink()
        log(f"deleted {kind}/{stored}")
        self._hash_wake.set()
        return {"deleted": f"{kind}/{stored}"}

    # ---- listing and hashing ------------------------------------------

    def _load_hashes(self) -> dict:
        try:
            return json.loads(self.hash_path.read_text())
        except (OSError, ValueError):
            return {}

    def listing(self) -> dict:
        hashes = self._load_hashes()
        files = []
        for kind, directory in self.dirs.items():
            for path in sorted(directory.iterdir()):
                if not path.is_file():
                    continue
                stat = path.stat()
                known = hashes.get(f"{kind}/{path.name}", {})
                current = known.get("size") == stat.st_size and known.get("mtime_ns") == stat.st_mtime_ns
                files.append({
                    "kind": kind,
                    "name": path.name,
                    "size": stat.st_size,
                    "modified": int(stat.st_mtime),
                    "sha256": known.get("sha256") if current else None,
                })
        partial = []
        for meta_path in sorted(self.incoming.glob("*.json")):
            try:
                meta = json.loads(meta_path.read_text())
                received = meta_path.with_suffix(".part").stat().st_size
            except (OSError, ValueError):
                continue
            partial.append({"id": meta_path.stem, "name": meta["name"], "size": meta["size"], "received": received})
        return {"files": files, "partial": partial, "free": shutil.disk_usage(self.root).free}

    def refresh_hashes(self) -> None:
        """Hash anything new, including files dropped directly with scp/rsync."""
        hashes = self._load_hashes()
        fresh = {}
        for kind, directory in self.dirs.items():
            for path in sorted(directory.iterdir()):
                if not path.is_file():
                    continue
                key = f"{kind}/{path.name}"
                stat = path.stat()
                known = hashes.get(key, {})
                if known.get("size") == stat.st_size and known.get("mtime_ns") == stat.st_mtime_ns:
                    fresh[key] = known
                    continue
                if self.gate_cpu and stat.st_size > (64 << 20):
                    wait_for_idle_cpu()
                try:
                    digest = sha256_file(path)
                    after = path.stat()
                except OSError:
                    continue
                if (after.st_size, after.st_mtime_ns) != (stat.st_size, stat.st_mtime_ns):
                    continue  # still being written; next pass picks it up
                fresh[key] = {"size": stat.st_size, "mtime_ns": stat.st_mtime_ns, "sha256": digest}
                self._write_hashes({**hashes, **fresh})
        if fresh != hashes:
            self._write_hashes(fresh)

    def _write_hashes(self, hashes: dict) -> None:
        temporary = self.hash_path.with_suffix(".tmp")
        temporary.write_text(json.dumps(hashes, indent=1, sort_keys=True))
        temporary.chmod(0o600)
        os.replace(temporary, self.hash_path)

    def hash_worker(self) -> None:
        while True:
            self._hash_wake.clear()
            self._hash_idle.clear()
            try:
                self.refresh_hashes()
            except Exception as error:  # keep the worker alive; report the class only
                log(f"hash pass failed: {type(error).__name__}")
            if not self._hash_wake.is_set():
                self._hash_idle.set()
            self._hash_wake.wait(timeout=60)

    def wait_hashed(self, timeout: float = 10.0) -> bool:
        return self._hash_idle.wait(timeout)


def log(message: str) -> None:
    print(f"dump-inbox: {message}", file=sys.stderr, flush=True)


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    server_version = "dump-inbox"
    sys_version = ""
    timeout = 300  # a stalled or vanished client must not pin a thread forever
    inbox: Inbox

    def log_message(self, *_: object) -> None:  # request lines stay out of the journal
        return

    def _send(self, status: int, body: bytes, content_type: str) -> None:
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        self.send_header("X-Content-Type-Options", "nosniff")
        if self.close_connection:
            self.send_header("Connection", "close")
        self.end_headers()
        self.wfile.write(body)

    def _json(self, status: int, payload: dict) -> None:
        self._send(status, json.dumps(payload).encode(), "application/json")

    def _body(self) -> bytes:
        try:
            length = int(self.headers.get("Content-Length", ""))
        except ValueError:
            self.close_connection = True
            raise InboxError(411, "Content-Length is required") from None
        if length < 0 or length > MAX_CHUNK:
            self.close_connection = True
            raise InboxError(413, f"request body may not exceed {MAX_CHUNK} bytes")
        try:
            data = self.rfile.read(length)
        except OSError:
            data = b""
        if len(data) != length:
            self.close_connection = True
            raise InboxError(400, "request body was truncated")
        return data

    def _dispatch(self, method: str) -> None:
        try:
            body = self._body() if method in ("POST", "PUT") else b""
            if method != "GET" and self.headers.get(CSRF_HEADER) != "1":
                raise InboxError(403, f"missing {CSRF_HEADER} header")
            url = urlparse(self.path)
            parts = [unquote(part) for part in url.path.split("/") if part]
            result = self._route(method, parts, parse_qs(url.query), body)
            if result is not None:
                self._json(200, result)
        except InboxError as error:
            self._reply_error(error.status, {"error": error.message, **error.extra})
        except OSError:  # client went away mid-request (tab frozen, network drop, timeout)
            self.close_connection = True
        except Exception as error:
            log(f"internal error: {type(error).__name__}")
            self.close_connection = True
            self._reply_error(500, {"error": "internal error"})

    def _reply_error(self, status: int, payload: dict) -> None:
        try:
            self._json(status, payload)
        except OSError:
            self.close_connection = True

    def _route(self, method: str, parts: list[str], query: dict, body: bytes) -> dict | None:
        inbox = self.inbox
        if method == "GET" and not parts:
            self._send(200, PAGE.encode(), "text/html; charset=utf-8")
            return None
        if method == "GET" and parts == ["dump_upload.py"]:
            try:
                script = CLIENT_SCRIPT.read_bytes()
            except OSError:
                raise InboxError(404, "terminal uploader is not installed") from None
            self._send(200, script, "text/x-python; charset=utf-8")
            return None
        if method == "GET" and parts == ["healthz"]:
            return {"ok": True}
        if method == "GET" and parts == ["api", "files"]:
            return inbox.listing()
        if method == "POST" and parts == ["api", "upload", "begin"]:
            try:
                request = json.loads(body)
            except ValueError:
                raise InboxError(400, "begin body must be JSON") from None
            if not isinstance(request, dict):
                raise InboxError(400, "begin body must be a JSON object")
            return inbox.begin(request.get("name"), request.get("size"))
        if len(parts) == 3 and parts[:2] == ["api", "upload"]:
            if method == "PUT":
                try:
                    offset = int(query.get("offset", [""])[0])
                except ValueError:
                    raise InboxError(400, "offset query parameter is required") from None
                return inbox.append(parts[2], offset, body, self.headers.get("X-Chunk-Sha256"))
            if method == "DELETE":
                return inbox.abort(parts[2])
        if method == "POST" and len(parts) == 4 and parts[:2] == ["api", "upload"] and parts[3] == "finish":
            return inbox.finish(parts[2])
        if method == "DELETE" and len(parts) == 4 and parts[:2] == ["api", "files"]:
            return inbox.delete(parts[2], parts[3])
        raise InboxError(404, "no such endpoint")

    def do_GET(self) -> None:
        self._dispatch("GET")

    def do_POST(self) -> None:
        self._dispatch("POST")

    def do_PUT(self) -> None:
        self._dispatch("PUT")

    def do_DELETE(self) -> None:
        self._dispatch("DELETE")


def make_server(root: Path, host: str, port: int, gate_cpu: bool = True) -> tuple[ThreadingHTTPServer, Inbox]:
    inbox = Inbox(root, gate_cpu=gate_cpu)
    handler = type("BoundHandler", (Handler,), {"inbox": inbox})
    server = ThreadingHTTPServer((host, port), handler)
    server.daemon_threads = True
    threading.Thread(target=inbox.hash_worker, daemon=True).start()
    return server, inbox


PAGE = r"""<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>Emulator dump inbox</title>
<style>
:root{--bg:#0f1114;--panel:#171a1f;--line:#2a2f37;--ink:#e8eaed;--dim:#8b93a1;--accent:#7cc4ff;--ok:#7ddc9a;--bad:#ff8a8a;--label:"Space Mono",ui-monospace,Menlo,Consolas,monospace}
*{box-sizing:border-box}body{margin:0;background:var(--bg);color:var(--ink);font:15px/1.5 system-ui,sans-serif}
main{max-width:760px;margin:0 auto;padding:24px 16px 64px}
h1{font:700 18px var(--label);letter-spacing:.04em;margin:0 0 4px}
h2{font:700 12px var(--label);letter-spacing:.12em;text-transform:uppercase;color:var(--dim);margin:28px 0 8px}
p{color:var(--dim);margin:0 0 12px}
.panel{background:var(--panel);border:1px solid var(--line);border-radius:6px;padding:14px}
.row{padding:10px 0;border-top:1px solid var(--line)}.row:first-child{border-top:0;padding-top:0}.row:last-child{padding-bottom:0}
.name{word-break:break-all}.meta{font:12px var(--label);color:var(--dim);word-break:break-all}
a{color:var(--accent)}code{font:13px var(--label);color:var(--ink)}
.ok{color:var(--ok)}.bad{color:var(--bad)}
progress{width:100%;height:8px;accent-color:var(--accent)}
button,label.pick{font:700 12px var(--label);letter-spacing:.08em;text-transform:uppercase;color:var(--bg);background:var(--accent);border:0;border-radius:4px;padding:10px 14px;cursor:pointer;display:inline-block}
button.ghost{background:transparent;color:var(--dim);border:1px solid var(--line);padding:4px 8px;float:right}
input[type=file]{display:none}
</style></head><body><main>
<h1>EMULATOR DUMP INBOX</h1>
<p>Private drop for your own dumps. Accepted: <b>.nsp</b>, <b>.xci</b>, <b>prod.keys</b>, <b>title.keys</b>, and firmware NCAs named <b>&lt;contentid&gt;.nca</b> (select many at once). Uploads retry on their own and resume from where they stopped; after a reload, pick the same file again.</p>
<p>Browsers pause pages that are minimised or in the background, so keep this tab visible for big files, or use the terminal uploader, which is not affected: download <a href="dump_upload.py" download>dump_upload.py</a> and run <code>python3 dump_upload.py YourGame.nsp</code>.</p>
<label class="pick">Choose files<input id="pick" type="file" multiple></label>
<h2>Transfers</h2><div class="panel" id="jobs"><div class="meta">Nothing in progress.</div></div>
<h2>On server</h2><div class="panel" id="files"><div class="meta">Loading…</div></div>
<script>
const H={"X-Dump-Inbox":"1"},$=id=>document.getElementById(id);
const fmt=n=>{const u=["B","KiB","MiB","GiB"];let i=0;while(n>=1024&&i<3){n/=1024;i++}return n.toFixed(i?1:0)+" "+u[i]};
let wake=()=>{},active=0,lock=null;
const sleep=ms=>new Promise(r=>{const t=setTimeout(r,ms);wake=()=>{clearTimeout(t);r()}});
async function keepAwake(){if(active&&document.visibilityState==="visible"&&!(lock&&!lock.released)){try{lock=await navigator.wakeLock.request("screen")}catch(_){}}}
document.addEventListener("visibilitychange",()=>{if(document.visibilityState==="visible"){keepAwake();wake()}});
addEventListener("online",()=>wake());
addEventListener("beforeunload",e=>{if(active){e.preventDefault();e.returnValue=""}});
const hex=b=>[...new Uint8Array(b)].map(x=>x.toString(16).padStart(2,"0")).join("");
const el=(tag,cls,text)=>{const e=document.createElement(tag);if(cls)e.className=cls;if(text!=null)e.textContent=text;return e};
async function api(method,path,body,headers){
  const c=new AbortController(),t=setTimeout(()=>c.abort(),180000);
  try{
    const r=await fetch(path,{method,body,headers:{...H,...headers},signal:c.signal});
    let j={};try{j=await r.json()}catch(e){}
    return {status:r.status,ok:r.ok,j};
  }finally{clearTimeout(t)}
}
async function patient(meta,call){
  for(let tries=1;;tries++){
    let why;
    try{const r=await call();if(r.status<500&&r.status!==429&&r.status!==422)return r;why=r.j.error||("HTTP "+r.status)}
    catch(e){why=e.name==="AbortError"?"timed out":(navigator.onLine===false?"offline":e.message)}
    const wait=Math.min(30,2**Math.min(tries,5));
    meta.textContent="paused ("+why+") — retrying in "+wait+" s, attempt "+tries;meta.className="meta bad";
    await sleep(wait*1000);meta.className="meta";
  }
}
async function refresh(){
  const box=$("files");
  try{
    const {j}=await api("GET","api/files");box.replaceChildren();
    for(const f of j.files){
      const row=el("div","row"),del=el("button","ghost","delete");
      del.onclick=async()=>{if(confirm("Delete "+f.name+" from the server?")){await api("DELETE","api/files/"+f.kind+"/"+encodeURIComponent(f.name));refresh()}};
      row.append(del,el("div","name",f.kind+"/"+f.name),el("div","meta",fmt(f.size)+" · sha256 "+(f.sha256||"computing…")));box.append(row);
    }
    for(const p of j.partial){
      const row=el("div","row"),del=el("button","ghost","discard");
      del.onclick=async()=>{await api("DELETE","api/upload/"+p.id);refresh()};
      row.append(del,el("div","name",p.name+" (partial)"),el("div","meta",fmt(p.received)+" of "+fmt(p.size)+" — choose the same file to resume"));box.append(row);
    }
    if(!box.children.length)box.append(el("div","meta","No files yet."));
    box.append(el("div","row meta","Free space: "+fmt(j.free)));
  }catch(e){box.replaceChildren(el("div","meta bad","Could not load the file list."))}
}
async function upload(file){
  const row=el("div","row"),name=el("div","name",file.name),bar=el("progress"),meta=el("div","meta","starting…");
  bar.max=file.size;bar.value=0;row.append(name,bar,meta);
  const jobs=$("jobs");if(!jobs.querySelector("progress"))jobs.replaceChildren();jobs.append(row);
  const fail=m=>{meta.textContent=m;meta.className="meta bad"};
  const b=await patient(meta,()=>api("POST","api/upload/begin",JSON.stringify({name:file.name,size:file.size}),{"Content-Type":"application/json"}));
  if(!b.ok)return fail(b.j.error||("begin failed: HTTP "+b.status));
  let off=b.j.received,started=Date.now(),base=off;bar.value=off;
  while(off<file.size){
    const at=off,r=await patient(meta,async()=>{
      const buf=await file.slice(at,Math.min(at+b.j.chunk,file.size)).arrayBuffer();
      const sum=hex(await crypto.subtle.digest("SHA-256",buf));
      return api("PUT","api/upload/"+b.j.id+"?offset="+at,buf,{"X-Chunk-Sha256":sum,"Content-Type":"application/octet-stream"});
    });
    if((r.ok||r.status===409)&&Number.isFinite(r.j.received))off=r.j.received;
    else return fail(r.j.error||("upload failed: HTTP "+r.status));
    bar.value=off;const rate=(off-base)/Math.max(1,(Date.now()-started)/1000);
    meta.textContent=fmt(off)+" of "+fmt(file.size)+" · "+fmt(rate)+"/s";
  }
  const f=await patient(meta,()=>api("POST","api/upload/"+b.j.id+"/finish"));
  if(!f.ok)return fail(f.j.error||("finish failed: HTTP "+f.status));
  bar.value=file.size;meta.textContent="stored as "+f.j.stored;meta.className="meta ok";refresh();
}
const queue=[];let running=false;
async function drain(){
  if(running)return;running=true;
  while(queue.length){
    active++;keepAwake();
    try{await upload(queue.shift())}catch(e){}
    active--;
  }
  running=false;if(lock){try{lock.release()}catch(_){}lock=null}
}
$("pick").onchange=e=>{
  queue.push(...e.target.files);e.target.value="";
  const hold=()=>drain();
  if(navigator.locks)navigator.locks.request("dump-inbox-upload",hold);else hold();
};
refresh();setInterval(refresh,15000);
</script></main></body></html>
"""


# ---- agent-side commands ---------------------------------------------------


def command_serve(arguments: argparse.Namespace) -> int:
    os.umask(0o077)
    server, _ = make_server(arguments.root, arguments.host, arguments.port)
    log(f"listening on {arguments.host}:{arguments.port}, root {arguments.root}")
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    return 0


def read_listing(root: Path) -> dict:
    hashes = {}
    try:
        hashes = json.loads((root / "hashes.json").read_text())
    except (OSError, ValueError):
        pass
    files = []
    for kind in ("games", "keys", "firmware"):
        directory = root / kind
        if not directory.is_dir():
            continue
        for path in sorted(directory.iterdir()):
            if path.is_file():
                stat = path.stat()
                known = hashes.get(f"{kind}/{path.name}", {})
                current = known.get("size") == stat.st_size and known.get("mtime_ns") == stat.st_mtime_ns
                files.append({"kind": kind, "path": str(path), "size": stat.st_size, "modified": int(stat.st_mtime),
                              "sha256": known.get("sha256") if current else None})
    partial = []
    for meta_path in sorted((root / "incoming").glob("*.json")) if (root / "incoming").is_dir() else []:
        try:
            meta = json.loads(meta_path.read_text())
            partial.append({"name": meta["name"], "size": meta["size"], "received": meta_path.with_suffix(".part").stat().st_size})
        except (OSError, ValueError, KeyError):
            continue
    return {"files": files, "partial": partial}


def command_status(arguments: argparse.Namespace) -> int:
    listing = read_listing(arguments.root)
    if arguments.json:
        print(json.dumps(listing, indent=1))
        return 0
    if not listing["files"] and not listing["partial"]:
        print(f"no dumps in {arguments.root}")
    for entry in listing["files"]:
        stamp = time.strftime("%Y-%m-%d %H:%M", time.localtime(entry["modified"]))
        print(f"{entry['kind']:5} {entry['size']:>12}  {stamp}  {entry['sha256'] or 'sha256 pending'}  {entry['path']}")
    for entry in listing["partial"]:
        print(f"partial {entry['received']}/{entry['size']}  {entry['name']}")
    return 0


SANDBOX_DIR = "/run/emulator-smoke"
SANDBOX_PROPERTIES = (
    # Untrusted guest code and container parsing run here: no network, no writable
    # disk, no privileges, no view of the host beyond the three bound inputs.
    "NoNewPrivileges=true",
    "PrivateNetwork=true",
    "RestrictAddressFamilies=~AF_UNIX AF_INET AF_INET6 AF_NETLINK AF_PACKET",
    "IPAddressDeny=any",
    "ProtectSystem=strict",
    "ProtectHome=true",
    "PrivateTmp=true",
    "PrivateDevices=true",
    "PrivateIPC=true",
    "ProtectProc=invisible",
    "ProcSubset=pid",
    "ProtectKernelTunables=true",
    "ProtectKernelModules=true",
    "ProtectKernelLogs=true",
    "ProtectControlGroups=true",
    "ProtectClock=true",
    "ProtectHostname=true",
    "RestrictNamespaces=true",
    "RestrictSUIDSGID=true",
    "RestrictRealtime=true",
    "LockPersonality=true",
    "MemoryDenyWriteExecute=true",
    "SystemCallArchitectures=native",
    "SystemCallFilter=@system-service",
    "SystemCallFilter=~@privileged @resources @network-io",  # no sockets at all, not even to host unix sockets
    "SystemCallErrorNumber=EPERM",
    "CapabilityBoundingSet=",
    "AmbientCapabilities=",
    "UMask=0077",
    "LimitCORE=0",  # a core dump would write decrypted game data and keys to disk
    "MemorySwapMax=0",
    "TasksMax=64",
    "Nice=10",
)


def sandbox_command(binary: Path, game: Path, prod_keys: Path, title_keys: Path | None, max_steps: int,
                    root: Path, memory_max: str, timeout: int, disasm: list[str] | None = None,
                    peek: list[str] | None = None, watch: str | None = None,
                    trace: list[str] | None = None) -> list[str]:
    """Wrap `emulator-cli run-nsp` in a transient, locked-down systemd service.

    The process runs as the owner of the dump (not root) and sees only the binary,
    the game and the key files, each bound read-only under SANDBOX_DIR; the rest of
    the inbox is hidden.
    """
    owner = game.stat()
    binds = [(binary, "emulator-cli"), (game, "game" + game.suffix.lower()), (prod_keys, "prod.keys")]
    inner = [f"{SANDBOX_DIR}/emulator-cli", "run-nsp", f"{SANDBOX_DIR}/game{game.suffix.lower()}",
             "--keys", f"{SANDBOX_DIR}/prod.keys", "--max-steps", str(max_steps)]
    if title_keys is not None:
        binds.append((title_keys, "title.keys"))
        inner += ["--titlekeys", f"{SANDBOX_DIR}/title.keys"]
    for flag, specs in (("--disasm", disasm), ("--peek", peek)):
        for spec in specs or []:
            if not re.fullmatch(r"(0x)?[0-9a-fA-F]{1,16}:[0-9]{1,4}", spec):
                raise ValueError(f"{flag} expects HEXADDR:COUNT, got {spec!r}")
            inner += [flag, spec]
    if watch is not None:
        if not re.fullmatch(r"(0x)?[0-9a-fA-F]{1,16}:[0-9]{1,4}", watch):
            raise ValueError(f"--watch expects HEXADDR:LEN, got {watch!r}")
        inner += ["--watch", watch]
    for spec in trace or []:
        if not re.fullmatch(r"(0x)?[0-9a-fA-F]{1,16}", spec):
            raise ValueError(f"--trace expects HEXPC, got {spec!r}")
        inner += ["--trace", spec]
    command = ["systemd-run", "--wait", "--pipe", "--collect", "--quiet",
               "--description=emulator dump smoke test (sandboxed)",
               "-p", f"User={owner.st_uid}", "-p", f"Group={owner.st_gid}",
               "-p", f"MemoryMax={memory_max}", "-p", f"RuntimeMaxSec={timeout}",
               "-p", f"InaccessiblePaths={root}"]
    for prop in SANDBOX_PROPERTIES:
        command += ["-p", prop]
    for source, name in binds:
        if any(character in str(source) for character in ": \t\n\\"):
            raise ValueError(f"path cannot be bound into the sandbox: {source.name}")
        command += ["-p", f"BindReadOnlyPaths={source}:{SANDBOX_DIR}/{name}"]
    return command + ["--", *inner]


def command_smoke(arguments: argparse.Namespace) -> int:
    root: Path = arguments.root.resolve()
    prod_keys = root / "keys" / "prod.keys"
    title_keys = root / "keys" / "title.keys"
    if arguments.game:
        game = Path(arguments.game).resolve()
    else:
        candidates = sorted((root / "games").glob("*.nsp"), key=lambda path: path.stat().st_mtime)
        if not candidates:
            print(f"no .nsp in {root / 'games'}; upload one first", file=sys.stderr)
            return 2
        game = candidates[-1]
    if not prod_keys.is_file():
        print(f"missing {prod_keys}; upload prod.keys first", file=sys.stderr)
        return 2
    repository = Path(__file__).resolve().parent.parent
    # Building our own source needs the toolchain and stays outside the sandbox;
    # nothing from the dump is involved until the binary runs.
    build = subprocess.run(["cargo", "build", "--release", "--quiet", "-p", "emulator-cli"], cwd=repository, check=False)
    if build.returncode != 0:
        return build.returncode
    binary = repository / "target" / "release" / "emulator-cli"
    if not binary.is_file():
        print(f"build did not produce {binary}", file=sys.stderr)
        return 2
    if shutil.which("systemd-run") is None or os.geteuid() != 0:
        # Fail closed: an unsandboxed run of untrusted guest code is never a silent fallback.
        print("smoke needs root and systemd-run for the sandbox; refusing to run unsandboxed", file=sys.stderr)
        return 2
    command = sandbox_command(binary, game, prod_keys, title_keys if title_keys.is_file() else None,
                              arguments.max_steps, root, arguments.memory_max, arguments.timeout,
                              arguments.disasm, arguments.peek, arguments.watch,
                              arguments.trace)
    print(f"smoke (sandboxed): {game.name} ({game.stat().st_size} bytes), max {arguments.max_steps} steps", flush=True)
    return subprocess.run(command, cwd="/", check=False).returncode


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--root", type=Path, default=DEFAULT_ROOT)
    commands = parser.add_subparsers(dest="command", required=True)
    serve = commands.add_parser("serve", help="run the loopback upload service")
    serve.add_argument("--host", default="127.0.0.1")
    serve.add_argument("--port", type=int, default=DEFAULT_PORT)
    serve.set_defaults(run=command_serve)
    status = commands.add_parser("status", help="list uploaded dumps and partial uploads")
    status.add_argument("--json", action="store_true")
    status.set_defaults(run=command_status)
    smoke = commands.add_parser("smoke", help="run the host CLI against an uploaded NSP")
    smoke.add_argument("game", nargs="?", help="NSP path; defaults to the newest upload")
    smoke.add_argument("--max-steps", type=int, default=10_000_000)
    smoke.add_argument("--memory-max", default="11G", help="cgroup memory cap for the sandboxed run")
    smoke.add_argument("--timeout", type=int, default=1800, help="wall-clock limit in seconds")
    smoke.add_argument("--disasm", action="append", default=[], metavar="HEXADDR:COUNT",
                       help="disassemble guest code after the run (repeatable)")
    smoke.add_argument("--peek", action="append", default=[], metavar="HEXADDR:COUNT",
                       help="print guest u64 words after the run (repeatable)")
    smoke.add_argument("--watch", metavar="HEXADDR:LEN",
                       help="log every guest/SVC write overlapping this range with its PC")
    smoke.add_argument("--trace", action="append", default=[], metavar="HEXPC",
                       help="record registers each time this PC executes (repeatable)")
    smoke.set_defaults(run=command_smoke)
    arguments = parser.parse_args()
    return arguments.run(arguments)


if __name__ == "__main__":
    raise SystemExit(main())
