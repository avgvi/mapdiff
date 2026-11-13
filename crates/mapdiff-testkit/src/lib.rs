//! Test support shared by the other crates: a throwaway git repository.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

pub struct TempRepo {
    dir: PathBuf,
}

impl TempRepo {
    pub fn new(name: &str) -> Self {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "mapdiff-test-{}-{unique}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir can be created");
        let repo = Self { dir };
        repo.git(&["init", "--quiet", "--initial-branch=main"]);
        repo
    }

    pub fn path(&self) -> &Path {
        &self.dir
    }

    /// Runs git with a fixed identity and no user or system configuration,
    /// so results do not depend on the machine.
    pub fn git(&self, args: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(&self.dir)
            .args([
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
            ])
            .args(["-c", "commit.gpgsign=false", "-c", "core.autocrlf=false"])
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_DATE", "2026-01-01T00:00:00Z")
            .env("GIT_COMMITTER_DATE", "2026-01-01T00:00:00Z")
            .output()
            .expect("git runs");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    pub fn write(&self, path: &str, content: &str) {
        let full = self.dir.join(path);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).expect("directories can be created");
        }
        std::fs::write(full, content).expect("file can be written");
    }

    pub fn read(&self, path: &str) -> String {
        std::fs::read_to_string(self.dir.join(path)).unwrap_or_else(|_| panic!("{path} exists"))
    }

    /// Replaces one exact piece of text in a file. Panics when the text is
    /// not there, so a typo in a test cannot pass silently.
    pub fn replace(&self, path: &str, from: &str, to: &str) {
        let content = self.read(path);
        assert!(content.contains(from), "{path} does not contain:\n{from}");
        self.write(path, &content.replacen(from, to, 1));
    }

    /// Cuts a piece of text out of one file and appends it to another.
    pub fn move_text(&self, from_path: &str, to_path: &str, text: &str) {
        self.replace(from_path, text, "");
        let mut target = self.read(to_path);
        target.push('\n');
        target.push_str(text);
        self.write(to_path, &target);
    }

    pub fn commit(&self, message: &str) {
        self.git(&["add", "--all"]);
        self.git(&["commit", "--quiet", "--allow-empty", "-m", message]);
    }

    /// Added and removed lines summed over `git diff --numstat`.
    pub fn git_totals(&self, sides: &[&str]) -> (u32, u32) {
        let mut args = vec!["diff", "--numstat", "-M"];
        args.extend(sides);
        let (mut added, mut removed) = (0, 0);
        for line in self.git(&args).lines() {
            let mut fields = line.split('\t');
            added += fields
                .next()
                .and_then(|n| n.parse::<u32>().ok())
                .unwrap_or(0);
            removed += fields
                .next()
                .and_then(|n| n.parse::<u32>().ok())
                .unwrap_or(0);
        }
        (added, removed)
    }
}

impl Drop for TempRepo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
