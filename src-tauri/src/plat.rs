// SPDX-License-Identifier: GPL-3.0-or-later
/* plat — the few questions whose answer depends on the operating system, each
   asked in ONE place so the rest of the bin stays free of `cfg`:
     where is the user's home, which program opens a file or URL with the
     desktop's default handler, is this pid still running, stop a process tree.
   Linux answers are the ones the callers had inline before this file existed
   (same expressions, moved); Windows and macOS get their own.

   The Windows half calls four Win32 functions directly. They are declared by
   hand rather than through a binding crate: std already links kernel32, the
   signatures are frozen ABI, and four declarations do not justify a direct
   dependency (AGENTS.md "Dependencies"). */
use std::path::PathBuf;

/// the user's home directory. Unix: $HOME, "/" when unset or not UTF-8 (as
/// before). Windows: %USERPROFILE%, the system drive root when unset.
pub fn home_string() -> String {
    #[cfg(not(windows))]
    return std::env::var("HOME").unwrap_or_else(|_| "/".into());
    #[cfg(windows)]
    return std::env::var("USERPROFILE").ok().filter(|h| !h.is_empty()).unwrap_or_else(|| "C:\\".into());
}

pub fn home() -> PathBuf {
    PathBuf::from(home_string())
}

/// `canonicalize`, in the form a person (and the config file) should see.
/// Windows answers in the verbatim form (`\\?\C:\Users\...`), which is right for
/// file calls but wrong as a NAME: it is not what the user typed or will
/// recognise, Explorer does not accept it, and the picker's rule (a plain `X:\`
/// path) refuses it. The prefix is dropped when what remains is an ordinary
/// drive or UNC path short enough to need no such prefix.
pub fn canon(p: &std::path::Path) -> std::io::Result<PathBuf> {
    let c = p.canonicalize()?;
    #[cfg(windows)]
    if let Some(s) = c.to_str() {
        if s.len() < 248 {
            if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
                return Ok(PathBuf::from(format!(r"\\{rest}")));
            }
            if let Some(rest) = s.strip_prefix(r"\\?\") {
                let b = rest.as_bytes();
                if b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\' {
                    return Ok(PathBuf::from(rest));
                }
            }
        }
    }
    Ok(c)
}

/// Windows: the roots a folder picker starts from — `C:\`, `D:\`, ... for every
/// drive letter the system has mounted (no probing of the drives themselves, so
/// an unreachable network drive cannot stall the listing).
#[cfg(windows)]
pub fn drives() -> Vec<String> {
    // SAFETY: no arguments; returns a bitmask, bit 0 = A:
    let mask = unsafe { win::GetLogicalDrives() };
    (0..26u32).filter(|i| mask & (1 << i) != 0).map(|i| format!("{}:\\", (b'A' + i as u8) as char)).collect()
}

/// the desktop opener run as a plain child process (no shell). Windows has no
/// such program worth trusting with arbitrary arguments: see `shell_open`.
#[cfg(all(unix, not(target_os = "macos")))]
pub const OPENER: &str = "xdg-open";
#[cfg(target_os = "macos")]
pub const OPENER: &str = "/usr/bin/open";

/// Windows: hand `target` (a URL or a path) to the shell's default handler.
/// ShellExecuteW takes the target as ONE string, so there is no command line
/// to quote and nothing a space, quote or `&` in it could split.
/// NOTE the difference from xdg-open: "open" on an executable file RUNS it.
/// Both callers are gated (http/https/mailto only, or a trusted vault's plugin).
#[cfg(windows)]
pub fn shell_open(target: &std::ffi::OsStr) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    let wide = |s: &std::ffi::OsStr| -> std::io::Result<Vec<u16>> {
        let mut v: Vec<u16> = s.encode_wide().collect();
        if v.contains(&0) {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "NUL in target"));
        }
        v.push(0);
        Ok(v)
    };
    let (verb, file) = (wide(std::ffi::OsStr::new("open"))?, wide(target)?);
    // SAFETY: both strings are NUL-terminated and outlive the call; the other
    // pointers are documented as optional (null)
    let r = unsafe { win::ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), file.as_ptr(), std::ptr::null(), std::ptr::null(), win::SW_SHOWNORMAL) };
    // "If the function succeeds, it returns a value greater than 32"
    if r > 32 {
        Ok(())
    } else {
        Err(std::io::Error::other(format!("ShellExecute failed (code {r})")))
    }
}

/// Windows: is `pid` a running process? A handle we may wait on that has not
/// been signalled yet. A pid we cannot open at all counts as gone.
#[cfg(windows)]
pub fn alive(pid: u32) -> bool {
    // SAFETY: plain calls; the handle is checked for null and closed once
    unsafe {
        let h = win::OpenProcess(win::SYNCHRONIZE, 0, pid);
        if h.is_null() {
            return false;
        }
        let running = win::WaitForSingleObject(h, 0) == win::WAIT_TIMEOUT;
        win::CloseHandle(h);
        running
    }
}

/// CreateProcess flag: no console window for a console child of this GUI app
#[cfg(windows)]
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// CreateProcess flag: the child is the root of its own process group
#[cfg(windows)]
pub const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

/// Windows: end `pid` and every process it started. There is no signal to
/// send; `taskkill /T /F` walks the tree and terminates it. (A Job Object
/// would also cover children that outlive their parent — AUDIT.md, point N.)
#[cfg(windows)]
pub fn kill_tree(pid: u32) -> bool {
    use std::os::windows::process::CommandExt;
    let sys = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("C:\\Windows"));
    std::process::Command::new(sys.join("System32").join("taskkill.exe"))
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .is_ok_and(|s| s.success())
}

#[cfg(windows)]
mod win {
    pub type Handle = *mut core::ffi::c_void;
    pub const SYNCHRONIZE: u32 = 0x0010_0000;
    pub const WAIT_TIMEOUT: u32 = 258;
    pub const SW_SHOWNORMAL: i32 = 1;

    #[link(name = "kernel32")]
    extern "system" {
        pub fn OpenProcess(access: u32, inherit: i32, pid: u32) -> Handle;
        pub fn WaitForSingleObject(h: Handle, ms: u32) -> u32;
        pub fn CloseHandle(h: Handle) -> i32;
        pub fn GetLogicalDrives() -> u32;
    }

    #[link(name = "shell32")]
    extern "system" {
        /// returns an HINSTANCE that is really an integer status
        pub fn ShellExecuteW(hwnd: Handle, verb: *const u16, file: *const u16, params: *const u16, dir: *const u16, show: i32) -> isize;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plat_home_is_an_absolute_directory_path() {
        assert!(home().is_absolute(), "{:?}", home());
    }

    #[cfg(windows)]
    #[test]
    fn plat_alive_follows_a_real_child() {
        use std::os::windows::process::CommandExt;
        let mut c = std::process::Command::new("cmd").args(["/C", "pause"]).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::null()).creation_flags(CREATE_NO_WINDOW).spawn().unwrap();
        let pid = c.id();
        assert!(alive(pid), "a running child reads as alive");
        assert!(alive(std::process::id()), "this process is alive");
        assert!(kill_tree(pid));
        let _ = c.wait();
        assert!(!alive(pid), "an exited child reads as gone");
    }

    #[cfg(windows)]
    #[test]
    fn plat_canon_is_a_plain_drive_path_and_drives_lists_its_drive() {
        let t = canon(&std::env::temp_dir()).unwrap();
        let s = t.to_str().unwrap();
        assert!(!s.starts_with(r"\\?\"), "verbatim prefix left in {s}");
        assert!(s.as_bytes()[1] == b':' && s.as_bytes()[2] == b'\\', "{s}");
        assert_eq!(t.canonicalize().unwrap(), std::env::temp_dir().canonicalize().unwrap(), "same directory");
        let d = drives();
        assert!(d.iter().any(|x| s.to_ascii_uppercase().starts_with(x.as_str())), "{d:?} lacks the drive of {s}");
        assert!(d.iter().all(|x| x.len() == 3 && x.ends_with(r":\")), "{d:?}");
    }

    #[cfg(windows)]
    #[test]
    fn plat_shell_open_refuses_an_embedded_nul() {
        let e = shell_open(std::ffi::OsStr::new("https://example.org/\0calc.exe")).unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::InvalidInput);
    }
}
