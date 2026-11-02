#!/usr/bin/env python3
"""Generate QA-only documents exclusively from our independently assembled NROs."""
from pathlib import Path
root=Path(__file__).resolve().parent.parent
out=root/'tools/qa-documents/build/generated/qa-assets';out.mkdir(parents=True,exist_ok=True)
hello=(root/'examples/homebrew/hello.nro').read_bytes();ipc=(root/'examples/homebrew/ipc-clone.nro').read_bytes()
for name,data in [('hello.nro',hello),('duplicate.nro',hello),('slow-hello.nro',hello),('slow-ipc.nro',ipc),('invalid.nro',bytes(4096))]:(out/name).write_bytes(data)
bad=bytearray(hello);bad[:4]=(0).to_bytes(4,'little');(out/'bad-opcode.nro').write_bytes(bad)
bad=bytearray(hello);bad[4:8]=(1).to_bytes(4,'little');(out/'module.nro').write_bytes(bad)
print('Seven owned QA documents generated; no emulator core or console content')
