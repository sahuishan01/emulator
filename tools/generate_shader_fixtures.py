#!/usr/bin/env python3
"""Build original raw SM53 shader fixtures; optionally verify NVIDIA disassembly."""
import argparse
from pathlib import Path
import re
import struct
import subprocess
import tempfile

NOP = 0x50b0000000070f00
EXIT = 0xe30000000007000f
CONTROL = 0x001f8000fc0007e0

def imm(r, x):
    bits = struct.unpack('<I', struct.pack('<f', x))[0]
    return 0x010000000007f000 | r | bits << 20, f'MOV32I R{r}, 0x{bits:x}'

def mov(d, a):
    return 0x5c98078000070000 | d | a << 20, f'MOV R{d}, ' + ('RZ' if a == 255 else f'R{a}')

def math(op, d, a, b):
    return (0x5c58000000070000 if op == 'FADD' else 0x5c68000000070000) | d | a << 8 | b << 20, f'{op} R{d}, R{a}, R{b}'

def fixtures():
    load = (0xefd9ff800807ff00, 'ALD.128 R0, a[0x80]')
    store = (0xeff1ff800707ff00, 'AST.128 a[0x70], R0')
    end = (EXIT, 'EXIT')
    return {
        'passthrough.vert': [load, store, end],
        'transform.vert': [load, imm(4, .5), math('FMUL', 0, 0, 4), math('FMUL', 1, 1, 4), mov(8, 0), mov(0, 8), store, end],
        'green.frag': [imm(8, .5), math('FADD', 1, 8, 8), imm(0, 0), mov(2, 255), imm(3, 1), end],
        'red.frag': [imm(8, .5), math('FADD', 0, 8, 8), mov(2, 255), mov(1, 255), imm(3, 1), end],
    }

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true')
    parser.add_argument('--nvdisasm', type=Path)
    parser.add_argument('--validate-spirv', action='store_true')
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent / 'examples/shaders'
    root.mkdir(parents=True, exist_ok=True)
    for name, operations in fixtures().items():
        ops = operations[:]
        while len(ops) % 3:
            ops.append((NOP, 'NOP'))
        words = []
        for i in range(0, len(ops), 3):
            words += [CONTROL] + [w for w, _ in ops[i:i+3]]
        raw = struct.pack('<' + 'Q' * len(words), *words)
        path = root / (name + '.bin')
        listing = '\n'.join(text for _, text in ops) + '\n'
        for target, content in [(path, raw), (root / (name + '.sass'), listing.encode())]:
            if args.check:
                assert target.read_bytes() == content, f'{target} differs'
            else:
                target.write_bytes(content)
        if args.nvdisasm:
            with tempfile.TemporaryDirectory() as temporary:
                binary = Path(temporary) / 'shader.bin'
                binary.write_bytes(raw)
                result = subprocess.run([str(args.nvdisasm.resolve()), '-b', 'SM53', '-hex', str(binary)], check=True, capture_output=True, text=True)
            actual = re.findall(r'/\*[0-9a-f]+\*/\s+(.+?);', result.stdout)
            assert actual == [text for _, text in ops], (name, actual, ops)
        print(f'{name}: {len(raw)} bytes / {len(operations)} instructions' + (' / NVIDIA SM53 exact disassembly match' if args.nvdisasm else ''))
    if args.validate_spirv:
        with tempfile.TemporaryDirectory(prefix='emulator-owned-spirv-') as temporary:
            subprocess.run(['cargo', 'run', '--offline', '-q', '-p', 'emulator-shader', '--example', 'translate_fixtures', '--', temporary], cwd=root.parent.parent, check=True)
            modules = sorted(Path(temporary).glob('*.spv'))
            assert len(modules) == 4
            for module in modules:
                subprocess.run(['spirv-val', '--target-env', 'vulkan1.1', str(module)], check=True)
        print('All four original emitted modules passed Vulkan 1.1 SPIR-V validation')


if __name__ == '__main__':
    main()
