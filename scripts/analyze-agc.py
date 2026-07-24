#!/usr/bin/env python3
"""Read-only check of the adaptive AGC decision (mirrors normalize_wav_file).

Prints, per WAV: noise floor (10th-pct 20ms-window RMS), speech level (90th pct),
the base target-loudness gain, and the noise-aware capped gain — so you can see the
AGC adapt to each recording instead of applying one fixed boost. Does NOT modify
any file. Handles PCM16/float32/extensible WAVs.

Usage:  python scripts/analyze-agc.py <file1.wav> [file2.wav ...]
"""
import array, math, struct, sys

TARGET_RMS, MAX_GAIN, NOISE_CEILING, SEPARATION, WINDOW_MS = 0.16, 12.0, 0.01, 3.0, 20


def read_mono(path):
    with open(path, "rb") as f:
        d = f.read()
    if d[:4] != b"RIFF" or d[8:12] != b"WAVE":
        raise SystemExit(f"not WAVE: {path}")
    fmt = pcm = None
    pos = 12
    while pos + 8 <= len(d):
        cid, size = d[pos:pos + 4], struct.unpack_from("<I", d, pos + 4)[0]
        body = d[pos + 8:pos + 8 + size]
        if cid == b"fmt ":
            af, ch, rate, _br, _ba, bits = struct.unpack_from("<HHIIHH", body, 0)
            if af == 0xFFFE and len(body) >= 40:
                af = struct.unpack_from("<H", body, 24)[0]
            fmt = (af, ch, rate, bits)
        elif cid == b"data":
            pcm = body
        pos += 8 + size + (size & 1)
    af, ch, rate, bits = fmt
    if af == 3 and bits == 32:
        a = array.array("f"); a.frombytes(pcm[:len(pcm)//4*4]); norm = 1.0
    elif af == 1 and bits == 16:
        a = array.array("h"); a.frombytes(pcm[:len(pcm)//2*2]); norm = 32768.0
    elif af == 1 and bits == 32:
        a = array.array("i"); a.frombytes(pcm[:len(pcm)//4*4]); norm = 2147483648.0
    else:
        raise SystemExit(f"unsupported: fmt={af} bits={bits}")
    ch = max(1, ch)
    frames = len(a) // ch
    mono = [sum(a[i:i+ch]) / (ch*norm) for i in range(0, frames*ch, ch)] if ch > 1 \
        else [s/norm for s in a]
    return mono, rate


def rms(xs):
    return math.sqrt(sum(x*x for x in xs)/len(xs)) if xs else 0.0


def db(x):
    return 20*math.log10(x) if x > 1e-12 else -120.0


def agc(path):
    mono, rate = read_mono(path)
    overall = rms(mono)
    if max((abs(s) for s in mono), default=0) < 1e-5 or overall < 1e-6:
        return f"{path}: SILENT (no-op)"
    win = max(1, rate*WINDOW_MS//1000)
    w = sorted(rms(mono[i:i+win]) for i in range(0, len(mono), win) if mono[i:i+win])
    nf = w[max(0, int(len(w)*0.10)-1)]
    sp = w[max(0, int(len(w)*0.90)-1)]
    base = min(max(TARGET_RMS/overall, 1.0), MAX_GAIN)
    gain = base
    capped = nf > 1e-6 and sp > SEPARATION*nf
    if capped:
        gain = min(gain, max(NOISE_CEILING/nf, 1.0))
    return (f"noise={db(nf):6.1f}dB speech={db(sp):6.1f}dB base=x{base:4.1f} "
            f"-> gain=x{gain:4.1f} {'(noise-capped)' if capped and gain < base else '(full boost)'}")


for p in sys.argv[1:]:
    try:
        print(f"{p.split(chr(92))[-1].split('/')[-1]:55} {agc(p)}")
    except SystemExit as e:
        print(f"{p}: {e}")
