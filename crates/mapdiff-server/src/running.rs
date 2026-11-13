//! A note of every mapdiff server that is running, and of every repository
//! one was started in. Both are kept in mapdiff's cache folder, so that
//! `mapdiff list`, `mapdiff stop` and `mapdiff clean` can find them from
//! any terminal.

use std::hash::{Hash, Hasher};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// One running server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Running {
    pub pid: u32,
    pub port: u16,
    /// The root folder of the repository it reads.
    pub repo: String,
    /// The change it last drew on the map, or `the map`.
    pub showing: String,
    /// When it started, in seconds since 1970.
    pub started: u64,
}

impl Running {
    pub fn new(port: u16, repo: &Path, showing: &str) -> Self {
        Self {
            pid: std::process::id(),
            port,
            repo: repo.to_string_lossy().into_owned(),
            showing: showing.to_owned(),
            started: now(),
        }
    }

    pub fn address(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// How long it has been running, in seconds.
    pub fn up(&self) -> u64 {
        now().saturating_sub(self.started)
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

fn notes(home: &Path) -> PathBuf {
    home.join("running")
}

fn note_of(home: &Path, pid: u32) -> PathBuf {
    notes(home).join(format!("{pid}.json"))
}

/// Writes or rewrites the note of a server. Failing is fine: the server
/// runs all the same, and is only missing from `mapdiff list`.
pub(crate) fn announce(home: &Path, running: &Running) {
    let _ = std::fs::create_dir_all(notes(home));
    if let Ok(text) = serde_json::to_string(running) {
        let _ = std::fs::write(note_of(home, running.pid), text);
    }
}

pub(crate) fn withdraw(home: &Path, pid: u32) {
    let _ = std::fs::remove_file(note_of(home, pid));
}

/// Every server that is running, oldest first. A note left behind by one
/// that is gone is removed.
pub fn list() -> Vec<Running> {
    list_in(&mapdiff_graph::cache_home())
}

pub(crate) fn list_in(home: &Path) -> Vec<Running> {
    let Ok(entries) = std::fs::read_dir(notes(home)) else {
        return Vec::new();
    };
    let mut running = Vec::new();
    for entry in entries.flatten() {
        let noted = std::fs::read(entry.path())
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Running>(&bytes).ok());
        match noted {
            Some(noted) if answers(&noted) => running.push(noted),
            _ => {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    running.sort_by_key(|each| (each.started, each.pid));
    running
}

/// Whether the server of a note is still there: something answers on its
/// port and says it is that process. A process number alone is not enough,
/// since the system gives the numbers of ended processes to new ones.
fn answers(running: &Running) -> bool {
    let address = SocketAddr::from(([127, 0, 0, 1], running.port));
    let Ok(mut stream) = TcpStream::connect_timeout(&address, Duration::from_millis(400)) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let request = format!(
        "GET /api/running HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n",
        running.port
    );
    if stream.write_all(request.as_bytes()).is_err() {
        return false;
    }
    let mut reply = String::new();
    let _ = stream.read_to_string(&mut reply);
    reply
        .split_once("\r\n\r\n")
        .and_then(|(_, body)| serde_json::from_str::<serde_json::Value>(body).ok())
        .is_some_and(|body| body["pid"] == running.pid)
}

/// Stops a server the way Ctrl-C does, and waits until it is gone.
pub fn stop(running: &Running) -> Result<(), String> {
    if !cfg!(unix) {
        return Err(String::from(
            "stopping a server is not supported on this system",
        ));
    }
    let sent = std::process::Command::new("kill")
        .args(["-INT", &running.pid.to_string()])
        .status()
        .map_err(|err| format!("cannot run kill: {err}"))?;
    if !sent.success() {
        return Err(format!("cannot signal process {}", running.pid));
    }
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(10) {
        if !answers(running) {
            withdraw(&mapdiff_graph::cache_home(), running.pid);
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err(format!("process {} did not stop", running.pid))
}

fn repositories(home: &Path) -> PathBuf {
    home.join("repositories")
}

/// Notes that a server was started in a repository, so that `mapdiff
/// clean` can find what was left in it.
pub(crate) fn remember_repository(home: &Path, root: &Path) {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    root.hash(&mut hasher);
    let _ = std::fs::create_dir_all(repositories(home));
    let _ = std::fs::write(
        repositories(home).join(format!("{:016x}", hasher.finish())),
        root.to_string_lossy().as_bytes(),
    );
}

fn known_repositories(home: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(repositories(home)) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| std::fs::read_to_string(entry.path()).ok())
        .map(PathBuf::from)
        .collect()
}

/// What `clean` removed from one repository.
#[derive(Debug, PartialEq, Eq)]
pub struct CleanedRepository {
    pub root: PathBuf,
    /// Review marks were there and are gone.
    pub marks: bool,
    /// How many fetched pull request refs were removed.
    pub refs: usize,
}

/// What `clean` did.
#[derive(Debug, Default)]
pub struct Cleaned {
    pub stopped: Vec<Running>,
    /// Servers that would not stop, with why.
    pub still_running: Vec<(Running, String)>,
    pub repositories: Vec<CleanedRepository>,
    /// How many temporary copies of files were removed.
    pub temporary: usize,
    /// The cache folder, when there was one to remove.
    pub cache: Option<PathBuf>,
}

/// Stops every server and removes everything mapdiff has written: review
/// marks and fetched pull requests in every repository a server was
/// started in (and in `here`, when that is inside one), temporary copies
/// of files, and the cache folder with its indexes.
pub fn clean(here: Option<&Path>) -> Cleaned {
    clean_in(&mapdiff_graph::cache_home(), here)
}

pub(crate) fn clean_in(home: &Path, here: Option<&Path>) -> Cleaned {
    let mut cleaned = Cleaned::default();
    for running in list_in(home) {
        match stop(&running) {
            Ok(()) => cleaned.stopped.push(running),
            Err(why) => cleaned.still_running.push((running, why)),
        }
    }

    let mut roots = known_repositories(home);
    if let Some(here) = here
        && let Ok(repo) = mapdiff_gitdiff::open(here)
    {
        roots.push(repo.root().to_path_buf());
    }
    roots.sort();
    roots.dedup();
    for root in roots {
        // A repository that was deleted since has nothing left to remove.
        let Ok(repo) = mapdiff_gitdiff::open(&root) else {
            continue;
        };
        let marks = repo
            .git_dir()
            .map(|directory| directory.join("mapdiff"))
            .is_ok_and(|directory| {
                directory.exists() && std::fs::remove_dir_all(directory).is_ok()
            });
        let refs = repo.remove_own_refs().unwrap_or(0);
        if marks || refs > 0 {
            cleaned
                .repositories
                .push(CleanedRepository { root, marks, refs });
        }
    }

    cleaned.temporary = mapdiff_graph::remove_temporary_copies();
    if home.exists() && std::fs::remove_dir_all(home).is_ok() {
        cleaned.cache = Some(home.to_path_buf());
    }
    cleaned
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home(name: &str) -> PathBuf {
        let home =
            std::env::temp_dir().join(format!("mapdiff-test-home-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        home
    }

    #[test]
    fn a_note_of_a_server_that_is_gone_is_dropped() {
        let home = home("gone");
        // Nothing listens on port 1.
        let gone = Running {
            pid: 1,
            port: 1,
            repo: String::from("/nowhere"),
            showing: String::from("the map"),
            started: 0,
        };
        announce(&home, &gone);
        assert!(note_of(&home, 1).is_file());
        assert_eq!(list_in(&home), Vec::new());
        assert!(!note_of(&home, 1).exists());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn repositories_are_remembered_once() {
        let home = home("repositories");
        remember_repository(&home, Path::new("/a/repo"));
        remember_repository(&home, Path::new("/a/repo"));
        remember_repository(&home, Path::new("/another"));
        let mut known = known_repositories(&home);
        known.sort();
        assert_eq!(known, [PathBuf::from("/a/repo"), PathBuf::from("/another")]);
        let _ = std::fs::remove_dir_all(&home);
    }
}
