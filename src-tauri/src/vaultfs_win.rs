// SPDX-License-Identifier: GPL-3.0-or-later
/* vaultfs on Windows — the same `Vault` surface as the POSIX fd walk in
   vaultfs.rs, built on what std offers here. Read that file's header first: the
   threat is the same (a sync client or a pull can plant a link anywhere in the
   vault, or swap a directory for one while we work).

   Windows has no openat in std, so the walk cannot be "relative to an fd".
   What it has instead is the share mode. The rule here:

     PIN   every directory on the way is opened as a HANDLE without
           FILE_SHARE_DELETE and with FILE_FLAG_OPEN_REPARSE_POINT. The second
           flag means a junction or a symlink is opened ITSELF, never its
           target; the first means that while we hold the handle nobody can
           rename, move or delete that directory — and NTFS refuses to rename
           an ancestor of an open handle too. So the chain from the vault root
           down to the directory we hold cannot be redirected under us.
     CHECK the opened HANDLE (not the path) must be a real directory: a name
           surrogate (junction, directory symlink) is refused, never followed.
           Cloud placeholders (OneDrive "files on demand") are reparse points
           but not name surrogates: they redirect nowhere and are accepted.
     LEAF  with the parent pinned, `parent\leaf` can only name an entry of that
           very directory. New files are CREATE_NEW (anything already there,
           a link included, is AlreadyExists); existing ones are opened with
           FILE_FLAG_OPEN_REPARSE_POINT and the HANDLE is checked to be a
           regular file before a byte is read or written.

   WEAKER THAN POSIX, and said plainly:
     * a leaf is checked by path and then renamed/removed by path. The parent
       cannot change in between, but the leaf itself can be swapped for a link
       in that window. A rename or delete then acts on the LINK (Windows never
       follows the last component for those), so nothing outside the vault is
       written or removed — but it is a check-then-act, not one atomic call.
     * no file identity (volume serial + file index need an unstable std API),
       so import_move never takes the hard-link road: it always copies and
       verifies byte for byte, and keeps the source open without write/delete
       sharing meanwhile so it cannot change under the copy.
     * no POSIX mode: nothing is chmod'ed; a new file gets the directory's
       inherited ACL. p_chmod maps to the read-only attribute, as Node does.
     * `durable` cannot flush the directory entry (FlushFileBuffers needs a
       write handle on the directory); the file's own bytes are flushed.

   Names: an alternate-data-stream form (`name:stream`) is refused before any
   call. Reserved device names and trailing dots/spaces are NOT handled here
   yet (windows-port/AUDIT.md, point W). */

use super::{fault, hook, parts, refused, same_bytes, ImportErr, Imported, PStat, SrcFate, PLUGIN_MODE_MASK};
use std::ffi::{OsStr, OsString};
use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

// Win32 values (winnt.h / fileapi.h), spelled out so no binding crate is needed
const FILE_SHARE_READ: u32 = 0x1;
const FILE_SHARE_WRITE: u32 = 0x2;
const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000; // required to open a directory
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000; // open a link itself, not its target
const FILE_READ_ATTRIBUTES: u32 = 0x80;
const FILE_WRITE_ATTRIBUTES: u32 = 0x100;
const ERROR_SHARING_VIOLATION: i32 = 32;
const ERROR_LOCK_VIOLATION: i32 = 33;

/// a directory INSIDE the vault, pinned by an open handle: the thing every
/// leaf op is relative to
pub struct Dir {
    pin: File,
    path: PathBuf,
}

/// the vault root, opened once
pub struct Vault {
    root: Dir,
}

/// open a directory ITSELF (a junction is not traversed), pinned against
/// rename/delete for as long as the handle lives
fn open_dir(p: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(p)
}

/// a regular file: not a directory, not a name surrogate (std reads the
/// reparse tag for both handle and path metadata)
fn is_reg(m: &Metadata) -> bool {
    m.file_type().is_file()
}

fn is_real_dir(m: &Metadata) -> bool {
    m.is_dir() && !m.file_type().is_symlink()
}

fn not_found(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::NotFound
}

/// `name:stream` would address an alternate data stream of `name`
fn name_ok(rel: &Path, c: &OsStr) -> io::Result<()> {
    if c.as_encoded_bytes().contains(&b':') {
        return Err(refused(rel, "':' in a name (alternate data stream form)"));
    }
    Ok(())
}

/// create `p` exclusively; a link already at that name is AlreadyExists
fn new_file(p: &Path, read: bool) -> io::Result<File> {
    OpenOptions::new().read(read).write(true).create_new(true).custom_flags(FILE_FLAG_OPEN_REPARSE_POINT).open(p)
}

/// 64 unpredictable bits without a crate: SipHash keyed from the OS RNG
fn rand64() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos()));
    h.finish()
}

/// a temp name nobody can predict. Dot-prefixed and `~`-suffixed, so no
/// index/watcher ever picks it up even if a crash leaves it behind. CREATE_NEW
/// refuses a planted name anyway; the randomness only avoids collisions.
fn tmp_name(leaf: &OsStr) -> OsString {
    let head: String = leaf.to_string_lossy().chars().take(48).collect();
    OsString::from(format!(".{head}.{:016x}~", rand64()))
}

/// rename `from` over `to` (an existing FILE is replaced). An antivirus or a
/// sync client holding either name for a moment is a sharing violation, not a
/// failure: retried briefly, bounded.
fn replace(from: &Path, to: &Path) -> io::Result<()> {
    let mut last = None;
    for i in 0..10u64 {
        match fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(e) if matches!(e.raw_os_error(), Some(ERROR_SHARING_VIOLATION) | Some(ERROR_LOCK_VIOLATION)) => {
                last = Some(e);
                std::thread::sleep(Duration::from_millis(10 + 10 * i));
            }
            Err(e) => return Err(e),
        }
    }
    Err(last.unwrap_or_else(|| io::Error::other("rename: retries exhausted")))
}

fn ms(t: io::Result<SystemTime>) -> i64 {
    t.ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_millis() as i64)
}

fn pstat_of(rel: &Path, m: &Metadata) -> io::Result<PStat> {
    let kind = if m.file_type().is_symlink() {
        return Err(refused(rel, "symlink in vault path"));
    } else if m.is_dir() {
        "folder"
    } else {
        "file"
    };
    Ok(PStat { kind, size: if kind == "file" { m.len() } else { 0 }, mtime_ms: ms(m.modified()), ctime_ms: ms(m.created()) })
}

impl Vault {
    pub fn open(root: &Path) -> io::Result<Vault> {
        // canonicalize yields the verbatim form (\\?\C:\...): no MAX_PATH limit,
        // and components are taken literally from here on
        let path = root.canonicalize()?;
        let pin = open_dir(&path)?;
        if !pin.metadata()?.is_dir() {
            return Err(io::Error::from(io::ErrorKind::NotADirectory));
        }
        Ok(Vault { root: Dir { pin, path } })
    }

    fn root_dir(&self) -> io::Result<Dir> {
        Ok(Dir { pin: self.root.pin.try_clone()?, path: self.root.path.clone() })
    }

    /// walk `comps` from the root: each one is opened pinned and checked on its
    /// handle. The previous pin is released only after the next one is held.
    fn walk(&self, rel: &Path, comps: &[&OsStr], create: bool) -> io::Result<Dir> {
        let mut cur = self.root_dir()?;
        for c in comps {
            name_ok(rel, c)?;
            let p = cur.path.join(c);
            let h = match open_dir(&p) {
                Ok(h) => h,
                Err(e) if not_found(&e) && create => {
                    match fs::create_dir(&p) {
                        Ok(()) => {}
                        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                        Err(e) => return Err(e),
                    }
                    open_dir(&p)?
                }
                Err(e) => return Err(e),
            };
            if !is_real_dir(&h.metadata()?) {
                return Err(refused(rel, "symlink or non-directory in vault path"));
            }
            cur = Dir { pin: h, path: p };
        }
        Ok(cur)
    }

    /// the directory `rel` names, created (`mkdir -p`) when `create`
    pub fn dir(&self, rel: &Path, create: bool) -> io::Result<Dir> {
        if rel.as_os_str().is_empty() {
            return self.root_dir();
        }
        let comps = parts(rel)?;
        self.walk(rel, &comps, create)
    }

    /// `mkdir -p rel`, never through a link
    pub fn mkdir_p(&self, rel: &Path) -> io::Result<()> {
        self.dir(rel, true).map(|_| ())
    }

    /// (pinned parent, full path of the leaf inside it)
    fn parent(&self, rel: &Path, create: bool) -> io::Result<(Dir, PathBuf)> {
        let comps = parts(rel)?;
        let (leaf, dirs) = comps.split_last().expect("parts is non-empty");
        name_ok(rel, leaf)?;
        let d = self.walk(rel, dirs, create)?;
        let p = d.path.join(leaf);
        Ok((d, p))
    }

    /// is `rel` a regular file (NOT a link to one)?
    pub fn is_file(&self, rel: &Path) -> bool {
        self.parent(rel, false).and_then(|(_d, p)| fs::symlink_metadata(p)).is_ok_and(|m| is_reg(&m))
    }

    /// replace `rel` with `bytes` through a random sibling temp and a rename.
    /// Parents are created. A leaf that exists and is not a regular file is
    /// refused. `durable`: see the header — the bytes are flushed either way.
    pub fn write_atomic(&self, rel: &Path, bytes: &[u8], _durable: bool) -> io::Result<()> {
        let (d, leaf) = self.parent(rel, true)?;
        hook("write_atomic:resolved");
        match fs::symlink_metadata(&leaf) {
            Ok(m) if is_reg(&m) => {}
            Ok(_) => return Err(refused(rel, "target is not a regular file")),
            Err(e) if not_found(&e) => {}
            Err(e) => return Err(e),
        }
        let tmp = d.path.join(tmp_name(leaf.file_name().unwrap_or_default()));
        let mut f = new_file(&tmp, false)?;
        let r = f.write_all(bytes).and_then(|_| f.sync_all());
        drop(f);
        if let Err(e) = r.and_then(|_| replace(&tmp, &leaf)) {
            let _ = fs::remove_file(&tmp); // only the temp WE created (CREATE_NEW)
            return Err(e);
        }
        Ok(())
    }

    /// create `rel` exclusively and hand back the open file; AlreadyExists when
    /// anything (a link included) is there. Parents are created when `parents`.
    pub fn create_new(&self, rel: &Path, parents: bool) -> io::Result<File> {
        let (_d, leaf) = self.parent(rel, parents)?;
        hook("create_new:resolved");
        new_file(&leaf, false)
    }

    /// remove the FILE `rel` (never a directory)
    pub fn unlink(&self, rel: &Path) -> io::Result<()> {
        let (_d, leaf) = self.parent(rel, false)?;
        fs::remove_file(leaf)
    }

    /// move the regular file `from` to `to`, never overwriting: `to` is claimed
    /// with CREATE_NEW first (AlreadyExists if taken), then the rename replaces
    /// only that zero-byte claim. Parents of `to` are created.
    pub fn move_file(&self, from: &Path, to: &Path) -> io::Result<()> {
        let (_df, pf) = self.parent(from, false)?;
        if !is_reg(&fs::symlink_metadata(&pf)?) {
            return Err(refused(from, "source is not a regular file"));
        }
        let (_dt, pt) = self.parent(to, true)?;
        drop(new_file(&pt, false)?);
        hook("move_file:claimed");
        if let Err(e) = replace(&pf, &pt) {
            let _ = fs::remove_file(&pt);
            return Err(e);
        }
        Ok(())
    }

    /// move the regular file `rel` into the directory `tdir` (created, never
    /// through a link) under `stem.ext`, `stem.1.ext`, ... — the first name we
    /// can CLAIM, so two deletes never overwrite each other. Returns the
    /// vault-relative destination.
    pub fn trash(&self, rel: &Path, tdir: &Path, stem: &str, ext: &str) -> io::Result<PathBuf> {
        let (_df, pf) = self.parent(rel, false)?;
        if !is_reg(&fs::symlink_metadata(&pf)?) {
            return Err(refused(rel, "not a regular file"));
        }
        let t = self.dir(tdir, true)?;
        hook("trash:resolved");
        for i in 0..10_000u32 {
            let name = if i == 0 { format!("{stem}.{ext}") } else { format!("{stem}.{i}.{ext}") };
            name_ok(rel, OsStr::new(&name))?;
            let pt = t.path.join(&name);
            match new_file(&pt, false) {
                Ok(f) => drop(f),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
            if let Err(e) = replace(&pf, &pt) {
                let _ = fs::remove_file(&pt);
                return Err(e);
            }
            return Ok(tdir.join(name));
        }
        Err(io::Error::new(io::ErrorKind::AlreadyExists, "no free name in the trash"))
    }

    /* osdrop — MOVE (or, keep_src, COPY) a file from OUTSIDE the vault into
       `dir`, never overwriting. Same contract as the POSIX version (vaultfs.rs):
       until the commit the source is untouched and the vault holds no new
       visible name; the source name goes only after the vault file is complete.
         1. the source is opened ITSELF (a symlink is refused) with read-only
            sharing: no other process can write, rename or delete it while the
            handle lives. Regular file, <= cap, magic bytes checked on the handle.
         2. STAGE: always a copy into a CREATE_NEW dot-temp (see header: no hard
            link road), flushed, then verified — size and mtime unchanged, and
            the temp read back from disk is byte-identical to the source.
         3. COMMIT under the first free name: a hard link refuses an existing
            name atomically; on a filesystem without hard links (FAT, some
            shares) the name is claimed with CREATE_NEW and the temp renamed
            over that claim.
         4. remove the source, only if the path still names a file of the same
            size and mtime (the handle had to be closed to delete it). */
    pub fn import_move(&self, src: &Path, dir: &Path, names: &mut dyn Iterator<Item = String>, cap: u64, magic: &[u8], keep_src: bool) -> Result<Imported, ImportErr> {
        let d = self.dir(dir, false).map_err(ImportErr::Io)?;
        // 1. the source, by handle
        match fs::symlink_metadata(src) {
            Ok(m) if !is_reg(&m) => return Err(ImportErr::NotRegular),
            Ok(_) => {}
            Err(e) => return Err(ImportErr::OpenSrc(e)),
        }
        let mut sf = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(src)
            .map_err(ImportErr::OpenSrc)?;
        let m0 = sf.metadata().map_err(ImportErr::Io)?;
        if !is_reg(&m0) {
            return Err(ImportErr::NotRegular);
        }
        if m0.len() > cap {
            return Err(ImportErr::TooBig(m0.len()));
        }
        if !magic.is_empty() {
            let mut head = vec![0u8; magic.len()];
            let n = super::fill(&mut sf, &mut head).map_err(ImportErr::Io)?;
            sf.seek(SeekFrom::Start(0)).map_err(ImportErr::Io)?;
            if head[..n] != *magic {
                return Err(ImportErr::BadMagic);
            }
        }
        let mut names = names.peekable();
        let first = names.peek().cloned().ok_or(ImportErr::NoFreeName)?;
        let tmp = d.path.join(tmp_name(OsStr::new(&first)));
        let unstage = || {
            let _ = fs::remove_file(&tmp);
        };
        // 2. stage: a verified copy
        let mut out = new_file(&tmp, true).map_err(ImportErr::Io)?;
        let r = (|| -> Result<(), ImportErr> {
            let mut buf = vec![0u8; 64 * 1024];
            let mut total: u64 = 0;
            loop {
                let n = sf.read(&mut buf).map_err(ImportErr::Io)?;
                if n == 0 {
                    break;
                }
                total += n as u64;
                if total > cap {
                    return Err(ImportErr::TooBig(total));
                }
                out.write_all(&buf[..n]).map_err(ImportErr::Io)?;
                fault("import:copying").map_err(ImportErr::Io)?;
            }
            out.sync_all().map_err(ImportErr::Io)?;
            let m1 = sf.metadata().map_err(ImportErr::Io)?;
            if m1.len() != m0.len() || m1.modified().ok() != m0.modified().ok() || total != m0.len() {
                return Err(ImportErr::Changed);
            }
            if !same_bytes(&mut sf, &mut out).map_err(ImportErr::Io)? {
                return Err(ImportErr::Io(io::Error::other("verify: the staged copy does not match the source")));
            }
            fault("import:staged").map_err(ImportErr::Io)
        })();
        drop(out);
        if let Err(e) = r {
            unstage();
            return Err(e);
        }
        // 3. commit under the first free name — never over an existing one
        let mut committed = None;
        for cand in names {
            if cand.is_empty() || cand.starts_with('.') || cand.contains(['/', '\\', ':']) {
                continue;
            }
            let dst = d.path.join(&cand);
            match fs::hard_link(&tmp, &dst) {
                Ok(()) => {
                    unstage();
                    committed = Some(cand);
                    break;
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(_) => {} // no hard links on this filesystem: claim + rename
            }
            match new_file(&dst, false) {
                Ok(f) => drop(f),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => {
                    unstage();
                    return Err(ImportErr::Io(e));
                }
            }
            if let Err(e) = replace(&tmp, &dst) {
                let _ = fs::remove_file(&dst);
                unstage();
                return Err(ImportErr::Io(e));
            }
            committed = Some(cand);
            break;
        }
        let Some(name) = committed else {
            unstage();
            return Err(ImportErr::NoFreeName);
        };
        if keep_src {
            return Ok(Imported { name, src: SrcFate::Copied, how: "copy" });
        }
        // 4. remove the source — only if the path still names the file we imported
        drop(sf);
        let fate = match fs::symlink_metadata(src) {
            Ok(m) if is_reg(&m) && m.len() == m0.len() && m.modified().ok() == m0.modified().ok() => {
                match fault("import:unlink-src").and_then(|_| fs::remove_file(src)) {
                    Ok(()) => SrcFate::Removed,
                    Err(e) => SrcFate::Kept(e.to_string()),
                }
            }
            Ok(_) => SrcFate::Kept("the source was replaced during the move".into()),
            Err(e) => SrcFate::Kept(e.to_string()),
        };
        Ok(Imported { name, src: fate, how: "copy" })
    }
}

/* ---- the plugin `fs` / DataAdapter backend (vaultfs.rs, td-vault-io) ---- */
impl Vault {
    pub fn p_stat(&self, rel: &Path) -> io::Result<PStat> {
        if rel.as_os_str().is_empty() {
            return pstat_of(rel, &self.root.pin.metadata()?);
        }
        let (_d, leaf) = self.parent(rel, false)?;
        pstat_of(rel, &fs::symlink_metadata(leaf)?)
    }

    /// open the regular file `rel` itself and check the OPENED handle
    fn p_open(&self, rel: &Path, append: bool) -> io::Result<File> {
        let (_d, leaf) = self.parent(rel, false)?;
        match fs::symlink_metadata(&leaf) {
            Ok(m) if is_reg(&m) => {}
            Ok(m) => return Err(pstat_of(rel, &m).err().unwrap_or_else(|| refused(rel, "not a regular file"))),
            Err(e) if not_found(&e) => {}
            Err(e) => return Err(e),
        }
        let mut o = OpenOptions::new();
        if append {
            o.append(true).create(true);
        } else {
            o.read(true);
        }
        let f = o.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT).open(&leaf)?;
        if !is_reg(&f.metadata()?) {
            return Err(refused(rel, "not a regular file"));
        }
        Ok(f)
    }

    /// the regular file `rel`, opened for reading and checked on its handle
    /// (main.rs media_open: canvas audio/video is read in slices from it)
    pub fn open_regular(&self, rel: &Path) -> io::Result<File> {
        self.p_open(rel, false)
    }

    pub fn p_read(&self, rel: &Path, cap: u64) -> io::Result<Vec<u8>> {
        let mut f = self.p_open(rel, false)?;
        let n = f.metadata()?.len();
        if n > cap {
            return Err(io::Error::other(format!("file too large ({n} > {cap} bytes)")));
        }
        let mut v = Vec::with_capacity(n as usize);
        (&mut f).take(cap + 1).read_to_end(&mut v)?;
        if v.len() as u64 > cap {
            return Err(io::Error::other("file too large"));
        }
        Ok(v)
    }

    /// writeFile (atomic replace) or appendFile; like Node, parents are NOT
    /// created (ENOENT)
    pub fn p_write(&self, rel: &Path, bytes: &[u8], append: bool) -> io::Result<()> {
        let (_d, _leaf) = self.parent(rel, false)?;
        if !append {
            return self.write_atomic(rel, bytes, false);
        }
        self.p_open(rel, true)?.write_all(bytes)
    }

    /// directory entries (name, "file"|"folder"); links are not listed —
    /// nothing a plugin could follow out
    pub fn p_list(&self, rel: &Path) -> io::Result<Vec<(String, &'static str)>> {
        let d = self.dir(rel, false)?;
        let mut out = Vec::new();
        for e in fs::read_dir(&d.path)? {
            let e = e?;
            // DirEntry::file_type does not follow: a junction is a symlink here
            let kind = match e.file_type() {
                Ok(t) if t.is_file() => "file",
                Ok(t) if t.is_dir() => "folder",
                _ => continue,
            };
            out.push((e.file_name().to_string_lossy().into_owned(), kind));
        }
        out.sort();
        Ok(out)
    }

    pub fn p_mkdir(&self, rel: &Path, recursive: bool) -> io::Result<()> {
        if recursive {
            return self.mkdir_p(rel);
        }
        let (_d, leaf) = self.parent(rel, false)?;
        if let Ok(m) = fs::symlink_metadata(&leaf) {
            pstat_of(rel, &m)?; // a link there is a refusal, not EEXIST
            return Err(io::Error::from(io::ErrorKind::AlreadyExists));
        }
        fs::create_dir(leaf)
    }

    /// remove a regular file, or an EMPTY directory (`dir`)
    pub fn p_remove(&self, rel: &Path, dir: bool) -> io::Result<()> {
        let (_d, leaf) = self.parent(rel, false)?;
        let st = pstat_of(rel, &fs::symlink_metadata(&leaf)?)?;
        match (st.kind, dir) {
            ("file", _) => fs::remove_file(leaf),
            ("folder", true) => fs::remove_dir(leaf),
            _ => Err(io::Error::from(io::ErrorKind::IsADirectory)),
        }
    }

    /// rename a regular file or directory; an existing regular-file target is
    /// replaced (Node semantics), a link target is refused. Both parents are
    /// pinned directories inside the vault, so the entry can only move within it.
    pub fn p_rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        let (_df, pf) = self.parent(from, false)?;
        pstat_of(from, &fs::symlink_metadata(&pf)?)?;
        let (_dt, pt) = self.parent(to, false)?;
        match fs::symlink_metadata(&pt) {
            Ok(m) => {
                pstat_of(to, &m)?;
            }
            Err(e) if not_found(&e) => {}
            Err(e) => return Err(e),
        }
        replace(&pf, &pt)
    }

    /// copyFile: the source's bytes by handle, then an atomic replace of `to`
    pub fn p_copy(&self, from: &Path, to: &Path) -> io::Result<()> {
        let bytes = self.p_read(from, super::PLUGIN_READ_CAP)?;
        self.p_write(to, &bytes, false)
    }

    /// fs.chmod for plugins. Windows has no permission bits: like Node here,
    /// only the owner-write bit means something — it maps to the read-only
    /// attribute of a regular FILE, set through a handle to the file itself
    /// (never a link's target). On a directory it is accepted and does nothing.
    pub fn p_chmod(&self, rel: &Path, mode: u32) -> io::Result<()> {
        let (_d, leaf) = self.parent(rel, false)?;
        let st = pstat_of(rel, &fs::symlink_metadata(&leaf)?)?; // a link -> refused
        hook("p_chmod:opened");
        if st.kind != "file" {
            return Ok(());
        }
        let f = OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES | FILE_WRITE_ATTRIBUTES)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(&leaf)?;
        let m = f.metadata()?;
        if !is_reg(&m) {
            return Err(refused(rel, "not a regular file"));
        }
        let mut p = m.permissions();
        let read_only = (mode & PLUGIN_MODE_MASK) & 0o200 == 0;
        p.set_readonly(read_only);
        f.set_permissions(p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vaultfs::plugin_op;

    /// (vault root, a directory outside it), both fresh
    fn fresh(tag: &str) -> (PathBuf, PathBuf) {
        let base = std::env::temp_dir().join(format!("opensidian-vfswin-{tag}-{}-{:x}", std::process::id(), rand64()));
        let (root, out) = (base.join("vault"), base.join("outside"));
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&out).unwrap();
        (root, out)
    }

    /// a junction needs no privilege (unlike a symlink) and redirects just the same
    fn junction(link: &Path, target: &Path) {
        let st = std::process::Command::new("cmd").args(["/C", "mklink", "/J"]).arg(link).arg(target).output().unwrap();
        assert!(st.status.success(), "mklink /J failed: {}", String::from_utf8_lossy(&st.stderr));
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        v.sort();
        v
    }

    #[test]
    fn vaultfs_win_write_atomic_creates_replaces_and_leaves_no_temp() {
        let (root, _) = fresh("write");
        let v = Vault::open(&root).unwrap();
        v.write_atomic(Path::new("sub/deep/N.md"), b"one", false).unwrap();
        v.write_atomic(Path::new("sub/deep/N.md"), b"two", true).unwrap();
        assert_eq!(fs::read(root.join("sub/deep/N.md")).unwrap(), b"two");
        assert_eq!(names(&root.join("sub/deep")), ["N.md"]);
        assert!(v.is_file(Path::new("sub/deep/N.md")));
        assert!(!v.is_file(Path::new("sub/deep")));
    }

    #[test]
    fn vaultfs_win_junction_in_path_is_refused_never_followed() {
        let (root, out) = fresh("junction");
        junction(&root.join("link"), &out);
        let v = Vault::open(&root).unwrap();
        let e = v.write_atomic(Path::new("link/X.md"), b"x", false).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::PermissionDenied, "{e}");
        assert!(e.raw_os_error().is_none(), "a refusal, not an OS error: {e}");
        assert!(v.mkdir_p(Path::new("link/sub")).is_err());
        assert!(v.create_new(Path::new("link/Y.md"), true).is_err());
        assert!(v.p_stat(Path::new("link")).is_err(), "a junction is not a folder");
        assert!(v.p_list(Path::new("link")).is_err());
        assert!(names(&out).is_empty(), "written through the junction: {:?}", names(&out));
        // and it is not listed among the vault's entries
        fs::write(root.join("real.md"), "r").unwrap();
        assert_eq!(v.p_list(Path::new("")).unwrap(), [("real.md".to_string(), "file")]);
    }

    #[test]
    fn vaultfs_win_trash_into_a_junction_is_refused() {
        let (root, out) = fresh("trash");
        fs::write(root.join("N.md"), "n").unwrap();
        junction(&root.join(".trash"), &out);
        let v = Vault::open(&root).unwrap();
        assert!(v.trash(Path::new("N.md"), Path::new(".trash"), "N", "md").is_err());
        assert!(root.join("N.md").exists(), "the note stays");
        assert!(names(&out).is_empty());
    }

    #[test]
    fn vaultfs_win_file_symlink_leaf_is_refused() {
        let (root, out) = fresh("symlink");
        fs::write(out.join("victim.md"), "keep").unwrap();
        if let Err(e) = std::os::windows::fs::symlink_file(out.join("victim.md"), root.join("L.md")) {
            eprintln!("vaultfs_win file-symlink test SKIPPED: cannot create a symlink here ({e}); enable Developer Mode to run it");
            return;
        }
        let v = Vault::open(&root).unwrap();
        assert!(v.write_atomic(Path::new("L.md"), b"evil", false).is_err());
        assert!(v.p_read(Path::new("L.md"), 1 << 20).is_err());
        assert!(v.p_write(Path::new("L.md"), b"evil", true).is_err());
        assert!(!v.is_file(Path::new("L.md")));
        assert_eq!(fs::read_to_string(out.join("victim.md")).unwrap(), "keep");
    }

    #[test]
    fn vaultfs_win_non_plain_paths_and_stream_names_are_refused() {
        let (root, _) = fresh("paths");
        let v = Vault::open(&root).unwrap();
        for p in ["../x.md", "a/../../x.md", "C:\\x.md", "\\x.md", "/x.md", "", "N.md:stream", "dir:s/N.md"] {
            assert!(v.write_atomic(Path::new(p), b"x", false).is_err(), "accepted {p:?}");
        }
        assert!(names(&root).is_empty(), "{:?}", names(&root));
    }

    #[test]
    fn vaultfs_win_a_held_dir_cannot_be_swapped() {
        let (root, out) = fresh("pin");
        fs::create_dir(root.join("a")).unwrap();
        let v = Vault::open(&root).unwrap();
        let held = v.dir(Path::new("a"), false).unwrap();
        // the swap an attacker needs: move the real dir away, plant a junction
        assert!(fs::rename(root.join("a"), root.join("b")).is_err(), "a pinned directory was renamed");
        assert!(fs::remove_dir(root.join("a")).is_err() || root.join("a").exists(), "a pinned directory was removed");
        drop(held);
        fs::rename(root.join("a"), root.join("b")).unwrap();
        junction(&root.join("a"), &out);
        assert!(v.write_atomic(Path::new("a/X.md"), b"x", false).is_err());
        assert!(names(&out).is_empty());
    }

    #[test]
    fn vaultfs_win_move_and_trash_never_overwrite() {
        let (root, _) = fresh("move");
        let v = Vault::open(&root).unwrap();
        v.write_atomic(Path::new("A.md"), b"a", false).unwrap();
        v.write_atomic(Path::new("B.md"), b"b", false).unwrap();
        let e = v.move_file(Path::new("A.md"), Path::new("B.md")).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(root.join("B.md")).unwrap(), b"b");
        v.move_file(Path::new("A.md"), Path::new("d/C.md")).unwrap();
        assert_eq!(fs::read(root.join("d/C.md")).unwrap(), b"a");
        assert!(!root.join("A.md").exists());
        v.write_atomic(Path::new("B2.md"), b"b2", false).unwrap();
        assert_eq!(v.trash(Path::new("B.md"), Path::new(".trash"), "B", "md").unwrap(), Path::new(".trash").join("B.md"));
        fs::rename(root.join("B2.md"), root.join("B.md")).unwrap();
        assert_eq!(v.trash(Path::new("B.md"), Path::new(".trash"), "B", "md").unwrap(), Path::new(".trash").join("B.1.md"));
        assert_eq!(fs::read(root.join(".trash/B.md")).unwrap(), b"b");
        assert_eq!(fs::read(root.join(".trash/B.1.md")).unwrap(), b"b2");
    }

    #[test]
    fn vaultfs_win_import_moves_copies_and_checks_magic() {
        let (root, out) = fresh("import");
        fs::create_dir(root.join("att")).unwrap();
        let v = Vault::open(&root).unwrap();
        let cands = || vec!["f.pdf".to_string(), "f 1.pdf".to_string()].into_iter();
        fs::write(out.join("f.pdf"), b"%PDF-1.7 body").unwrap();
        // copy: the source stays
        let i = v.import_move(&out.join("f.pdf"), Path::new("att"), &mut cands(), 1 << 20, b"%PDF-", true).unwrap();
        assert_eq!((i.name.as_str(), &i.src), ("f.pdf", &SrcFate::Copied));
        assert!(out.join("f.pdf").exists());
        // move: the next free name, the source goes, no temp is left
        let i = v.import_move(&out.join("f.pdf"), Path::new("att"), &mut cands(), 1 << 20, b"%PDF-", false).unwrap();
        assert_eq!((i.name.as_str(), &i.src), ("f 1.pdf", &SrcFate::Removed));
        assert!(!out.join("f.pdf").exists());
        assert_eq!(names(&root.join("att")), ["f 1.pdf", "f.pdf"]);
        assert_eq!(fs::read(root.join("att/f 1.pdf")).unwrap(), b"%PDF-1.7 body");
        // wrong magic, over the cap, a directory: refused with the source intact
        fs::write(out.join("g.pdf"), b"not a pdf").unwrap();
        assert!(matches!(v.import_move(&out.join("g.pdf"), Path::new("att"), &mut cands(), 1 << 20, b"%PDF-", false), Err(ImportErr::BadMagic)));
        assert!(matches!(v.import_move(&out.join("g.pdf"), Path::new("att"), &mut cands(), 3, b"", false), Err(ImportErr::TooBig(9))));
        assert!(matches!(v.import_move(&out, Path::new("att"), &mut cands(), 1 << 20, b"", false), Err(ImportErr::NotRegular)));
        assert!(out.join("g.pdf").exists());
        assert_eq!(names(&root.join("att")), ["f 1.pdf", "f.pdf"]);
    }

    #[test]
    fn vaultfs_win_plugin_ops_stay_inside_and_report_node_codes() {
        let (root, out) = fresh("pvfs");
        junction(&root.join("link"), &out);
        let op = |op: &str, p: &str, to: Option<&str>, data: Option<&[u8]>| plugin_op(&root, op, p, to, data.map(crate::vaultfs::b64_encode).as_deref(), false);
        op("mkdir", "d", None, None).unwrap();
        op("write", "d/a.txt", None, Some(b"hi")).unwrap();
        op("append", "d/a.txt", None, Some(b"!")).unwrap();
        assert_eq!(op("read", "d/a.txt", None, None).unwrap(), serde_json::json!(crate::vaultfs::b64_encode(b"hi!")));
        op("copy", "d/a.txt", Some("d/b.txt"), None).unwrap();
        op("rename", "d/b.txt", Some("d/c.txt"), None).unwrap();
        assert_eq!(op("exists", "d/c.txt", None, None).unwrap(), serde_json::json!(true));
        assert_eq!(op("exists", "d/nope.txt", None, None).unwrap(), serde_json::json!(false));
        assert_eq!(op("exists", "nodir/nope.txt", None, None).unwrap(), serde_json::json!(false));
        assert_eq!(op("stat", "d", None, None).unwrap()["type"], "folder");
        for (o, p, code) in [("read", "d/nope.txt", "ENOENT"), ("mkdir", "d", "EEXIST"), ("rmdir", "d", "ENOTEMPTY"), ("unlink", "d", "EISDIR")] {
            let e = op(o, p, None, None).unwrap_err();
            assert!(e.starts_with(&format!("{code}:")), "{o} {p}: {e}");
        }
        for p in ["link/x.txt", "../x.txt", "d/../../x.txt", "C:/x.txt", "d\\a.txt"] {
            let e = op("write", p, None, Some(b"x")).unwrap_err();
            assert!(e.starts_with("EPERM: vault-escape refused"), "{p}: {e}");
        }
        assert!(names(&out).is_empty());
        op("unlink", "d/a.txt", None, None).unwrap();
        op("unlink", "d/c.txt", None, None).unwrap();
        op("rmdir", "d", None, None).unwrap();
        assert!(!root.join("d").exists());
    }

    #[test]
    fn vaultfs_win_chmod_maps_owner_write_to_read_only() {
        let (root, _) = fresh("chmod");
        let v = Vault::open(&root).unwrap();
        v.write_atomic(Path::new("h.sh"), b"#!/bin/sh\n", false).unwrap();
        v.p_chmod(Path::new("h.sh"), 0o444).unwrap();
        assert!(fs::metadata(root.join("h.sh")).unwrap().permissions().readonly());
        v.p_chmod(Path::new("h.sh"), 0o755).unwrap();
        assert!(!fs::metadata(root.join("h.sh")).unwrap().permissions().readonly());
        fs::create_dir(root.join("d")).unwrap();
        v.p_chmod(Path::new("d"), 0o700).unwrap();
        assert!(v.p_chmod(Path::new("nope"), 0o644).is_err());
    }
}
