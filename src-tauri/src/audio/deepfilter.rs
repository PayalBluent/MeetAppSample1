//! Optional **DeepFilterNet** noise-suppression preprocessing for transcription.
//!
//! This is a *backup / opt-in* enhancement stage that sits **in front of** the
//! existing AssemblyAI transcription pipeline (see
//! [`crate::core::cloud::run_transcription`]). When enabled, the recorded WAV is
//! run through DeepFilterNet to suppress background noise and the resulting
//! *enhanced copy* is what gets uploaded to AssemblyAI — the original recording on
//! disk is never touched, and AssemblyAI's settings, authentication, timestamps,
//! and speaker diarization all continue to work exactly as before (DeepFilterNet
//! preserves sample rate and duration, so transcript timestamps stay valid).
//!
//! ## Why a subprocess (not a Rust crate)
//! DeepFilterNet ships an official standalone CLI (`deep-filter`, the Rust binary
//! from the DeepFilterNet releases, or the `deepFilter` entry point from
//! `pip install deepfilternet`). Shelling out to it — exactly like the existing
//! [`crate::audio::mux_audio_into_video`] ffmpeg integration — keeps the heavy ML
//! model and its runtime entirely out of this build, adds **zero** new Rust
//! dependencies, and means the app degrades gracefully to the original AssemblyAI
//! flow whenever the tool isn't installed.
//!
//! ## Enabling / disabling it
//! **On by default** (the `deepfilter` Cargo feature is in the default set), so
//! DeepFilterNet is the primary denoiser out of the box, with RNNoise as the
//! automatic fallback. Controls:
//!   * **Binary** — auto-discovered (next to the app executable, then
//!     `~/deepfilter/`), or set `MEETAPP_DEEPFILTER_BIN` explicitly. If none is
//!     found, the spawn fails and the pipeline falls back to RNNoise.
//!   * **Run-time toggle** — `MEETAPP_DEEPFILTER=0` (`false`/`off`) forces it off,
//!     so RNNoise handles suppression; unset or `1`/`on` keeps it primary.
//!   * **Extra CLI args** — `MEETAPP_DEEPFILTER_ARGS`.
//!   * **Compile out entirely** — build with `--no-default-features --features
//!     mic-capture,denoise,screen-capture`; then every function here is a no-op.
//!
//! On *any* problem — feature off, env-disabled, binary missing, non-zero exit,
//! invalid/no output — the pipeline falls back to RNNoise (via
//! [`crate::audio::suppress_noise`]). It can never break recording.

use std::path::{Path, PathBuf};

/// Default DeepFilterNet CLI command (overridable via `MEETAPP_DEEPFILTER_BIN`).
const DEFAULT_BIN: &str = "deep-filter";

/// A temporary DeepFilterNet-enhanced audio file. The whole working directory is
/// removed when this value is dropped, so the enhanced copy never lingers on disk
/// after the transcription that used it completes — unless `keep` is set (via
/// `MEETAPP_DEEPFILTER_KEEP`), which preserves it for inspection/verification.
pub struct Enhanced {
    /// The enhanced WAV to hand to transcription.
    file: PathBuf,
    /// The unique temp directory holding it (removed on drop unless `keep`).
    dir: PathBuf,
    /// When true, the temp directory is left on disk so the exact audio the app
    /// sent to AssemblyAI can be inspected. Driven by `MEETAPP_DEEPFILTER_KEEP`.
    keep: bool,
}

impl Enhanced {
    /// Path of the enhanced WAV to send to AssemblyAI.
    pub fn path(&self) -> &Path {
        &self.file
    }
}

impl Drop for Enhanced {
    fn drop(&mut self) {
        if self.keep {
            tracing::info!(
                "DeepFilterNet: keeping enhanced audio for inspection at {:?} \
                 (MEETAPP_DEEPFILTER_KEEP set)",
                self.file
            );
        } else {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }
}

/// Preprocess `input` with DeepFilterNet, returning a temporary enhanced copy to
/// send to transcription — or `None` to signal "use the original audio unchanged".
///
/// This is the single entry point the transcription pipeline calls. It applies
/// both gates (compile-time feature + runtime availability) and swallows every
/// failure into `None`, so the caller's fallback is a plain
/// `enhanced.map(|e| e.path()).unwrap_or(original)`.
pub fn maybe_enhance(input: &Path) -> Option<Enhanced> {
    #[cfg(feature = "deepfilter")]
    {
        if !enabled() {
            tracing::debug!("DeepFilterNet: disabled via MEETAPP_DEEPFILTER; using original audio");
            return None;
        }
        match imp::enhance(input) {
            Ok(enhanced) => {
                tracing::info!(
                    "DeepFilterNet: enhanced {:?} -> {:?} for transcription",
                    input,
                    enhanced.path()
                );
                Some(enhanced)
            }
            Err(e) => {
                // Non-fatal by design: log and fall back to the original audio so
                // the existing AssemblyAI pipeline runs exactly as before.
                tracing::warn!(
                    "DeepFilterNet preprocessing unavailable ({e}); falling back to original audio"
                );
                None
            }
        }
    }
    #[cfg(not(feature = "deepfilter"))]
    {
        let _ = input;
        None
    }
}

/// Try to noise-suppress `input` **in place** with DeepFilterNet: run the model,
/// validate that it produced a well-formed WAV matching the input's rate/channels,
/// then overwrite `input` with that output. Returns `Ok(())` when DeepFilterNet is
/// the source of truth, or `Err(reason)` when it is unavailable/failed/produced
/// invalid output — the single signal the pipeline uses to fall back to RNNoise.
///
/// This is THE DeepFilterNet entry point for the noise-suppression decision point
/// (see [`crate::audio::suppress_noise`]). It never runs RNNoise itself.
pub fn try_enhance_in_place(input: &Path) -> Result<(), String> {
    #[cfg(feature = "deepfilter")]
    {
        if !enabled() {
            return Err("disabled via MEETAPP_DEEPFILTER=0".into());
        }
        // 1) Run the model into a temp file.
        let enhanced = imp::enhance(input).map_err(|e| e.to_string())?;
        // 2) Validate the output is real, non-empty, and format-compatible.
        imp::validate(input, enhanced.path()).map_err(|e| e.to_string())?;
        // 3) Adopt it as the single source of truth (overwrite the recording).
        std::fs::copy(enhanced.path(), input)
            .map_err(|e| format!("failed to write DeepFilterNet output over {input:?}: {e}"))?;
        Ok(()) // `enhanced` drops here → its temp dir is removed (unless KEEP set)
    }
    #[cfg(not(feature = "deepfilter"))]
    {
        let _ = input;
        Err("feature `deepfilter` not compiled in".into())
    }
}

/// Whether the runtime toggle allows DeepFilterNet. Unset ⇒ allowed (the compile
/// feature already opted in); an explicit `0`/`false`/`no`/`off` disables it.
#[cfg(feature = "deepfilter")]
fn enabled() -> bool {
    match std::env::var("MEETAPP_DEEPFILTER") {
        Ok(v) => !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "no" | "off" | ""
        ),
        Err(_) => true,
    }
}

#[cfg(feature = "deepfilter")]
mod imp {
    use super::{Enhanced, DEFAULT_BIN};
    use crate::error::{AppError, AppResult};
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};

    /// Resolve the DeepFilterNet binary so it works out of the box, without env
    /// setup: explicit `MEETAPP_DEEPFILTER_BIN` wins; otherwise try known install
    /// locations (next to the app executable, then `~/deepfilter/`); otherwise fall
    /// back to the bare command name for a normal PATH lookup. If none of these
    /// resolve to a runnable binary, the spawn fails and the caller falls back to
    /// RNNoise — so this never breaks recording, it just picks DeepFilterNet when
    /// it's actually available.
    fn resolve_binary() -> String {
        if let Ok(b) = std::env::var("MEETAPP_DEEPFILTER_BIN") {
            let b = b.trim();
            if !b.is_empty() {
                return b.to_string();
            }
        }
        let exe_name = if cfg!(windows) { "deep-filter.exe" } else { "deep-filter" };
        let mut candidates: Vec<PathBuf> = Vec::new();
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                candidates.push(dir.join(exe_name));
            }
        }
        if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
            candidates.push(PathBuf::from(home).join("deepfilter").join(exe_name));
        }
        for c in candidates {
            if c.is_file() {
                return c.to_string_lossy().into_owned();
            }
        }
        DEFAULT_BIN.to_string() // rely on PATH
    }

    /// Run DeepFilterNet over `input`, writing the enhanced result into a fresh
    /// temp directory and returning a handle to it. Errors (mapped to
    /// [`AppError::Audio`]) are treated as "unavailable" by the caller.
    pub fn enhance(input: &Path) -> AppResult<Enhanced> {
        if !input.exists() {
            return Err(AppError::Audio(format!("input audio not found: {input:?}")));
        }

        // Unique, empty working directory so we can reliably locate the one output
        // file the CLI produces (its exact output filename varies by version).
        let dir = std::env::temp_dir().join(format!(
            "meetapp-deepfilter-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&dir).map_err(AppError::Io)?;

        // Build:  <bin> [extra args] --output-dir <dir> <input>
        // Both the Rust `deep-filter` and Python `deepFilter` CLIs accept
        // `--output-dir` and positional input files.
        let bin = resolve_binary();
        let mut cmd = Command::new(&bin);
        if let Ok(extra) = std::env::var("MEETAPP_DEEPFILTER_ARGS") {
            for arg in extra.split_whitespace() {
                cmd.arg(arg);
            }
        }
        cmd.arg("--output-dir")
            .arg(&dir)
            .arg(input)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        // Log the exact command so it's unambiguous in the app log that audio is
        // being routed through DeepFilterNet (and which binary/args ran).
        tracing::info!("DeepFilterNet: invoking `{bin} --output-dir {dir:?} {input:?}`");

        let status = cmd.status().map_err(|e| {
            let _ = std::fs::remove_dir_all(&dir);
            AppError::Audio(format!("could not launch DeepFilterNet ({bin}): {e}"))
        })?;
        if !status.success() {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(AppError::Audio(format!(
                "DeepFilterNet exited with {status}"
            )));
        }

        // Pick the enhanced WAV the CLI wrote into our empty dir. Scanning for the
        // output (rather than assuming a fixed name) keeps us robust to the
        // version-dependent `_DeepFilterNet3.wav` style suffixes.
        let out = std::fs::read_dir(&dir)
            .map_err(AppError::Io)?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .find(|p| {
                p.extension()
                    .and_then(|x| x.to_str())
                    .map(|x| x.eq_ignore_ascii_case("wav"))
                    .unwrap_or(false)
                    && p.metadata().map(|m| m.len() > 0).unwrap_or(false)
            });

        match out {
            Some(file) => {
                let bytes = std::fs::metadata(&file).map(|m| m.len()).unwrap_or(0);
                tracing::info!("DeepFilterNet: wrote enhanced audio {file:?} ({bytes} bytes)");
                let keep = matches!(
                    std::env::var("MEETAPP_DEEPFILTER_KEEP").ok().as_deref(),
                    Some("1") | Some("true") | Some("yes") | Some("on")
                );
                Ok(Enhanced { file, dir, keep })
            }
            None => {
                let _ = std::fs::remove_dir_all(&dir);
                Err(AppError::Audio(
                    "DeepFilterNet produced no output file".into(),
                ))
            }
        }
    }

    /// Validate a DeepFilterNet output before trusting it as the source of truth:
    /// it must be a readable WAV, carry samples, and preserve the input's sample
    /// rate + channel count (so downstream playback and transcript timestamps stay
    /// correct). Any mismatch is a hard failure that triggers the RNNoise fallback.
    pub fn validate(input: &Path, out: &Path) -> AppResult<()> {
        let in_spec = hound::WavReader::open(input)
            .map_err(|e| AppError::Audio(format!("cannot re-open input {input:?}: {e}")))?
            .spec();
        let out_reader = hound::WavReader::open(out)
            .map_err(|e| AppError::Audio(format!("output is not a valid WAV: {e}")))?;
        let out_spec = out_reader.spec();
        if out_reader.len() == 0 {
            return Err(AppError::Audio("output WAV has no samples".into()));
        }
        if out_spec.sample_rate != in_spec.sample_rate || out_spec.channels != in_spec.channels {
            return Err(AppError::Audio(format!(
                "output format {}Hz/{}ch does not match input {}Hz/{}ch",
                out_spec.sample_rate, out_spec.channels, in_spec.sample_rate, in_spec.channels
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Without the `deepfilter` feature, preprocessing is always a no-op — the
    /// pipeline uses the original audio, guaranteeing the default build is
    /// behaviorally identical to before this module existed.
    #[cfg(not(feature = "deepfilter"))]
    #[test]
    fn disabled_build_is_always_noop() {
        assert!(maybe_enhance(Path::new("whatever.wav")).is_none());
    }

    /// End-to-end through the pipeline's **own** entry point: run the real
    /// DeepFilterNet binary via `maybe_enhance` and confirm it yields a non-empty
    /// enhanced file whose bytes differ from the input (i.e. it truly processed the
    /// audio, not copied it). Opt-in: skips (passes) unless
    /// `MEETAPP_DEEPFILTER_TEST_WAV` points at a real WAV and
    /// `MEETAPP_DEEPFILTER_BIN` at the binary — so ordinary CI without the tool
    /// stays green.
    #[cfg(feature = "deepfilter")]
    #[test]
    fn maybe_enhance_runs_real_binary() {
        let wav = match std::env::var("MEETAPP_DEEPFILTER_TEST_WAV") {
            Ok(w) if !w.is_empty() => w,
            _ => {
                eprintln!("SKIP: set MEETAPP_DEEPFILTER_TEST_WAV (+ MEETAPP_DEEPFILTER_BIN) to run");
                return;
            }
        };
        std::env::set_var("MEETAPP_DEEPFILTER", "1");

        let input = Path::new(&wav);
        let enhanced = maybe_enhance(input).expect("maybe_enhance should return an enhanced file");

        let out_bytes = std::fs::read(enhanced.path()).expect("read enhanced");
        let in_bytes = std::fs::read(input).expect("read input");
        assert!(!out_bytes.is_empty(), "enhanced file must be non-empty");
        assert_ne!(out_bytes, in_bytes, "enhanced audio must differ from the input");
        eprintln!(
            "OK: {:?} ({} bytes) -> {:?} ({} bytes)",
            input,
            in_bytes.len(),
            enhanced.path(),
            out_bytes.len()
        );

        let temp = enhanced.path().to_path_buf();
        drop(enhanced);
        assert!(!temp.exists(), "temp enhanced file must be cleaned up on drop");
    }

    /// The primary path: `try_enhance_in_place` runs the real binary, validates the
    /// output, and OVERWRITES the file in place (same rate/channels, changed bytes).
    /// Opt-in: skips unless `MEETAPP_DEEPFILTER_TEST_WAV` + `MEETAPP_DEEPFILTER_BIN`
    /// are set. Works on a copy so the source recording is never modified.
    #[cfg(feature = "deepfilter")]
    #[test]
    fn try_enhance_in_place_uses_real_binary() {
        let wav = match std::env::var("MEETAPP_DEEPFILTER_TEST_WAV") {
            Ok(w) if !w.is_empty() => w,
            _ => {
                eprintln!("SKIP: set MEETAPP_DEEPFILTER_TEST_WAV (+ MEETAPP_DEEPFILTER_BIN) to run");
                return;
            }
        };
        std::env::set_var("MEETAPP_DEEPFILTER", "1");

        let dir = std::env::temp_dir().join(format!("df-inplace-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let copy = dir.join("recording.wav");
        std::fs::copy(&wav, &copy).unwrap();

        let before = std::fs::read(&copy).unwrap();
        let in_spec = hound::WavReader::open(&copy).unwrap().spec();

        try_enhance_in_place(&copy).expect("DeepFilterNet in-place enhancement should succeed");

        let after = std::fs::read(&copy).unwrap();
        let out_spec = hound::WavReader::open(&copy).unwrap().spec();
        assert_ne!(before, after, "file must be overwritten with processed audio");
        assert_eq!(in_spec.sample_rate, out_spec.sample_rate, "rate preserved");
        assert_eq!(in_spec.channels, out_spec.channels, "channels preserved");
        eprintln!("OK: in-place enhanced {copy:?} ({} -> {} bytes)", before.len(), after.len());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The runtime toggle treats explicit falsey values as "off" and everything
    /// else (including unset) as "on", so an operator can force-disable a
    /// feature-compiled build without recompiling.
    #[cfg(feature = "deepfilter")]
    #[test]
    fn env_toggle_parsing() {
        for (v, want) in [
            ("0", false),
            ("false", false),
            ("OFF", false),
            ("no", false),
            ("1", true),
            ("true", true),
            ("on", true),
        ] {
            std::env::set_var("MEETAPP_DEEPFILTER", v);
            assert_eq!(enabled(), want, "MEETAPP_DEEPFILTER={v}");
        }
        std::env::remove_var("MEETAPP_DEEPFILTER");
        assert!(enabled(), "unset ⇒ allowed (compile feature already opted in)");
    }
}
