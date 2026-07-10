import { useEffect, useRef, useState } from "react";

export interface Playback {
  currentMs: number;
  playing: boolean;
  rate: number;
  durationMs: number;
  toggle: () => void;
  play: () => void;
  pause: () => void;
  seek: (ms: number) => void;
  skip: (deltaMs: number) => void;
  cycleRate: () => void;
}

const RATES = [1, 1.25, 1.5, 2, 0.75];

/**
 * Playback controller. When `mediaEl` is provided it drives / follows a real
 * media element; otherwise it runs a synthetic clock so the transport and
 * transcript-sync work against the in-memory mock (no real audio file).
 */
export function usePlayback(
  durationMs: number,
  mediaEl?: HTMLMediaElement | null,
): Playback {
  const [currentMs, setCurrentMs] = useState(0);
  const [playing, setPlaying] = useState(false);
  const [rate, setRate] = useState(1);
  const raf = useRef<number | null>(null);
  const last = useRef<number>(0);

  const synthetic = !mediaEl;

  // Synthetic clock (no real media element).
  useEffect(() => {
    if (!synthetic || !playing) return;
    last.current = performance.now();
    const tick = (now: number) => {
      const dt = (now - last.current) * rate;
      last.current = now;
      setCurrentMs((ms) => {
        const next = ms + dt;
        if (next >= durationMs) {
          setPlaying(false);
          return durationMs;
        }
        return next;
      });
      raf.current = requestAnimationFrame(tick);
    };
    raf.current = requestAnimationFrame(tick);
    return () => {
      if (raf.current) cancelAnimationFrame(raf.current);
    };
  }, [synthetic, playing, rate, durationMs]);

  // Real media element: reflect props → element and element → state.
  useEffect(() => {
    if (!mediaEl) return;
    const onTime = () => setCurrentMs(mediaEl.currentTime * 1000);
    const onEnd = () => setPlaying(false);
    mediaEl.addEventListener("timeupdate", onTime);
    mediaEl.addEventListener("ended", onEnd);
    return () => {
      mediaEl.removeEventListener("timeupdate", onTime);
      mediaEl.removeEventListener("ended", onEnd);
    };
  }, [mediaEl]);

  useEffect(() => {
    if (!mediaEl) return;
    mediaEl.playbackRate = rate;
  }, [mediaEl, rate]);

  useEffect(() => {
    if (!mediaEl) return;
    if (playing) void mediaEl.play().catch(() => setPlaying(false));
    else mediaEl.pause();
  }, [mediaEl, playing]);

  const seek = (ms: number) => {
    const clamped = Math.max(0, Math.min(ms, durationMs));
    setCurrentMs(clamped);
    if (mediaEl) mediaEl.currentTime = clamped / 1000;
  };

  return {
    currentMs,
    playing,
    rate,
    durationMs,
    toggle: () => setPlaying((p) => !p),
    play: () => setPlaying(true),
    pause: () => setPlaying(false),
    seek,
    skip: (delta) => seek(currentMs + delta),
    cycleRate: () => {
      const idx = RATES.indexOf(rate);
      setRate(RATES[(idx + 1) % RATES.length]!);
    },
  };
}
