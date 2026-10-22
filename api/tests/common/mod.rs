//! Test helpers shared by the integration tests that drive the Claude runner.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use tempfile::TempDir;
use waypoint_api::claude::ClaudeRunner;

/// A runner pointed at `fake-claude.sh` in one mode, with the directory the fake records
/// its argv, cwd, cwd listing, env and stdin into.
pub struct Fake {
    pub runner: ClaudeRunner,
    #[allow(dead_code)]
    pub record: PathBuf,
    _dir: TempDir,
}

/// Builds a per-test wrapper script that runs the fake in `mode`, so no test mutates the
/// process environment.
pub fn fake(mode: &str, timeout: Duration) -> Fake {
    let dir = tempfile::tempdir().expect("temp dir");
    let record = dir.path().join("record");
    fs::create_dir(&record).expect("record dir");
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/claude/fake-claude.sh");
    let wrapper = dir.path().join("claude");
    fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\nexec '{}' '{mode}' '{}' \"$@\"\n",
            script.display(),
            record.display()
        ),
    )
    .expect("write wrapper");
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).expect("chmod wrapper");
    Fake {
        runner: ClaudeRunner::new(wrapper, timeout),
        record,
        _dir: dir,
    }
}
