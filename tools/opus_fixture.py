#!/usr/bin/env python3
"""Generate the emulator-opus regression fixture with the host libopus.

Host-only developer tool. It encodes deterministic synthetic audio with the
system libopus (via ctypes) in many configurations, decodes it again with
libopus, and stores packets, encoder final ranges and the reference PCM in
crates/opus/tests/data/libopus_fixture.bin. The emulator never links libopus;
this file only produces test expectations.

usage: python3 -I tools/opus_fixture.py [output]
"""
import ctypes
import math
import struct
import sys

lib = ctypes.CDLL("libopus.so.0")
lib.opus_encoder_create.restype = ctypes.c_void_p
lib.opus_encoder_create.argtypes = [ctypes.c_int32, ctypes.c_int, ctypes.c_int, ctypes.POINTER(ctypes.c_int)]
lib.opus_decoder_create.restype = ctypes.c_void_p
lib.opus_decoder_create.argtypes = [ctypes.c_int32, ctypes.c_int, ctypes.POINTER(ctypes.c_int)]
lib.opus_encode.argtypes = [ctypes.c_void_p, ctypes.POINTER(ctypes.c_int16), ctypes.c_int, ctypes.c_char_p, ctypes.c_int32]
lib.opus_decode.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_int32, ctypes.POINTER(ctypes.c_int16), ctypes.c_int, ctypes.c_int]
lib.opus_multistream_encoder_create.restype = ctypes.c_void_p
lib.opus_multistream_encoder_create.argtypes = [ctypes.c_int32, ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_char_p, ctypes.c_int, ctypes.POINTER(ctypes.c_int)]
lib.opus_multistream_decoder_create.restype = ctypes.c_void_p
lib.opus_multistream_decoder_create.argtypes = [ctypes.c_int32, ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_char_p, ctypes.POINTER(ctypes.c_int)]
lib.opus_multistream_encode.argtypes = [ctypes.c_void_p, ctypes.POINTER(ctypes.c_int16), ctypes.c_int, ctypes.c_char_p, ctypes.c_int32]
lib.opus_multistream_decode.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_int32, ctypes.POINTER(ctypes.c_int16), ctypes.c_int, ctypes.c_int]

APP_VOIP, APP_AUDIO, APP_LOWDELAY = 2048, 2049, 2051
SET_BITRATE, SET_VBR, SET_BANDWIDTH, SET_COMPLEXITY = 4002, 4006, 4008, 4010
SET_INBAND_FEC, SET_PACKET_LOSS_PERC, SET_FORCE_CHANNELS = 4012, 4014, 4022
SET_EXPERT_FRAME_DURATION, GET_FINAL_RANGE, SET_FORCE_MODE = 4040, 4031, 11002
SILK, HYBRID, CELT = 1000, 1001, 1002
NB, MB, WB, SWB, FB = 1101, 1102, 1103, 1104, 1105
DURATION = {2.5: 5001, 5: 5002, 10: 5003, 20: 5004, 40: 5005, 60: 5006, 80: 5007, 100: 5008, 120: 5009}


def ctl(fn, st, req, val=None):
    if val is None:
        out = ctypes.c_uint32()
        assert fn(ctypes.c_void_p(st), ctypes.c_int(req), ctypes.byref(out)) == 0
        return out.value
    assert fn(ctypes.c_void_p(st), ctypes.c_int(req), ctypes.c_int(val)) == 0


class Lcg:
    def __init__(self, seed):
        self.s = seed

    def next(self):
        self.s = (self.s * 1103515245 + 12345) & 0x7FFFFFFF
        return self.s / 0x7FFFFFFF * 2 - 1


def signal(n, channels, seed):
    """Tones, a chirp, noise bursts and a few clicks (transients)."""
    rng = Lcg(seed)
    out = []
    for i in range(n):
        t = i / 48000
        base = 0.25 * math.sin(2 * math.pi * 220 * t) + 0.15 * math.sin(2 * math.pi * (300 + 2000 * t) * t)
        burst = 0.2 * rng.next() if (i // 4800) % 3 == 1 else 0.02 * rng.next()
        click = 0.8 if i % 7000 == 0 else 0.0
        for c in range(channels):
            v = base * (1.0 if c % 2 == 0 else 0.7) + burst + click + 0.05 * math.sin(2 * math.pi * 1000 * (c + 1) * t)
            out.append(max(-32768, min(32767, int(v * 20000))))
    return out


class Stream:
    def __init__(self, name, rate, channels, ms=None):
        self.name, self.rate, self.channels, self.ms = name, rate, channels, ms
        self.ops = []  # (op, packet, final_range, pcm)


def encode_single(name, rate, channels, frames, app, ctls, frame_ms=20, enc_channels=None,
                  lost=(), fec_recover=(), per_frame_ctls=None, seed=1):
    enc_channels = enc_channels or channels
    err = ctypes.c_int()
    enc = lib.opus_encoder_create(48000, enc_channels, app, ctypes.byref(err))
    assert err.value == 0
    dec = lib.opus_decoder_create(rate, channels, ctypes.byref(err))
    assert err.value == 0
    for req, val in ctls:
        ctl(lib.opus_encoder_ctl, enc, req, val)
    ctl(lib.opus_encoder_ctl, enc, SET_EXPERT_FRAME_DURATION, DURATION[frame_ms])
    fs = int(48000 * frame_ms / 1000)
    audio = signal(fs * frames, enc_channels, seed)
    st = Stream(name, rate, channels)
    buf = (ctypes.c_int16 * (5760 * channels))()
    data = ctypes.create_string_buffer(4000)
    packets = []
    for f in range(frames):
        if per_frame_ctls and f in per_frame_ctls:
            for req, val in per_frame_ctls[f]:
                ctl(lib.opus_encoder_ctl, enc, req, val)
        pcm = (ctypes.c_int16 * (fs * enc_channels))(*audio[f * fs * enc_channels:(f + 1) * fs * enc_channels])
        n = lib.opus_encode(enc, pcm, fs, data, 4000)
        assert n > 0, n
        packets.append((data.raw[:n], ctl(lib.opus_encoder_ctl, enc, GET_FINAL_RANGE)))
    out_fs = rate * frame_ms // 1000
    for f, (pkt, rng) in enumerate(packets):
        if f in lost:
            r = lib.opus_decode(dec, None, 0, buf, out_fs, 0)
            st.ops.append((1, b"", 0, list(buf[: r * channels])))
            continue
        if f in fec_recover:
            r = lib.opus_decode(dec, pkt, len(pkt), buf, out_fs, 1)
            assert r > 0, r
            st.ops.append((2, pkt, 0, list(buf[: r * channels])))
        r = lib.opus_decode(dec, pkt, len(pkt), buf, 5760, 0)
        assert r > 0, r
        st.ops.append((0, pkt, rng, list(buf[: r * channels])))
    return st


def encode_multistream(name, rate, channels, streams, coupled, mapping, frames, bitrate):
    err = ctypes.c_int()
    m = bytes(mapping)
    enc = lib.opus_multistream_encoder_create(48000, channels, streams, coupled, m, APP_AUDIO, ctypes.byref(err))
    assert err.value == 0
    dec = lib.opus_multistream_decoder_create(rate, channels, streams, coupled, m, ctypes.byref(err))
    assert err.value == 0
    ctl(lib.opus_multistream_encoder_ctl, enc, SET_BITRATE, bitrate)
    fs = 960
    audio = signal(fs * frames, channels, 7)
    st = Stream(name, rate, channels, (streams, coupled, mapping))
    buf = (ctypes.c_int16 * (5760 * channels))()
    data = ctypes.create_string_buffer(8000)
    for f in range(frames):
        pcm = (ctypes.c_int16 * (fs * channels))(*audio[f * fs * channels:(f + 1) * fs * channels])
        n = lib.opus_multistream_encode(enc, pcm, fs, data, 8000)
        assert n > 0, n
        pkt = data.raw[:n]
        rng = ctl(lib.opus_multistream_encoder_ctl, enc, GET_FINAL_RANGE)
        r = lib.opus_multistream_decode(dec, pkt, n, buf, 5760, 0)
        assert r > 0, r
        st.ops.append((0, pkt, rng, list(buf[: r * channels])))
    return st


def build():
    s = []
    s.append(encode_single("silk_nb_mono_20ms_8k", 8000, 1, 8, APP_VOIP,
                           [(SET_FORCE_MODE, SILK), (SET_BANDWIDTH, NB), (SET_BITRATE, 12000)], 20))
    s.append(encode_single("silk_mb_mono_60ms_12k", 12000, 1, 3, APP_VOIP,
                           [(SET_FORCE_MODE, SILK), (SET_BANDWIDTH, MB), (SET_BITRATE, 16000)], 60))
    s.append(encode_single("silk_wb_stereo_40ms_16k", 16000, 2, 4, APP_VOIP,
                           [(SET_FORCE_MODE, SILK), (SET_BANDWIDTH, WB), (SET_BITRATE, 32000)], 40))
    # Narrowband on purpose: libopus builds with deep PLC skip the
    # loss-recovery fade at 16 kHz, so only NB/MB expectations are portable.
    s.append(encode_single("silk_nb_stereo_20ms_48k_plc_fec", 48000, 2, 10, APP_VOIP,
                           [(SET_FORCE_MODE, SILK), (SET_BANDWIDTH, NB), (SET_BITRATE, 24000),
                            (SET_INBAND_FEC, 1), (SET_PACKET_LOSS_PERC, 30)], 20,
                           lost=(4, 7), fec_recover=(5,)))
    s.append(encode_single("hybrid_fb_stereo_20ms_48k", 48000, 2, 4, APP_AUDIO,
                           [(SET_FORCE_MODE, HYBRID), (SET_BANDWIDTH, FB), (SET_BITRATE, 48000)], 20))
    s.append(encode_single("hybrid_swb_mono_10ms_24k", 24000, 1, 8, APP_VOIP,
                           [(SET_FORCE_MODE, HYBRID), (SET_BANDWIDTH, SWB), (SET_BITRATE, 32000)], 10))
    s.append(encode_single("celt_fb_stereo_20ms_48k", 48000, 2, 4, APP_AUDIO,
                           [(SET_FORCE_MODE, CELT), (SET_BITRATE, 96000)], 20))
    s.append(encode_single("celt_fb_mono_2_5ms_48k", 48000, 1, 16, APP_LOWDELAY,
                           [(SET_BITRATE, 64000)], 2.5))
    s.append(encode_single("celt_wb_stereo_5ms_16k_from_mono", 16000, 2, 12, APP_LOWDELAY,
                           [(SET_BANDWIDTH, WB), (SET_BITRATE, 32000)], 5, enc_channels=1))
    s.append(encode_single("celt_stereo_to_mono_10ms_48k_plc", 48000, 1, 10, APP_AUDIO,
                           [(SET_FORCE_MODE, CELT), (SET_BITRATE, 64000)], 10, enc_channels=2, lost=(3, 4, 5)))
    s.append(encode_single("mixed_120ms_24k", 24000, 1, 2, APP_AUDIO,
                           [(SET_BITRATE, 64000)], 120, enc_channels=2))
    switches = {
        2: [(SET_FORCE_MODE, HYBRID), (SET_BANDWIDTH, SWB)],
        4: [(SET_FORCE_MODE, CELT), (SET_BANDWIDTH, FB)],
        6: [(SET_FORCE_MODE, SILK), (SET_BANDWIDTH, WB)],
        8: [(SET_FORCE_MODE, CELT), (SET_BANDWIDTH, FB)],
        10: [(SET_FORCE_MODE, HYBRID), (SET_BANDWIDTH, FB)],
        12: [(SET_FORCE_MODE, SILK), (SET_BANDWIDTH, NB)],
    }
    s.append(encode_single("mode_switching_20ms_48k", 48000, 2, 14, APP_AUDIO,
                           [(SET_FORCE_MODE, SILK), (SET_BANDWIDTH, MB), (SET_BITRATE, 48000)], 20,
                           per_frame_ctls=switches))
    s.append(encode_multistream("multistream_5_1_48k", 48000, 6, 4, 2, [0, 4, 1, 2, 3, 5], 2, 192000))
    return s


def write(streams, path):
    out = bytearray(b"OPFX")
    out += struct.pack("<II", 1, len(streams))
    for st in streams:
        name = st.name.encode()
        out += struct.pack("<B", len(name)) + name
        out += struct.pack("<II", st.rate, st.channels)
        if st.ms:
            streams_, coupled, mapping = st.ms
            out += struct.pack("<II", streams_, coupled) + bytes(mapping)
        else:
            out += struct.pack("<II", 0, 0)
        out += struct.pack("<I", len(st.ops))
        for op, pkt, rng, pcm in st.ops:
            out += struct.pack("<BII", op, len(pkt), rng) + pkt
            out += struct.pack("<I", len(pcm)) + struct.pack(f"<{len(pcm)}h", *pcm)
    open(path, "wb").write(out)
    print(f"wrote {path}: {len(out)} bytes, {len(streams)} streams")


if __name__ == "__main__":
    write(build(), sys.argv[1] if len(sys.argv) > 1 else "crates/opus/tests/data/libopus_fixture.bin")
