#!/usr/bin/env python3
"""Generate original MAXWELL_B clear command fixtures from public register fields."""
import argparse
from pathlib import Path
import struct

ROOT = Path(__file__).resolve().parent.parent


def commands(format_code, masked=False):
    one = 0x3f800000
    methods = [(0, 0xb197), (0x800, 1), (0x804, 0x800), (0x808, 64),
               (0x80c, 8), (0x810, format_code), (0x814, 0x1000), (0x818, 1),
               (0x81c, 0), (0x820, 0), (0xff4, 16 << 16), (0xff8, 8 << 16),
               (0x10f8, 0), (0xd80, 0), (0xd84, 0), (0xd88, one),
               (0xd8c, one), (0x19d0, 0x3c),
               (0x10f8, 0x10), (0xd6c, 8 << 16), (0xd70, 8 << 16),
               (0xd80, one), (0xd88, 0), (0x19d0, 0x3c),
               (0xd6c, (12 << 16) | 4), (0xd70, (6 << 16) | 2),
               (0xd80, 0), (0xd84, one), (0x19d0, 0x08 if masked else 0x3c)]
    return b''.join(struct.pack('<II', 0x20010000 | (offset // 4), data)
                    for offset, data in methods)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    for name, data in [('rgba', commands(0xd5)), ('bgra', commands(0xcf)),
                       ('masked', commands(0xd5, True))]:
        target = ROOT / 'examples/gpu' / f'color-clear-{name}.bin'
        if args.check:
            assert target.read_bytes() == data, target
        else:
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(data)
        print(f'{target.name}: {len(data)} bytes / {len(data)//4} words')


if __name__ == '__main__':
    main()
