"""Small bounded client for the existing Codex private Unix WebSocket.

No daemon restart, public endpoint, model override or new agent process.
"""
import base64
import hashlib
import json
import os
import socket
import struct
import time
from pathlib import Path


class Control:
    def __init__(self, path=None):
        self.socket = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.socket.settimeout(12)
        self.socket.connect(str(path or Path.home() / '.codex/app-server-control/app-server-control.sock'))
        self.buffer = b''
        self.counter = 0
        key = base64.b64encode(os.urandom(16)).decode()
        self.socket.sendall(('GET / HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: ' + key + '\r\nSec-WebSocket-Version: 13\r\n\r\n').encode())
        header = b''
        while b'\r\n\r\n' not in header:
            data = self.socket.recv(4096)
            if not data or len(header) > 16384:
                raise ValueError('Invalid private control handshake')
            header += data
        head, self.buffer = header.split(b'\r\n\r\n', 1)
        expected = base64.b64encode(hashlib.sha1((key + '258EAFA5-E914-47DA-95CA-C5AB0DC85B11').encode()).digest())
        if not head.startswith(b'HTTP/1.1 101') or expected not in head:
            raise ValueError('Private control handshake rejected')
        self.request('initialize', {'clientInfo': {'name': 'emulator_report_watch', 'title': 'Emulator reports', 'version': '1.0'}, 'capabilities': {'experimentalApi': True}})
        self.send({'method': 'initialized'})

    def _read(self, size):
        while len(self.buffer) < size:
            data = self.socket.recv(max(4096, size - len(self.buffer)))
            if not data:
                raise EOFError('Private control connection closed')
            self.buffer += data
        value, self.buffer = self.buffer[:size], self.buffer[size:]
        return value

    def _frame(self, data, opcode=1):
        mask = os.urandom(4)
        size = len(data)
        if size < 126:
            head = bytes([0x80 | opcode, 0x80 | size])
        elif size < 65536:
            head = bytes([0x80 | opcode, 0x80 | 126]) + struct.pack('>H', size)
        else:
            head = bytes([0x80 | opcode, 0x80 | 127]) + struct.pack('>Q', size)
        self.socket.sendall(head + mask + bytes(value ^ mask[i % 4] for i, value in enumerate(data)))

    def send(self, value):
        self._frame(json.dumps(value, separators=(',', ':')).encode())

    def receive(self):
        fragments = bytearray()
        while True:
            a, b = self._read(2)
            size = b & 0x7f
            if size == 126:
                size = struct.unpack('>H', self._read(2))[0]
            elif size == 127:
                size = struct.unpack('>Q', self._read(8))[0]
            if size > 4 * 1024 * 1024 or len(fragments) + size > 4 * 1024 * 1024:
                raise ValueError('Private control message exceeds limit')
            if b & 0x80:
                raise ValueError('Server control frames must be unmasked')
            payload = self._read(size)
            opcode = a & 15
            if opcode == 9:
                self._frame(payload, 10)
                continue
            if opcode == 10:
                continue
            if opcode == 8:
                raise EOFError('Private control connection closed')
            if opcode not in (0, 1):
                raise ValueError('Unexpected private control frame')
            fragments.extend(payload)
            if a & 0x80:
                return json.loads(fragments)

    def request(self, method, params=None, timeout=15):
        self.counter += 1
        request_id = self.counter
        self.send({'id': request_id, 'method': method, 'params': params})
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            self.socket.settimeout(max(.1, end - time.monotonic()))
            value = self.receive()
            if value.get('id') == request_id:
                if 'error' in value:
                    raise RuntimeError('Codex report handoff was rejected')
                return value.get('result')
        raise TimeoutError('Codex report handoff timed out')

    def close(self):
        self.socket.close()
