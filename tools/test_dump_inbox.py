#!/usr/bin/env python3
"""Loopback tests for the private dump inbox: resume, rejection, rollback and lifetime."""
import hashlib
import http.client
import json
import socket
import tempfile
import threading
import unittest
from pathlib import Path

import dump_inbox
import dump_upload

HEADERS = {dump_inbox.CSRF_HEADER: "1"}
KEYS = b"; comment\nheader_key = 00112233445566778899aabbccddeeff\n\naes_kek_generation_source = ffeeddccbbaa99887766554433221100\n"
NSP = b"PFS0" + bytes(range(256)) * 40


class DumpInboxTest(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name) / "dumps"
        self.server, self.inbox = dump_inbox.make_server(self.root, "127.0.0.1", 0, gate_cpu=False)
        self.port = self.server.server_address[1]
        threading.Thread(target=self.server.serve_forever, daemon=True).start()

    def tearDown(self):
        self.server.shutdown()
        self.server.server_close()
        self.temporary.cleanup()

    def call(self, method, path, body=None, headers=HEADERS):
        connection = http.client.HTTPConnection("127.0.0.1", self.port, timeout=10)
        connection.request(method, path, body=body, headers=dict(headers))
        response = connection.getresponse()
        raw = response.read()
        connection.close()
        is_json = response.getheader("Content-Type") == "application/json"
        return response.status, (json.loads(raw) if is_json else raw)

    def begin(self, name, size):
        return self.call("POST", "/api/upload/begin", json.dumps({"name": name, "size": size}))

    def put(self, upload_id, offset, data, digest=None):
        headers = dict(HEADERS)
        headers["X-Chunk-Sha256"] = digest or hashlib.sha256(data).hexdigest()
        return self.call("PUT", f"/api/upload/{upload_id}?offset={offset}", data, headers)

    def store(self, name, data):
        status, begun = self.begin(name, len(data))
        self.assertEqual(status, 200, begun)
        self.assertEqual(self.put(begun["id"], begun["received"], data)[0], 200)
        return self.call("POST", f"/api/upload/{begun['id']}/finish")

    def test_chunked_upload_resumes_and_is_hashed(self):
        status, begun = self.begin("Game [v0].nsp", len(NSP))
        self.assertEqual((status, begun["received"], begun["kind"]), (200, 0, "games"))
        upload_id = begun["id"]
        self.assertEqual(self.put(upload_id, 0, NSP[:5000])[1]["received"], 5000)
        # A retried chunk at a stale offset reports where the server really is.
        status, conflict = self.put(upload_id, 0, NSP[:5000])
        self.assertEqual((status, conflict["received"]), (409, 5000))
        # A corrupted chunk is refused and nothing is appended.
        status, corrupt = self.put(upload_id, 5000, NSP[5000:], digest="0" * 64)
        self.assertEqual((status, corrupt["received"]), (422, 5000))
        # Finishing early is refused; beginning again resumes rather than restarting.
        self.assertEqual(self.call("POST", f"/api/upload/{upload_id}/finish")[0], 409)
        status, resumed = self.begin("Game [v0].nsp", len(NSP))
        self.assertEqual((resumed["id"], resumed["received"]), (upload_id, 5000))
        self.assertEqual(self.call("GET", "/api/files")[1]["partial"][0]["received"], 5000)
        # Overrunning the declared size is refused.
        self.assertEqual(self.put(upload_id, 5000, NSP[5000:] + b"x")[0], 400)
        self.assertEqual(self.put(upload_id, 5000, NSP[5000:])[0], 200)
        status, done = self.call("POST", f"/api/upload/{upload_id}/finish")
        self.assertEqual((status, done["stored"]), (200, "games/Game [v0].nsp"))
        stored = self.root / "games" / "Game [v0].nsp"
        self.assertEqual(stored.read_bytes(), NSP)
        self.assertEqual(stored.stat().st_mode & 0o777, 0o600)
        self.assertEqual(list((self.root / "incoming").iterdir()), [])
        self.inbox._hash_wake.set()
        for _ in range(100):
            listing = self.call("GET", "/api/files")[1]
            if listing["files"][0]["sha256"]:
                break
            threading.Event().wait(0.05)
        self.assertEqual(listing["files"][0]["sha256"], hashlib.sha256(NSP).hexdigest())
        self.assertEqual(listing["partial"], [])
        self.assertEqual(dump_inbox.read_listing(self.root)["files"][0]["sha256"], hashlib.sha256(NSP).hexdigest())
        # Lifetime: delete removes the file.
        self.assertEqual(self.call("DELETE", "/api/files/games/Game%20%5Bv0%5D.nsp")[0], 200)
        self.assertFalse(stored.exists())
        self.assertEqual(self.call("DELETE", "/api/files/games/Game%20%5Bv0%5D.nsp")[0], 404)

    def test_key_files_are_normalised_validated_and_never_echoed(self):
        status, done = self.store("PROD.KEYS", KEYS)
        self.assertEqual((status, done["stored"]), (200, "keys/prod.keys"))
        self.assertEqual((self.root / "keys" / "prod.keys").read_bytes(), KEYS)
        self.assertNotIn("00112233445566778899aabbccddeeff", json.dumps(self.call("GET", "/api/files")[1]))
        # Invalid replacement is rejected, rolled back, and leaves the good file intact.
        status, error = self.store("prod.keys", b"this is not a key file\n")
        self.assertEqual(status, 422, error)
        self.assertEqual((self.root / "keys" / "prod.keys").read_bytes(), KEYS)
        self.assertEqual(list((self.root / "incoming").iterdir()), [])
        self.assertEqual(self.store("title.keys", b"\xff\xfe\x00binary")[0], 422)
        self.assertEqual(self.begin("prod.keys", dump_inbox.MAX_KEY_FILE + 1)[0], 413)

    def test_rejects_wrong_content_names_and_unauthenticated_shapes(self):
        self.assertEqual(self.store("fake.nsp", b"NOPE" + b"\0" * 64)[0], 422)
        self.assertEqual(self.store("fake.xci", b"\0" * 0x200)[0], 422)
        self.assertEqual(self.store("real.xci", b"\0" * 0x100 + b"HEAD" + b"\0" * 16)[0], 200)
        self.assertEqual(list((self.root / "games").iterdir()), [self.root / "games" / "real.xci"])
        for name in ("../prod.keys", "a/b.nsp", ".hidden.nsp", "evil.sh", "x..nsp", "", "a\\b.nsp", "n" * 300 + ".nsp"):
            self.assertEqual(self.begin(name, 10)[0], 400, name)
        # Firmware NCAs: content-ID names only, at least a header long.
        nca = "0123456789abcdef0123456789abcdef.nca"
        self.assertEqual(self.store(nca, b"\0" * 0x10)[0], 422)
        self.assertEqual(self.store(nca.upper().replace(".NCA", ".nca"), b"\1" * 0xC00)[0], 200)
        self.assertEqual(list((self.root / "firmware").iterdir()), [self.root / "firmware" / nca])
        for name in ("font.nca", "0123.nca", nca + ".zip"):
            self.assertEqual(self.begin(name, 0xC00)[0], 400, name)
        self.assertEqual(self.begin(nca, dump_inbox.MAX_FIRMWARE_FILE + 1)[0], 413)
        self.assertEqual(self.begin("a.nsp", 0)[0], 413)
        self.assertEqual(self.begin("a.nsp", dump_inbox.MAX_FILE + 1)[0], 413)
        self.assertEqual(self.begin("a.nsp", "10")[0], 400)
        self.assertEqual(self.call("POST", "/api/upload/begin", "[]")[0], 400)
        # Mutations without the custom header are refused (cross-site form posts cannot set it).
        self.assertEqual(self.call("POST", "/api/upload/begin", json.dumps({"name": "a.nsp", "size": 10}), {})[0], 403)
        self.assertEqual(self.call("DELETE", "/api/files/games/real.xci", headers={})[0], 403)
        self.assertEqual(self.put("f" * 32, 0, b"data")[0], 404)
        self.assertEqual(self.put("../../etc/passwd", 0, b"data")[0], 404)
        self.assertEqual(self.put("not-an-id", 0, b"data")[0], 400)
        self.assertEqual(self.call("DELETE", "/api/files/incoming/real.xci")[0], 404)
        self.assertEqual(self.call("GET", "/api/nope")[0], 404)

    def test_abort_discards_partial_and_page_is_served(self):
        _, begun = self.begin("b.nsp", len(NSP))
        self.put(begun["id"], 0, NSP[:100])
        self.assertEqual(self.call("DELETE", f"/api/upload/{begun['id']}")[0], 200)
        self.assertEqual(list((self.root / "incoming").iterdir()), [])
        self.assertEqual(self.begin("b.nsp", len(NSP))[1]["received"], 0)
        status, page = self.call("GET", "/")
        self.assertEqual(status, 200)
        self.assertIn(b"EMULATOR DUMP INBOX", page)
        self.assertEqual(self.call("GET", "/healthz")[1], {"ok": True})


    def test_terminal_uploader_resumes_and_survives_a_dropped_client(self):
        big = b"PFS0" + bytes(range(256)) * 4096  # several chunks at the patched chunk size
        source = Path(self.temporary.name) / "Big.nsp"
        source.write_bytes(big)
        client = dump_upload.Client(f"http://127.0.0.1:{self.port}", None, None)
        original = dump_inbox.CLIENT_CHUNK
        dump_inbox.CLIENT_CHUNK = 100_000
        try:
            # Someone else (an interrupted browser tab) already stored a prefix.
            _, begun = self.begin("Big.nsp", len(big))
            self.put(begun["id"], 0, big[:250_000])
            # A client that vanishes mid-body must not kill the server or append anything.
            raw = socket.create_connection(("127.0.0.1", self.port))
            raw.sendall(f"PUT /api/upload/{begun['id']}?offset=250000 HTTP/1.1\r\nHost: x\r\n"
                        f"X-Dump-Inbox: 1\r\nContent-Length: 100000\r\n\r\n".encode() + b"x" * 10)
            raw.close()
            # The first chunk attempt fails at the network layer and is retried.
            real_call, failures = client.call, [OSError("simulated drop")]
            def flaky(method, path, body=None, headers=None):
                if method == "PUT" and failures:
                    raise failures.pop()
                return real_call(method, path, body, headers)
            client.call = flaky
            real_sleep = dump_upload.time.sleep
            dump_upload.time.sleep = lambda _: None
            try:
                dump_upload.upload(client, source, quiet=True)
            finally:
                dump_upload.time.sleep = real_sleep
        finally:
            dump_inbox.CLIENT_CHUNK = original
        self.assertEqual(failures, [])
        self.assertEqual((self.root / "games" / "Big.nsp").read_bytes(), big)
        self.assertEqual(list((self.root / "incoming").iterdir()), [])
        # Wrong content is a fatal error, not an endless retry.
        bad = Path(self.temporary.name) / "Bad.nsp"
        bad.write_bytes(b"nope" * 10)
        with self.assertRaises(dump_upload.Fatal):
            dump_upload.upload(client, bad, quiet=True)
        status, script = self.call("GET", "/dump_upload.py")
        self.assertEqual(status, 200)
        self.assertEqual(script, Path(dump_upload.__file__).read_bytes())


class SmokeSandboxTest(unittest.TestCase):
    def test_smoke_command_confines_the_run_and_binds_only_its_inputs(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "dumps"
            (root / "games").mkdir(parents=True)
            (root / "keys").mkdir()
            game, keys, binary = root / "games" / "Game.NSP", root / "keys" / "prod.keys", Path(temporary) / "emulator-cli"
            for path in (game, keys, binary):
                path.write_bytes(b"x")
            command = dump_inbox.sandbox_command(binary, game, keys, None, 1234, root, "2G", 90)
            split = command.index("--")
            outer, inner = command[:split], command[split + 1:]
            self.assertEqual(outer[0], "systemd-run")
            properties = [outer[index + 1] for index, value in enumerate(outer) if value == "-p"]
            for required in ("PrivateNetwork=true", "NoNewPrivileges=true", "ProtectSystem=strict", "ProtectHome=true",
                             "CapabilityBoundingSet=", "LimitCORE=0", "MemoryMax=2G", "RuntimeMaxSec=90",
                             f"InaccessiblePaths={root}", f"User={game.stat().st_uid}"):
                self.assertIn(required, properties)
            self.assertTrue(any("@network-io" in value and value.startswith("SystemCallFilter=~") for value in properties))
            binds = sorted(value for value in properties if value.startswith("BindReadOnlyPaths="))
            self.assertEqual(binds, sorted(f"BindReadOnlyPaths={source}:{dump_inbox.SANDBOX_DIR}/{name}" for source, name in
                                           ((binary, "emulator-cli"), (game, "game.nsp"), (keys, "prod.keys"))))
            self.assertFalse(any(value.startswith(("BindPaths=", "ReadWritePaths=")) for value in properties))
            # The guest-facing command names sandbox paths only, never host paths.
            self.assertEqual(inner, [f"{dump_inbox.SANDBOX_DIR}/emulator-cli", "run-nsp", f"{dump_inbox.SANDBOX_DIR}/game.nsp",
                                     "--keys", f"{dump_inbox.SANDBOX_DIR}/prod.keys", "--max-steps", "1234"])
            titled = dump_inbox.sandbox_command(binary, game, keys, keys, 1, root, "2G", 90)
            self.assertEqual(titled[-2:], ["--titlekeys", f"{dump_inbox.SANDBOX_DIR}/title.keys"])
            odd = root / "games" / "a:b.nsp"
            odd.write_bytes(b"x")
            with self.assertRaises(ValueError):
                dump_inbox.sandbox_command(binary, odd, keys, None, 1, root, "2G", 90)


if __name__ == "__main__":
    unittest.main()
