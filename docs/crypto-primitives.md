# NCA crypto foundations

The new dependency-free `emulator-crypto` crate provides an original AES-128
block implementation, big-endian-counter CTR XOR, and full-block XTS encrypt /
decrypt operations. CTR callers supply the 128-bit initial counter explicitly;
NCA-specific counter construction is not inferred. XTS accepts one data unit
and an explicit tweak input, and rejects partial blocks instead of silently
applying unsupported ciphertext stealing.

`KeyFile` parses bounded user-provided `name = hex` files in memory. It exposes
lookups for the header key, per-generation application/ocean/system key-area
keys, title Keks, and rights-ID title keys. It enforces file, entry, name, and
key-size limits, rejects duplicate names, redacts `Debug`, and makes a best-
effort to overwrite key bytes on drop. It does not read paths, save key files,
log values, or implement the NCA key hierarchy.

The implementation passes AES-128 FIPS 197, CTR SP 800-38A, and XTS known-answer
vectors plus round-trip, length-rejection, counter-overflow, and key-file
validation tests. NIST describes XTS as storage confidentiality without
authentication; this implementation therefore does not establish content
integrity or signature authenticity. AES uses lookup tables and is not
constant-time or production-hardened. Keep it as a development correctness
prototype until replaced or hardened and independently reviewed.

References: [NIST FIPS 197](https://csrc.nist.gov/pubs/fips/197/final),
[NIST SP 800-38A](https://csrc.nist.gov/pubs/sp/800/38/a/final),
[NIST SP 800-38E](https://csrc.nist.gov/pubs/sp/800/38/e/final), and the
[NIST informal mode test-vector index](https://csrc.nist.gov/projects/cryptographic-algorithm-validation-program/cavp-testing-block-cipher-modes).
