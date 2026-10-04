//! Drag-and-drop file transfer to the tablet over the adb link the session
//! already uses. Files land in a user-configurable folder on the tablet's
//! shared storage (default `/sdcard/Download/Penflow`, which shows up under
//! Downloads in the tablet's Files app). Works whenever adb can see the
//! tablet, with or without a display session running.

use std::path::Path;
use std::process::{Command, Output, Stdio};

/// Default destination folder on the tablet.
pub const DEFAULT_DEST: &str = "/sdcard/Download/Penflow";

/// Destinations must sit on shared storage and use a conservative character
/// set. The folder is created with `adb shell mkdir`, which runs through the
/// tablet's shell, so anything that could be read as shell syntax is
/// rejected rather than escaped.
pub fn validate_dest(dir: &str) -> Result<(), String> {
    if !(dir.starts_with("/sdcard/") || dir.starts_with("/storage/")) {
        return Err("tablet folder must start with /sdcard/ or /storage/".into());
    }
    let allowed = |c: char| c.is_ascii_alphanumeric() || " _-./()".contains(c);
    if !dir.chars().all(allowed) {
        return Err(
            "tablet folder may only contain letters, digits, spaces and _ - . / ( )".into(),
        );
    }
    if dir.split('/').any(|part| part == "..") {
        return Err("tablet folder may not contain '..'".into());
    }
    Ok(())
}

/// Push `paths` (files or folders, recursively) into `dest` on the tablet.
/// Blocks until adb finishes; returns adb's one-line transfer summary.
pub fn send(adb: &str, dest: &str, paths: &[String]) -> Result<String, String> {
    validate_dest(dest)?;
    if paths.is_empty() {
        return Err("nothing to send".into());
    }
    if let Some(missing) = paths.iter().find(|p| !Path::new(p.as_str()).exists()) {
        return Err(format!("{missing} no longer exists"));
    }

    let dest = dest.trim_end_matches('/');
    run(adb, &["shell", &format!("mkdir -p '{dest}'")])?;

    let mut args: Vec<&str> = vec!["push"];
    args.extend(paths.iter().map(String::as_str));
    let target = format!("{dest}/");
    args.push(&target);
    let out = run(adb, &args)?;

    // adb ends a push with e.g. "3 files pushed, 0 skipped. 41.2 MB/s
    // (12345678 bytes in 0.286s)"; surface that line to the user.
    let stdout = String::from_utf8_lossy(&out.stdout);
    Ok(stdout
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("done")
        .to_string())
}

fn run(adb: &str, args: &[&str]) -> Result<Output, String> {
    let mut cmd = Command::new(adb);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        // No console flash: the release GUI has no console of its own.
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let out = cmd
        .output()
        .map_err(|e| format!("could not run adb ({adb}): {e}"))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let stdout = String::from_utf8_lossy(&out.stdout);
        let msg = stderr
            .lines()
            .chain(stdout.lines())
            .map(str::trim)
            .find(|l| !l.is_empty())
            .unwrap_or("unknown error");
        return Err(format!("adb {}: {msg}", args[0]));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_dest_is_valid() {
        validate_dest(DEFAULT_DEST).expect("default destination should be valid");
    }

    #[test]
    fn dest_must_be_on_shared_storage() {
        assert!(validate_dest("/data/local/tmp").is_err());
        assert!(validate_dest("sdcard/Download").is_err());
        assert!(validate_dest("/storage/emulated/0/Pictures/Refs").is_ok());
    }

    #[test]
    fn dest_rejects_shell_syntax_and_traversal() {
        assert!(validate_dest("/sdcard/Download/a'b").is_err());
        assert!(validate_dest("/sdcard/Download/$(reboot)").is_err());
        assert!(validate_dest("/sdcard/Download/a;b").is_err());
        assert!(validate_dest("/sdcard/../data").is_err());
        assert!(validate_dest("/sdcard/Download/My Refs (2026)").is_ok());
    }

    #[test]
    fn send_rejects_missing_files_before_touching_adb() {
        let err = send(
            "adb-that-does-not-exist",
            DEFAULT_DEST,
            &["Z:/definitely/not/here.png".to_string()],
        )
        .expect_err("missing file should fail");
        assert!(err.contains("no longer exists"), "got: {err}");
    }
}
