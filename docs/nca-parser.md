# NCA3 header parser

The loader parses the **decrypted NCA3 main header** into bounded metadata.
The API requires the original archive length and checks that the declared
content size matches it. Four section entries are decoded from 0x200-byte media
units into exclusive byte ranges; the four stored 32-byte filesystem-header
hashes remain opaque until section headers are available.

The parser validates magic, supported distribution/content/key-area values,
reserved fields, archive-size alignment, valid half-open section ranges, file
bounds, and pairwise section overlap. It preserves each section's table index even if
the physical ranges appear in another order.

Raw NCA headers are encrypted. `decrypt_nca3_header` decrypts only the initial
0xC00-byte header region, including its four 0x200-byte filesystem headers,
using a caller-supplied header key. `NcaHeader::parse_decrypted` parses the
decrypted main header. Neither validates RSA signatures, verifies the stored
filesystem-header hashes, decrypts section payloads, parses section
filesystems, or derives keys. No game files or keys are included in this
repository.

The documented layout is based on the [NCA format reference](https://www.switchbrew.org/wiki/NCA),
including its warning that format details describe plaintext data. Implementation
tests use synthetic headers only; they do not assert compatibility with a
commercial title or a real keyset.
