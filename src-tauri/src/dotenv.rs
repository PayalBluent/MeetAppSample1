//! Minimal `.env` loader (no external crate).
//!
//! At startup we look for a `.env` file — first walking up from the current
//! working directory (during `tauri dev` that's `src-tauri/`, so the project-root
//! `.env` is found), then next to the executable — and copy its `KEY=VALUE`
//! entries into the process environment. Real environment variables always win:
//! a key already set in the environment is never overwritten.
//!
//! Supported syntax: `KEY=VALUE` lines, `#` comments, blank lines, an optional
//! `export ` prefix, and single/double quotes around the value.

use std::path::{Path, PathBuf};

/// Load the nearest `.env` file into the process environment, if one exists.
pub fn load() {
    if load_from_ancestors(std::env::current_dir().ok()) {
        return;
    }
    if let Ok(exe) = std::env::current_exe() {
        load_from_ancestors(exe.parent().map(Path::to_path_buf));
    }
}

/// Walk `start` and its ancestors looking for a `.env`; apply the first found.
fn load_from_ancestors(start: Option<PathBuf>) -> bool {
    let mut dir = start;
    while let Some(d) = dir {
        let candidate = d.join(".env");
        if candidate.is_file() {
            apply(&candidate);
            return true;
        }
        dir = d.parent().map(Path::to_path_buf);
    }
    false
}

fn apply(path: &Path) {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("could not read {path:?}: {e}");
            return;
        }
    };
    let mut loaded = 0usize;
    for raw in content.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() {
            continue;
        }
        let value = unquote(value.trim());
        // Real environment variables take precedence over the file.
        if std::env::var_os(key).is_none() {
            std::env::set_var(key, value);
            loaded += 1;
        }
    }
    if loaded > 0 {
        tracing::info!("loaded {loaded} variable(s) from {path:?}");
    }
}

/// Strip a single matching pair of surrounding quotes, if present.
fn unquote(v: &str) -> &str {
    let bytes = v.as_bytes();
    if bytes.len() >= 2 {
        let first = bytes[0];
        let last = bytes[bytes.len() - 1];
        if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
            return &v[1..v.len() - 1];
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_lines_comments_and_quotes() {
        let dir = std::env::temp_dir().join("meetapp-dotenv-1");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(".env");
        std::fs::write(
            &path,
            "# a comment\n\
             DOTENV_T_PLAIN=hello\n\
             export DOTENV_T_EXPORT=world\n\
             DOTENV_T_DQUOTE=\"quoted value\"\n\
             DOTENV_T_SQUOTE='single'\n\
             \n\
             DOTENV_T_EMPTY=\n",
        )
        .unwrap();
        for k in [
            "DOTENV_T_PLAIN",
            "DOTENV_T_EXPORT",
            "DOTENV_T_DQUOTE",
            "DOTENV_T_SQUOTE",
            "DOTENV_T_EMPTY",
        ] {
            std::env::remove_var(k);
        }

        apply(&path);

        assert_eq!(std::env::var("DOTENV_T_PLAIN").unwrap(), "hello");
        assert_eq!(std::env::var("DOTENV_T_EXPORT").unwrap(), "world");
        assert_eq!(std::env::var("DOTENV_T_DQUOTE").unwrap(), "quoted value");
        assert_eq!(std::env::var("DOTENV_T_SQUOTE").unwrap(), "single");
        assert_eq!(std::env::var("DOTENV_T_EMPTY").unwrap(), "");
    }

    #[test]
    fn real_environment_wins() {
        let dir = std::env::temp_dir().join("meetapp-dotenv-2");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(".env");
        std::fs::write(&path, "DOTENV_T_PREC=from_file\n").unwrap();

        std::env::set_var("DOTENV_T_PREC", "from_env");
        apply(&path);
        assert_eq!(std::env::var("DOTENV_T_PREC").unwrap(), "from_env");
    }
}
