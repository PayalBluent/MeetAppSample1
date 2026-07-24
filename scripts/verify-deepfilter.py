#!/usr/bin/env python3
"""Prove DeepFilterNet is actually processing audio.

Runs the `deep-filter` CLI on an input WAV and compares input vs. output:
  - MD5        : must DIFFER  -> the audio was really processed, not copied
  - format     : must MATCH   -> same rate/channels/frames, so transcript
                                 timestamps & diarization stay valid
  - noise floor: should DROP  -> DeepFilterNet suppressed background noise
                                 (10th-percentile short-window RMS, a proxy for
                                  the quiet/noise-only parts of the recording)

Stdlib only (wave, array, hashlib) — no numpy/torch needed.

Usage (PowerShell):
  python scripts/verify-deepfilter.py "<input.wav>" [path-to-deep-filter.exe]

If the binary path is omitted it uses $env:MEETAPP_DEEPFILTER_BIN, then
%USERPROFILE%\\deepfilter\\deep-filter.exe.
"""
import array
import hashlib
import math
import os
import struct
import subprocess
import sys
import tempfile


def md5(path):
    h = hashlib.md5()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def read_mono(path):
    """Robust minimal RIFF/WAVE reader → (mono float[-1..1], (channels, rate, frames)).

    Handles PCM16/PCM32/uint8, IEEE float32, and WAVE_FORMAT_EXTENSIBLE (0xFFFE),
    which the stdlib `wave` module rejects — MeetApp writes several of these.
    """
    with open(path, "rb") as f:
        data = f.read()
    if data[:4] != b"RIFF" or data[8:12] != b"WAVE":
        raise SystemExit(f"not a RIFF/WAVE file: {path}")
    fmt = None
    pcm = None
    pos = 12
    while pos + 8 <= len(data):
        cid = data[pos:pos + 4]
        size = struct.unpack_from("<I", data, pos + 4)[0]
        body = data[pos + 8:pos + 8 + size]
        if cid == b"fmt ":
            afmt, ch, rate, _br, _ba, bits = struct.unpack_from("<HHIIHH", body, 0)
            if afmt == 0xFFFE and len(body) >= 40:  # EXTENSIBLE → real fmt in SubFormat GUID
                afmt = struct.unpack_from("<H", body, 24)[0]
            fmt = (afmt, ch, rate, bits)
        elif cid == b"data":
            pcm = body
        pos += 8 + size + (size & 1)  # chunks are word-aligned
    if fmt is None or pcm is None:
        raise SystemExit("missing fmt/data chunk")
    afmt, ch, rate, bits = fmt

    if afmt == 3 and bits == 32:      # IEEE float
        a = array.array("f"); a.frombytes(pcm[:len(pcm) // 4 * 4]); norm = 1.0
    elif afmt == 1 and bits == 16:    # PCM16
        a = array.array("h"); a.frombytes(pcm[:len(pcm) // 2 * 2]); norm = 32768.0
    elif afmt == 1 and bits == 32:    # PCM32
        a = array.array("i"); a.frombytes(pcm[:len(pcm) // 4 * 4]); norm = 2147483648.0
    elif afmt == 1 and bits == 8:     # uint8
        a = array.array("b", bytes((x - 128) & 0xFF for x in pcm)); norm = 128.0
    else:
        raise SystemExit(f"unsupported WAV: format={afmt} bits={bits}")

    ch = max(1, ch)
    frames = len(a) // ch
    if ch > 1:
        mono = [sum(a[i:i + ch]) / (ch * norm) for i in range(0, frames * ch, ch)]
    else:
        mono = [s / norm for s in a]
    return mono, (ch, rate, frames)


def rms(xs):
    if not xs:
        return 0.0
    return math.sqrt(sum(x * x for x in xs) / len(xs))


def noise_floor(mono, rate, win_ms=20):
    """10th-percentile of per-window RMS = a proxy for the noise-only stretches."""
    n = max(1, int(rate * win_ms / 1000))
    windows = [rms(mono[i:i + n]) for i in range(0, len(mono), n) if mono[i:i + n]]
    if not windows:
        return 0.0
    windows.sort()
    return windows[max(0, int(len(windows) * 0.10) - 1)]


def db(x):
    return 20 * math.log10(x) if x > 1e-12 else -120.0


def main():
    if len(sys.argv) < 2:
        raise SystemExit(__doc__)
    inp = sys.argv[1]
    binary = (
        sys.argv[2] if len(sys.argv) > 2
        else os.environ.get("MEETAPP_DEEPFILTER_BIN")
        or os.path.join(os.environ.get("USERPROFILE", ""), "deepfilter", "deep-filter.exe")
    )
    if not os.path.isfile(inp):
        raise SystemExit(f"input not found: {inp}")
    if not os.path.isfile(binary):
        raise SystemExit(f"deep-filter binary not found: {binary}")

    outdir = tempfile.mkdtemp(prefix="verify-df-")
    print(f"binary : {binary}")
    print(f"input  : {inp}")
    cmd = [binary, "--output-dir", outdir, inp]
    print("running:", " ".join(f'"{c}"' if " " in c else c for c in cmd))
    r = subprocess.run(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    if r.returncode != 0:
        raise SystemExit(f"deep-filter exited with {r.returncode}")

    outs = [f for f in os.listdir(outdir) if f.lower().endswith(".wav")]
    if not outs:
        raise SystemExit("deep-filter produced no .wav output")
    out = os.path.join(outdir, outs[0])
    print(f"output : {out}\n")

    in_mono, in_fmt = read_mono(inp)
    out_mono, out_fmt = read_mono(out)
    in_md5, out_md5 = md5(inp), md5(out)

    print(f"MD5   input : {in_md5}")
    print(f"MD5   output: {out_md5}")
    processed = in_md5 != out_md5
    print(f"  -> content changed : {'YES (processed)' if processed else 'NO (identical!)'}\n")

    print(f"format input : channels={in_fmt[0]} rate={in_fmt[1]} frames={in_fmt[2]}")
    print(f"format output: channels={out_fmt[0]} rate={out_fmt[1]} frames={out_fmt[2]}")
    fmt_ok = in_fmt == out_fmt
    print(f"  -> format preserved: {'YES' if fmt_ok else 'NO (timestamps at risk!)'}\n")

    in_nf, out_nf = noise_floor(in_mono, in_fmt[1]), noise_floor(out_mono, out_fmt[1])
    if in_nf > 0 and out_nf > 0:
        print(f"noise floor input : {db(in_nf):6.1f} dBFS  (rms {in_nf:.5f})")
        print(f"noise floor output: {db(out_nf):6.1f} dBFS  (rms {out_nf:.5f})")
        drop_db = db(in_nf) - db(out_nf)
        print(f"  -> noise-floor reduction: {drop_db:5.1f} dB "
              f"({'suppressed' if drop_db > 1 else 'little/none'})\n")
    else:
        print("noise floor : n/a (quiet windows are exact digital silence in this "
              "already-normalized file)\n")

    in_rms_db = db(rms(in_mono))
    print(f"overall RMS  input : {in_rms_db:6.1f} dBFS")
    print(f"overall RMS  output: {db(rms(out_mono)):6.1f} dBFS")

    print("\n==============================")
    if in_rms_db < -60:  # essentially silent → nothing to denoise, inconclusive
        print("VERDICT: [INCONCLUSIVE] input is essentially silent (nothing to")
        print("         denoise). Use a recording with audible speech/noise.")
        print("==============================")
        sys.exit(2)
    verdict = processed and fmt_ok
    print(f"VERDICT: DeepFilterNet is {'[ACTIVE] processing audio' if verdict else '[FAIL] NOT confirmed'}")
    print("==============================")
    sys.exit(0 if verdict else 1)


if __name__ == "__main__":
    main()
