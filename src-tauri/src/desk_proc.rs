// opensidian, a vault-compatible markdown notes app.
// Copyright (C) 2026 Jendrik Poloczek
// SPDX-License-Identifier: GPL-3.0-or-later
/* cp-unrestricted (operator ruling 2026-10-18, stock parity): the real
   `child_process` behind ui/plugins/node-desktop.js. Stock runs a plugin's
   spawn/exec with the user's own rights; so do we, with one gate: the vault is
   trusted (the restricted-mode flip, ~/.opensidian.json). main.rs checks that
   on every spawn; there is no per-plugin consent.

   Process model, adapted from goal/td-desktop-caps desktop_claude.rs (our code)
   without its grants, broker dir, minimal env or output caps:
     - one long-lived thread does every fork, so PR_SET_PDEATHSIG (bound to the
       forking thread) fires only when the app itself goes away;
     - the child gets its own process group (kill reaches git's helpers too),
       stdio pipes, and no other fd of ours (close_range CLOEXEC);
     - two pump threads fill byte buffers the page drains with read();
     - a waiter reaps and records the exit code or the signal.
   cwd: the page sees the vault as "/vault" (getBasePath), so "/vault" and
   anything under it map to the real vault dir; a relative cwd or none at all
   is the vault root. Any other absolute dir is used as given (stock).
   env: the page's process.env is an empty shim, so a plugin's
   {...process.env, X: y} names only X. The child starts from the app's real
   environment and the plugin's keys are laid over it — what stock's child sees
   for the same call.
   Bytes cross the IPC as latin1 strings (one char per byte, lossless both ways),
   so no codec crate is needed. */
use std::collections::HashMap;
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::Duration;

use serde::Serialize;

/// Signal numbers. Unix: the platform's. Windows has no signals, but the page
/// still names them (child.kill("SIGTERM")) and reads the name back, so the
/// numbers Node documents are kept as plain labels; every one of them ends the
/// process tree (`Run::kill`).
#[cfg(unix)]
mod sig {
    pub use libc::{SIGABRT, SIGALRM, SIGHUP, SIGINT, SIGKILL, SIGPIPE, SIGQUIT, SIGSEGV, SIGTERM, SIGUSR1, SIGUSR2};
}
#[cfg(not(unix))]
mod sig {
    pub const SIGHUP: i32 = 1;
    pub const SIGINT: i32 = 2;
    pub const SIGQUIT: i32 = 3;
    pub const SIGABRT: i32 = 6;
    pub const SIGKILL: i32 = 9;
    pub const SIGUSR1: i32 = 10;
    pub const SIGSEGV: i32 = 11;
    pub const SIGUSR2: i32 = 12;
    pub const SIGPIPE: i32 = 13;
    pub const SIGALRM: i32 = 14;
    pub const SIGTERM: i32 = 15;
}

/// the page-side name of the vault root (FileSystemAdapter.getBasePath)
pub const VAULT_ALIAS: &str = "/vault";

/// "/vault[/x]" -> root[/x]; relative or empty -> under root; other absolute -> as given
pub fn map_cwd(root: &Path, cwd: Option<&str>) -> PathBuf {
    let c = cwd.unwrap_or("").trim();
    if c.is_empty() || c == "." {
        return root.to_path_buf();
    }
    if c == VAULT_ALIAS {
        return root.to_path_buf();
    }
    if let Some(rest) = c.strip_prefix(VAULT_ALIAS).and_then(|r| r.strip_prefix('/')) {
        return root.join(rest);
    }
    if c.starts_with('/') {
        return PathBuf::from(c);
    }
    root.join(c)
}

/// an argument naming a path under "/vault" means the same file in the real dir
/// (simple-git passes `-C <basePath>`-style paths, git config core.worktree, ...)
pub fn map_arg(root: &Path, a: &str) -> String {
    if a == VAULT_ALIAS || a.starts_with("/vault/") {
        return map_cwd(root, Some(a)).to_string_lossy().into_owned();
    }
    a.to_string()
}

/// app.openWithDefaultApp(path): a VAULT path only. "x" / "/vault/x" name a file or folder in
/// the vault; any other absolute path, any ".." component, a NUL, or a path whose real location
/// (symlinks resolved) is outside the vault's real root is refused with EPERM. The target must
/// exist (ENOENT otherwise), so the opener is never handed a name it would guess at.
pub fn vault_open_target(root: &Path, target: &str) -> Result<PathBuf, String> {
    let t = target.trim();
    if t.contains('\0') {
        return Err(format!("EPERM: vault-escape refused: {target}"));
    }
    let rel = if t == VAULT_ALIAS {
        ""
    } else if let Some(r) = t.strip_prefix(VAULT_ALIAS).and_then(|r| r.strip_prefix('/')) {
        r
    } else if t.starts_with('/') || t.starts_with('\\') || t.starts_with('~') {
        return Err(format!("EPERM: vault-escape refused: {target}"));
    } else {
        t
    };
    if rel.split(['/', '\\']).any(|c| c == "..") {
        return Err(format!("EPERM: vault-escape refused: {target}"));
    }
    let real_root = root.canonicalize().map_err(|e| format!("ENOENT: vault root: {e}"))?;
    let p = real_root.join(rel);
    let real = p.canonicalize().map_err(|_| format!("ENOENT: no such file or directory, '{target}'"))?;
    if !real.starts_with(&real_root) {
        return Err(format!("EPERM: vault-escape refused (symlink out of the vault): {target}"));
    }
    Ok(real)
}

pub fn latin1_to_bytes(s: &str) -> Vec<u8> {
    s.chars().map(|c| c as u32 as u8).collect()
}

pub fn bytes_to_latin1(b: &[u8]) -> String {
    b.iter().map(|&x| x as char).collect()
}

#[derive(Default)]
struct State {
    out: Vec<u8>,
    err: Vec<u8>,
    exit: Option<i32>,
    signal: Option<i32>,
    reaped: bool,
    pumps: u8,
}

pub struct Run {
    pid: i32,
    stdin: Mutex<Option<ChildStdin>>,
    st: Mutex<State>,
    cv: Condvar,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Chunk {
    /// latin1 byte strings
    pub out: String,
    pub err: String,
    pub exit: Option<i32>,
    /// Node signal name ("SIGTERM") when the child died of a signal
    pub signal: Option<&'static str>,
    /// reaped and both pipes drained: nothing more will ever come
    pub done: bool,
}

pub fn sig_name(n: i32) -> &'static str {
    match n {
        sig::SIGHUP => "SIGHUP",
        sig::SIGINT => "SIGINT",
        sig::SIGQUIT => "SIGQUIT",
        sig::SIGKILL => "SIGKILL",
        sig::SIGPIPE => "SIGPIPE",
        sig::SIGALRM => "SIGALRM",
        sig::SIGTERM => "SIGTERM",
        sig::SIGUSR1 => "SIGUSR1",
        sig::SIGUSR2 => "SIGUSR2",
        sig::SIGABRT => "SIGABRT",
        sig::SIGSEGV => "SIGSEGV",
        _ => "SIGTERM",
    }
}

pub fn sig_num(name: &str) -> Option<i32> {
    Some(match name.trim_start_matches("SIG").to_ascii_uppercase().as_str() {
        "HUP" => sig::SIGHUP,
        "INT" => sig::SIGINT,
        "QUIT" => sig::SIGQUIT,
        "KILL" => sig::SIGKILL,
        "PIPE" => sig::SIGPIPE,
        "ALRM" => sig::SIGALRM,
        "" | "TERM" => sig::SIGTERM,
        "USR1" => sig::SIGUSR1,
        "USR2" => sig::SIGUSR2,
        "ABRT" => sig::SIGABRT,
        _ => return None,
    })
}

impl Run {
    pub fn pid(&self) -> i32 {
        self.pid
    }

    /// signal the child's process group; false once it is reaped (Node: kill() -> false)
    pub fn kill(&self, sig: i32) -> bool {
        if self.st.lock().unwrap_or_else(|e| e.into_inner()).reaped {
            return false;
        }
        #[cfg(unix)]
        // SAFETY: plain syscall on our child's own group (process_group(0))
        return unsafe { libc::kill(-self.pid, sig) == 0 };
        // Windows: no signal can be delivered, so every one ends the tree. The
        // number is recorded FIRST: the waiter reports it as the cause of death
        // (Node: code null, signal "SIGTERM"), and it may reap before we return.
        #[cfg(windows)]
        return {
            self.st.lock().unwrap_or_else(|e| e.into_inner()).signal.get_or_insert(sig);
            let ok = crate::plat::kill_tree(self.pid as u32);
            if !ok {
                let mut s = self.st.lock().unwrap_or_else(|e| e.into_inner());
                if !s.reaped {
                    s.signal = None;
                }
            }
            ok
        };
    }

    pub fn write(&self, data: &[u8]) -> Result<(), String> {
        let mut g = self.stdin.lock().unwrap_or_else(|e| e.into_inner());
        let w = g.as_mut().ok_or("EPIPE: stdin is closed")?;
        w.write_all(data).and_then(|_| w.flush()).map_err(|e| format!("EPIPE: {e}"))
    }

    pub fn end(&self) {
        *self.stdin.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// what arrived since the last read, waiting up to `wait` for something new
    pub fn read(&self, wait: Duration) -> Chunk {
        let mut s = self.st.lock().unwrap_or_else(|e| e.into_inner());
        let idle = |s: &State| s.out.is_empty() && s.err.is_empty() && !(s.reaped && s.pumps == 0);
        if idle(&s) {
            s = self.cv.wait_timeout_while(s, wait, |s| idle(s)).unwrap_or_else(|e| e.into_inner()).0;
        }
        let out = bytes_to_latin1(&std::mem::take(&mut s.out));
        let err = bytes_to_latin1(&std::mem::take(&mut s.err));
        let done = s.reaped && s.pumps == 0;
        Chunk { out, err, exit: if done { s.exit } else { None }, signal: if done { s.signal.map(sig_name) } else { None }, done }
    }
}

type Job = Box<dyn FnOnce() + Send>;

fn forker() -> &'static Mutex<std::sync::mpsc::Sender<Job>> {
    static S: OnceLock<Mutex<std::sync::mpsc::Sender<Job>>> = OnceLock::new();
    S.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("desk-proc-fork".into())
            .spawn(move || {
                for job in rx {
                    job();
                }
            })
            .expect("desk_proc fork thread");
        Mutex::new(tx)
    })
}

pub struct Spec {
    pub file: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
}

/// Windows: the command line of a `sh -c <line>` request. The page presents
/// itself to plugins as Linux (node-desktop.js: process.platform, "/vault"), so
/// child_process.exec() and `shell: true` arrive as `/bin/sh -c "<line>"`.
/// There is no /bin/sh here; what Node itself runs for exec() on Windows is the
/// system's command interpreter, and so do we.
#[cfg(windows)]
fn shell_line(sp: &Spec) -> Option<&str> {
    let sh = matches!(sp.file.as_str(), "/bin/sh" | "sh" | "/bin/bash" | "bash");
    (sh && sp.args.len() == 2 && sp.args[0] == "-c").then(|| sp.args[1].as_str())
}

fn build(sp: &Spec) -> Command {
    #[cfg(not(windows))]
    let mut c = {
        let mut c = Command::new(&sp.file);
        c.args(&sp.args);
        c
    };
    #[cfg(windows)]
    let mut c = match shell_line(sp) {
        // Node's own form: cmd.exe /d /s /c "<line>", the line passed VERBATIM —
        // cmd does not parse quotes the way std's argument escaping writes them
        Some(line) => {
            let comspec = std::env::var_os("ComSpec").unwrap_or_else(|| "cmd.exe".into());
            let mut c = Command::new(comspec);
            std::os::windows::process::CommandExt::raw_arg(&mut c, format!("/d /s /c \"{line}\""));
            c
        }
        None => {
            let mut c = Command::new(&sp.file);
            c.args(&sp.args);
            c
        }
    };
    c.current_dir(&sp.cwd).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    for (k, v) in &sp.env {
        c.env(k, v);
    }
    #[cfg(unix)]
    c.process_group(0);
    // Windows: std marks every handle it opens non-inheritable and passes only
    // the three stdio pipes, so the child gets no other handle of ours (the
    // vault lock included). No console window for a console program.
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut c, crate::plat::CREATE_NO_WINDOW | crate::plat::CREATE_NEW_PROCESS_GROUP);
    // prctl and close_range are Linux syscalls. Elsewhere a child can outlive
    // the app (windows-port/AUDIT.md, point N).
    #[cfg(target_os = "linux")]
    // SAFETY: async-signal-safe syscalls only, between fork and exec
    unsafe {
        c.pre_exec(|| {
            libc::prctl(libc::PR_SET_PDEATHSIG, sig::SIGKILL as libc::c_ulong, 0, 0, 0);
            if libc::getppid() == 1 {
                return Err(std::io::Error::from_raw_os_error(libc::ESRCH));
            }
            // CLOEXEC, not close: std reports a failed exec (ENOENT) over its own cloexec
            // pipe, which a plain close_range would cut (spawn would "succeed")
            libc::syscall(libc::SYS_close_range, 3u32, u32::MAX, 4u32 /* CLOSE_RANGE_CLOEXEC */);
            Ok(())
        });
    }
    c
}

fn fork(sp: Spec) -> std::io::Result<Child> {
    let (tx, rx) = std::sync::mpsc::channel();
    forker()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .send(Box::new(move || {
            let _ = tx.send(build(&sp).spawn());
        }))
        .map_err(|_| std::io::Error::other("fork thread gone"))?;
    rx.recv().map_err(|_| std::io::Error::other("fork thread gone"))?
}

fn pump(r: Arc<Run>, mut src: impl Read, is_err: bool) {
    let mut b = [0u8; 16 * 1024];
    loop {
        match src.read(&mut b) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let mut s = r.st.lock().unwrap_or_else(|e| e.into_inner());
                if is_err { s.err.extend_from_slice(&b[..n]) } else { s.out.extend_from_slice(&b[..n]) }
            }
        }
        r.cv.notify_all();
    }
    r.st.lock().unwrap_or_else(|e| e.into_inner()).pumps -= 1;
    r.cv.notify_all();
}

/// Node-style error text: "<CODE>: spawn <file> <reason>"
fn spawn_err(file: &str, e: &std::io::Error) -> String {
    let code = match e.kind() {
        std::io::ErrorKind::NotFound => "ENOENT",
        std::io::ErrorKind::PermissionDenied => "EACCES",
        _ => "EIO",
    };
    format!("{code}: spawn {file} {e}")
}

pub fn start(sp: Spec) -> Result<Arc<Run>, String> {
    if sp.file.is_empty() {
        return Err("EINVAL: spawn: empty file".into());
    }
    if !sp.cwd.is_dir() {
        return Err(format!("ENOENT: spawn {} cwd {} is not a directory", sp.file, sp.cwd.display()));
    }
    let file = sp.file.clone();
    let mut child = fork(sp).map_err(|e| spawn_err(&file, &e))?;
    // Windows: tie the child (and what it starts) to the app's lifetime
    #[cfg(windows)]
    if let Err(e) = crate::plat::adopt(&child) {
        eprintln!("[desk] child {} is not in the app's job ({e}): it can outlive the app", child.id());
    }
    let run = Arc::new(Run {
        pid: child.id() as i32,
        stdin: Mutex::new(child.stdin.take()),
        st: Mutex::new(State { pumps: 2, ..State::default() }),
        cv: Condvar::new(),
    });
    let (o, e) = (child.stdout.take(), child.stderr.take());
    for (src, is_err) in [(o.map(|x| Box::new(x) as Box<dyn Read + Send>), false), (e.map(|x| Box::new(x) as Box<dyn Read + Send>), true)] {
        let r = run.clone();
        match src {
            Some(s) => {
                std::thread::spawn(move || pump(r, s, is_err));
            }
            None => r.st.lock().unwrap_or_else(|e| e.into_inner()).pumps -= 1,
        }
    }
    let r = run.clone();
    std::thread::spawn(move || {
        let st = child.wait();
        let mut s = r.st.lock().unwrap_or_else(|e| e.into_inner());
        if let Ok(st) = st {
            s.exit = st.code();
            #[cfg(unix)]
            {
                s.signal = st.signal();
            }
            // Windows: a child we terminated has a signal recorded by kill()
            // and, like Node reports it, no exit code
            #[cfg(not(unix))]
            if s.signal.is_some() {
                s.exit = None;
            }
        }
        s.reaped = true;
        drop(s);
        r.cv.notify_all();
    });
    Ok(run)
}

/* ---- the live runs, keyed by an id the page holds */
fn runs() -> &'static Mutex<HashMap<u64, Arc<Run>>> {
    static R: OnceLock<Mutex<HashMap<u64, Arc<Run>>>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn register(r: Arc<Run>) -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    runs().lock().unwrap_or_else(|e| e.into_inner()).insert(id, r);
    id
}

pub fn get(id: u64) -> Result<Arc<Run>, String> {
    runs().lock().unwrap_or_else(|e| e.into_inner()).get(&id).cloned().ok_or_else(|| "ESRCH: no such child".into())
}

/// a read that reported done drops the run
pub fn forget(id: u64) {
    runs().lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
}

/// flip OFF: plugin code stops, and so does everything it started
pub fn kill_all() {
    let all: Vec<Arc<Run>> = runs().lock().unwrap_or_else(|e| e.into_inner()).drain().map(|(_, r)| r).collect();
    for r in all {
        r.kill(sig::SIGKILL);
        r.end();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(file: &str, args: &[&str], cwd: &Path) -> Spec {
        Spec { file: file.into(), args: args.iter().map(|s| s.to_string()).collect(), cwd: cwd.into(), env: vec![] }
    }

    fn drain(r: &Run) -> (Vec<u8>, Vec<u8>, Chunk) {
        let (mut o, mut e) = (vec![], vec![]);
        for _ in 0..200 {
            let c = r.read(Duration::from_millis(50));
            o.extend(latin1_to_bytes(&c.out));
            e.extend(latin1_to_bytes(&c.err));
            if c.done {
                return (o, e, c);
            }
        }
        panic!("child never finished");
    }

    #[cfg(unix)] // asserts '/'-joined host paths
    #[test]
    fn desk_cwd_maps_the_vault_alias_to_the_real_dir() {
        let root = Path::new("/home/u/notes");
        assert_eq!(map_cwd(root, None), root);
        assert_eq!(map_cwd(root, Some("")), root);
        assert_eq!(map_cwd(root, Some("/vault")), root);
        assert_eq!(map_cwd(root, Some("/vault/sub/dir")), root.join("sub/dir"));
        assert_eq!(map_cwd(root, Some("sub")), root.join("sub"));
        assert_eq!(map_cwd(root, Some("/vaultx")), PathBuf::from("/vaultx"));
        assert_eq!(map_cwd(root, Some("/tmp")), PathBuf::from("/tmp"));
        assert_eq!(map_arg(root, "/vault/a.md"), "/home/u/notes/a.md");
        assert_eq!(map_arg(root, "--vault"), "--vault");
    }


    #[cfg(unix)]
    #[test]
    fn desk_open_target_is_vault_confined_and_symlink_safe() {
        let base = std::env::temp_dir().join(format!("desk-vot-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join("vault");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("sub/a.md"), b"a").unwrap();
        std::fs::write(base.join("outside.txt"), b"o").unwrap();
        std::os::unix::fs::symlink(base.join("outside.txt"), root.join("link.txt")).unwrap();
        std::os::unix::fs::symlink(root.join("sub/a.md"), root.join("in.md")).unwrap();
        let rr = root.canonicalize().unwrap();
        assert_eq!(vault_open_target(&root, "sub/a.md").unwrap(), rr.join("sub/a.md"));
        assert_eq!(vault_open_target(&root, "/vault/sub/a.md").unwrap(), rr.join("sub/a.md"));
        assert_eq!(vault_open_target(&root, "sub").unwrap(), rr.join("sub"));
        assert_eq!(vault_open_target(&root, "/vault").unwrap(), rr);
        assert_eq!(vault_open_target(&root, "in.md").unwrap(), rr.join("sub/a.md"));
        for bad in ["../outside.txt", "sub/../../outside.txt", "/etc/passwd", "/vaultx/a", "~/x", "link.txt", "a\0b"] {
            let e = vault_open_target(&root, bad).unwrap_err();
            assert!(e.starts_with("EPERM: vault-escape refused"), "{bad}: {e}");
        }
        assert!(vault_open_target(&root, "nope.md").unwrap_err().starts_with("ENOENT"));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn desk_latin1_round_trips_every_byte() {
        let all: Vec<u8> = (0..=255u8).collect();
        assert_eq!(latin1_to_bytes(&bytes_to_latin1(&all)), all);
    }

    #[cfg(unix)]
    #[test]
    fn desk_spawn_runs_in_cwd_with_env_stdin_and_exit_code() {
        let d = std::env::temp_dir();
        let mut sp = spec("/bin/sh", &["-c", "pwd; printf %s \"$DESK_T\"; cat; echo e >&2; exit 3"], &d);
        sp.env.push(("DESK_T".into(), "v1".into()));
        let r = start(sp).unwrap();
        r.write(b"in\xff").unwrap();
        r.end();
        let (o, e, c) = drain(&r);
        let want = format!("{}\nv1in\u{ff}", d.canonicalize().unwrap().display());
        assert_eq!(bytes_to_latin1(&o), want);
        assert_eq!(e, b"e\n");
        assert_eq!((c.exit, c.signal), (Some(3), None));
    }

    #[cfg(unix)]
    #[test]
    fn desk_kill_reports_the_signal_and_missing_exe_is_enoent() {
        let r = start(spec("/bin/sleep", &["30"], &std::env::temp_dir())).unwrap();
        assert!(r.kill(sig::SIGTERM));
        let (_, _, c) = drain(&r);
        assert_eq!((c.exit, c.signal), (None, Some("SIGTERM")));
        assert!(!r.kill(sig::SIGTERM));
        let e = start(spec("no-such-exe-desk", &[], &std::env::temp_dir())).err().unwrap();
        assert!(e.starts_with("ENOENT: spawn no-such-exe-desk"), "{e}");
        assert_eq!(sig_num("SIGKILL"), Some(sig::SIGKILL));
        assert_eq!(sig_num(""), Some(sig::SIGTERM));
    }

    /// Windows: a program run directly — cwd, the plugin's env laid over ours,
    /// stdin, both pipes and the exit code
    #[cfg(windows)]
    #[test]
    fn desk_win_spawn_runs_in_cwd_with_env_stdin_and_exit_code() {
        let d = std::env::temp_dir().canonicalize().unwrap();
        let mut sp = spec("cmd.exe", &["/d", "/c", "cd & echo %DESK_T% & findstr x & echo e 1>&2 & exit 3"], &d);
        sp.env = vec![("DESK_T".into(), "tee".into())];
        let r = start(sp).unwrap();
        r.write(b"axb\r\n").unwrap();
        r.end();
        let (o, e, c) = drain(&r);
        let o = String::from_utf8_lossy(&o).to_lowercase();
        let cwd = d.to_string_lossy().trim_start_matches(r"\\?\").to_lowercase();
        assert!(o.contains(&cwd), "cwd {cwd:?} not in {o:?}");
        assert!(o.contains("tee") && o.contains("axb"), "{o:?}");
        assert_eq!(String::from_utf8_lossy(&e).trim(), "e");
        assert_eq!((c.exit, c.signal, c.done), (Some(3), None, true));
    }

    /// Windows: exec() / shell:true arrive as `/bin/sh -c <line>` (the page says
    /// it is Linux) and run in the system's command interpreter, the line verbatim
    #[cfg(windows)]
    #[test]
    fn desk_win_sh_c_runs_the_line_in_the_command_interpreter() {
        let d = std::env::temp_dir();
        let r = start(spec("/bin/sh", &["-c", "echo \"a b\" & echo two>&2 & exit 5"], &d)).unwrap();
        let (o, e, c) = drain(&r);
        assert_eq!(String::from_utf8_lossy(&o).trim(), "\"a b\"", "quotes reach cmd as written");
        assert_eq!(String::from_utf8_lossy(&e).trim(), "two");
        assert_eq!(c.exit, Some(5));
        // not a shell request: three arguments, or another program
        assert!(shell_line(&spec("/bin/sh", &["-c", "x", "y"], &d)).is_none());
        assert!(shell_line(&spec("git", &["-c", "x"], &d)).is_none());
    }

    /// Windows: no signal exists, so kill() ends the tree and the page still
    /// reads the name it asked for; a missing program is ENOENT
    #[cfg(windows)]
    #[test]
    fn desk_win_kill_ends_the_tree_and_reports_the_signal() {
        let d = std::env::temp_dir();
        // cmd -> ping: the grandchild must go too
        let r = start(spec("cmd.exe", &["/d", "/c", "ping -n 60 127.0.0.1 >nul"], &d)).unwrap();
        std::thread::sleep(Duration::from_millis(400));
        assert!(r.kill(sig::SIGTERM));
        let (_, _, c) = drain(&r);
        assert_eq!((c.exit, c.signal, c.done), (None, Some("SIGTERM"), true));
        assert!(!r.kill(sig::SIGTERM), "a reaped child cannot be killed again");
        let e = start(spec("no-such-exe-desk", &[], &d)).err().unwrap();
        assert!(e.starts_with("ENOENT: spawn no-such-exe-desk"), "{e}");
    }
}
