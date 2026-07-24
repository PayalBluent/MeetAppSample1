#!/usr/bin/env python3
"""Measure acoustic echo in a raw STEREO MeetApp recording (L=system, R=mic).

Echo = the mic (R) picking up the far-end audio played through speakers. It shows
up as a delayed, attenuated copy of the system channel (L) inside the mic channel.
We detect it by normalized cross-correlation of L vs R over realistic acoustic
delays (0-250 ms); a strong peak means echo is present and AEC would help.

REQUIRES a recording where BOTH channels are active at the same time — i.e. made
with far-end audio on the SPEAKERS (not headphones) while talking. If the system
channel is silent, echo cannot occur and cannot be measured (the tool says so).

Usage:  python scripts/measure-echo.py <stereo-recording.wav>
Read-only; needs numpy.
"""
import struct, array, sys
import numpy as np

MAX_LAG_MS = 250          # search 0..250 ms of acoustic delay
ACTIVE_RMS = 0.001        # a channel below this is considered silent
STRONG, MILD = 0.30, 0.15 # correlation-coefficient verdict thresholds


def read_stereo(path):
    d = open(path, "rb").read()
    if d[:4] != b"RIFF" or d[8:12] != b"WAVE":
        raise SystemExit("not a WAVE file")
    fmt = pcm = None; pos = 12
    while pos + 8 <= len(d):
        cid = d[pos:pos+4]; size = struct.unpack_from("<I", d, pos+4)[0]; body = d[pos+8:pos+8+size]
        if cid == b"fmt ":
            af, ch, rate, _br, _ba, bits = struct.unpack_from("<HHIIHH", body, 0)
            if af == 0xFFFE and len(body) >= 40: af = struct.unpack_from("<H", body, 24)[0]
            fmt = (af, ch, rate, bits)
        elif cid == b"data": pcm = body
        pos += 8 + size + (size & 1)
    af, ch, rate, bits = fmt
    if ch != 2:
        raise SystemExit(f"need a STEREO recording (L=system, R=mic); this file has {ch} channel(s). "
                         "Finalized/mono recordings can't be used — use a raw stereo capture.")
    if af == 3 and bits == 32: a = np.frombuffer(pcm[:len(pcm)//4*4], "<f4").astype(np.float64)
    elif af == 1 and bits == 16: a = np.frombuffer(pcm[:len(pcm)//2*2], "<i2").astype(np.float64)/32768.0
    elif af == 1 and bits == 32: a = np.frombuffer(pcm[:len(pcm)//4*4], "<i4").astype(np.float64)/2147483648.0
    else: raise SystemExit(f"unsupported: fmt={af} bits={bits}")
    return a[0::2], a[1::2], rate  # L=system, R=mic


def main():
    if len(sys.argv) < 2:
        raise SystemExit(__doc__)
    sysL, micR, rate = read_stereo(sys.argv[1])
    rms = lambda x: float(np.sqrt(np.mean(x*x))) if len(x) else 0.0
    ls, rr = rms(sysL), rms(micR)
    print(f"system(L) rms={ls:.4f}   mic(R) rms={rr:.4f}   rate={rate} Hz   dur={len(micR)/rate:.1f}s")
    if ls < ACTIVE_RMS or rr < ACTIVE_RMS:
        which = "system(far-end)" if ls < ACTIVE_RMS else "mic"
        print(f"\nCANNOT MEASURE: the {which} channel is silent. Echo requires far-end "
              f"audio on speakers AND the mic recording at the same time. Re-record with "
              f"call/far-end audio playing through SPEAKERS while you talk, then re-run.")
        sys.exit(2)

    # Zero-mean; cross-correlate over 0..MAX_LAG_MS via FFT (mic lags system).
    x = sysL - sysL.mean(); y = micR - micR.mean()
    max_lag = int(rate * MAX_LAG_MS / 1000)
    n = 1 << int(np.ceil(np.log2(len(x) + max_lag + 1)))
    corr = np.fft.irfft(np.fft.rfft(y, n) * np.conj(np.fft.rfft(x, n)), n)
    denom = (np.linalg.norm(x) * np.linalg.norm(y)) or 1.0
    lags = corr[:max_lag + 1] / denom               # positive lags: mic after system
    peak = int(np.argmax(np.abs(lags)))
    coeff = float(abs(lags[peak])); lag_ms = peak * 1000.0 / rate

    print(f"peak correlation = {coeff:.3f} at lag {lag_ms:.0f} ms")
    verdict = ("STRONG echo — AEC strongly recommended" if coeff >= STRONG else
               "MILD echo — AEC likely helps" if coeff >= MILD else
               "NEGLIGIBLE echo — AEC not needed (headphones/clean separation)")
    print(f"VERDICT: {verdict}")
    sys.exit(0 if coeff >= MILD else 1)


if __name__ == "__main__":
    main()
