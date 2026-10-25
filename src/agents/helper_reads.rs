//! Bounded, shell-free repository tools for managed helpers. Unix walks use
//! pinned directory handles and openat(O_NOFOLLOW), not check-then-open paths.
use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};
use std::fs::File;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

const MAX_FILE: usize = 256 * 1024;
const MAX_REPLY: usize = 24 * 1024;
const MAX_SCAN: usize = 8 * 1024 * 1024;
const MAX_ENTRIES: usize = 2000;

pub struct RepoReads { root: PathBuf, directory: File }

impl RepoReads {
    pub fn new(root: &Path) -> Result<Self> {
        let root = root.canonicalize()?;
        #[cfg(unix)]
        let directory = Self::open_from(&File::open("/")?,root.strip_prefix("/")?)?;
        #[cfg(not(unix))]
        let directory = File::open(&root)?;
        ensure!(directory.metadata()?.is_dir(), "helper root is not a directory");
        ensure!(cfg!(unix), "Codex helper file access currently requires Unix descriptor-relative reads");
        Ok(Self { root, directory })
    }

    fn relative(&self, raw: &str) -> Result<PathBuf> {
        ensure!(raw.len() <= 4096, "path is too long");
        let path = Path::new(raw);
        let path = if path.is_absolute() { path.strip_prefix(&self.root).context("path is outside the helper repository")? } else { path };
        ensure!(path.components().all(|c|matches!(c, Component::Normal(_) | Component::CurDir)), "path must remain inside the helper repository");
        Ok(path.to_path_buf())
    }

    #[cfg(unix)]
    fn open(&self, relative: &Path) -> Result<File> {
        Self::open_from(&self.directory,relative)
    }

    #[cfg(unix)]
    fn open_from(directory: &File, relative: &Path) -> Result<File> {
        use std::os::{fd::{AsRawFd, FromRawFd}, unix::ffi::OsStrExt};
        let mut current = directory.try_clone()?;
        let parts: Vec<_> = relative.components().filter_map(|p| if let Component::Normal(n)=p {Some(n)} else {None}).collect();
        for (index, part) in parts.iter().enumerate() {
            let name = std::ffi::CString::new(part.as_bytes())?;
            let flags = libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK
                | if index + 1 < parts.len() { libc::O_DIRECTORY } else { 0 };
            // The parent descriptor pins each directory even if a path is
            // renamed concurrently. No component may be a symlink.
            let fd = unsafe { libc::openat(current.as_raw_fd(), name.as_ptr(), flags) };
            ensure!(fd >= 0, "helper read refused (missing, inaccessible or symlink): {}", relative.display());
            current = unsafe { File::from_raw_fd(fd) };
        }
        Ok(current)
    }

    #[cfg(not(unix))]
    fn open(&self, _: &Path) -> Result<File> { anyhow::bail!("descriptor-relative helper reads unavailable") }

    fn text(&self, path: &Path) -> Result<(String, bool)> {
        let file = self.open(path)?;
        ensure!(file.metadata()?.is_file(), "helper reads require a regular file");
        let mut bytes = Vec::new();
        file.take((MAX_FILE + 1) as u64).read_to_end(&mut bytes)?;
        let truncated = bytes.len() > MAX_FILE;
        bytes.truncate(MAX_FILE);
        ensure!(!bytes.contains(&0), "binary file is not supported");
        Ok((String::from_utf8_lossy(&bytes).into_owned(), truncated))
    }

    fn entries(&self, path: &Path) -> Result<Vec<PathBuf>> {
        let directory = self.open(path)?;
        ensure!(directory.metadata()?.is_dir(), "helper list requires a directory");
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            use std::os::unix::ffi::OsStrExt;
            // A fresh description (not dup) gives each enumeration its own
            // directory offset, including concurrent calls at the root.
            let fd = unsafe { libc::openat(directory.as_raw_fd(), c".".as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC | libc::O_DIRECTORY) };
            ensure!(fd>=0,"cannot enumerate helper directory");
            let stream = unsafe { libc::fdopendir(fd) };
            if stream.is_null() { unsafe {libc::close(fd)}; anyhow::bail!("cannot enumerate helper directory"); }
            struct Directory(*mut libc::DIR);
            impl Drop for Directory { fn drop(&mut self) { unsafe {libc::closedir(self.0)}; } }
            let stream = Directory(stream);
            let mut entries = Vec::new();
            loop {
                #[cfg(target_os="macos")]
                let errno = unsafe { libc::__error() };
                #[cfg(not(target_os="macos"))]
                let errno = unsafe { libc::__errno_location() };
                unsafe { *errno=0; }
                let entry = unsafe { libc::readdir(stream.0) };
                if entry.is_null() { ensure!(unsafe{*errno}==0,"helper directory enumeration failed"); break; }
                let name = unsafe { std::ffi::CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
                if name==b"." || name==b".." {continue;}
                entries.push(path.join(std::ffi::OsStr::from_bytes(name)));
                if entries.len()>MAX_ENTRIES {break;}
            }
            entries.sort();
            Ok(entries)
        }
        #[cfg(not(unix))]
        anyhow::bail!("descriptor-relative helper reads unavailable")
    }

    pub fn call(&self, name: &str, arguments: &Value) -> Result<Value> {
        let args = arguments.as_object().context("tool arguments must be an object")?;
        let keys: &[&str] = match name { "repo_read" => &["path","start_line","max_lines"], "repo_list" => &["path"], "repo_search" => &["path","pattern"], _ => anyhow::bail!("unknown helper tool {name}") };
        ensure!(args.keys().all(|k| keys.contains(&k.as_str())), "unknown helper argument");
        let raw = match args.get("path") { None => ".", Some(v) => v.as_str().context("path must be a string")? };
        let path = self.relative(raw)?;
        let mut out = Vec::new();
        let mut bytes = 0;
        let mut truncated = false;
        if name == "repo_read" {
            let number = |key: &str, default: u64| -> Result<u64> { match args.get(key) {None => Ok(default),Some(v) => v.as_u64().context("line bounds must be positive integers")} };
            let start = number("start_line",1)?;
            let count = number("max_lines",200)?;
            ensure!((1..=100_000).contains(&start) && (1..=300).contains(&count), "line bounds out of range");
            let (text, cut) = self.text(&path)?;
            truncated = cut;
            for (i,line) in text.lines().enumerate().skip((start-1) as usize).take(count as usize) {
                let row = json!({"path":path,"line":i+1,"text":line});
                let size = row.to_string().len();
                if bytes+size > MAX_REPLY { truncated=true; break; }
                bytes += size; out.push(row);
            }
        } else {
            let initial=self.open(&path)?;
            ensure!(initial.metadata()?.is_file() || initial.metadata()?.is_dir(),"helper reads require regular files or directories");
            let pattern = if name == "repo_search" { Some(args.get("pattern").and_then(Value::as_str).filter(|p|!p.is_empty() && p.len()<=1024).context("search needs a nonempty pattern of at most 1024 bytes")?) } else {None};
            let mut pending = vec![path];
            let mut visited = 0;
            let mut scanned = 0;
            let deadline = std::time::Instant::now()+std::time::Duration::from_secs(2);
            while let Some(path) = pending.pop() {
                visited += 1;
                if visited > MAX_ENTRIES || scanned >= MAX_SCAN || std::time::Instant::now()>=deadline { truncated=true; break; }
                let file = match self.open(&path) { Ok(f)=>f, Err(_) => continue };
                if file.metadata()?.is_dir() {
                    if path.file_name().is_some_and(|n|n==".git" || n=="node_modules" || n=="target") { continue; }
                    let entries = self.entries(&path)?;
                    if pending.len()+entries.len()>MAX_ENTRIES { truncated=true; }
                    let room = MAX_ENTRIES.saturating_sub(pending.len());
                    pending.extend(entries.into_iter().take(room).rev());
                    continue;
                }
                if !file.metadata()?.is_file() { continue; }
                let rows = if let Some(pattern) = pattern {
                    let (text, cut) = match self.text(&path) { Ok(t)=>t, Err(_)=>continue };
                    scanned += text.len(); truncated |= cut;
                    text.lines().enumerate().filter(|(_,l)|l.contains(pattern)).take(MAX_REPLY/16+1).map(|(i,l)|json!({"path":path,"line":i+1,"text":l})).collect::<Vec<_>>()
                } else { vec![json!({"path":path})] };
                for row in rows {
                    let size = row.to_string().len();
                    if bytes+size>MAX_REPLY { truncated=true; pending.clear(); break; }
                    bytes+=size; out.push(row);
                }
            }
        }
        Ok(json!({"results":out,"truncated":truncated}))
    }
}

pub fn tool_specs() -> Value {
    json!([
        {"type":"function","name":"repo_read","description":"Read bounded lines of a repository text file; symlinks and outside paths are refused.","inputSchema":{"type":"object","properties":{"path":{"type":"string"},"start_line":{"type":"integer","minimum":1},"max_lines":{"type":"integer","minimum":1,"maximum":300}},"required":["path"],"additionalProperties":false}},
        {"type":"function","name":"repo_list","description":"List repository files recursively, bounded; skips symlinks, .git, target and node_modules.","inputSchema":{"type":"object","properties":{"path":{"type":"string"}},"additionalProperties":false}},
        {"type":"function","name":"repo_search","description":"Search repository text files for a literal string, with path and line citations. Bounded; skips symlinks and generated/dependency directories.","inputSchema":{"type":"object","properties":{"path":{"type":"string"},"pattern":{"type":"string"}},"required":["pattern"],"additionalProperties":false}}
    ])
}

#[cfg(all(test,unix))]
mod tests {
    use super::*;
    #[test]
    fn read_list_and_search_stay_inside_pinned_repository() {
        let t=tempfile::tempdir().unwrap(); let root=t.path().join("repo"); std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("source.rs"),"first\nneedle\nthird\n").unwrap();
        std::fs::write(t.path().join("outside"),"private needle").unwrap();
        std::os::unix::fs::symlink(t.path().join("outside"),root.join("escape")).unwrap();
        let r=RepoReads::new(&root).unwrap();
        assert_eq!(r.call("repo_read",&json!({"path":"source.rs","start_line":2,"max_lines":1})).unwrap()["results"][0]["text"],"needle");
        for path in ["../outside".to_string(),t.path().join("outside").display().to_string(),"escape".into()] { assert!(r.call("repo_read",&json!({"path":path})).is_err()); }
        let list=r.call("repo_list",&json!({})).unwrap(); assert_eq!(list["results"].as_array().unwrap().len(),1);
        let search=r.call("repo_search",&json!({"pattern":"needle"})).unwrap(); assert_eq!(search["results"].as_array().unwrap().len(),1);
        assert!(!search.to_string().contains("private"));
        assert!(r.call("repo_read",&json!({"path":"source.rs","command":"anything"})).is_err());
        assert!(r.call("execute",&json!({})).is_err());
    }
    #[test]
    fn replacing_a_directory_with_a_symlink_cannot_redirect_a_read() {
        let t=tempfile::tempdir().unwrap(); let root=t.path().join("repo"); std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(t.path().join("secret"),"outside").unwrap();
        let r=RepoReads::new(&root).unwrap();
        std::fs::remove_dir(root.join("sub")).unwrap(); std::os::unix::fs::symlink(t.path(),root.join("sub")).unwrap();
        assert!(r.call("repo_read",&json!({"path":"sub/secret"})).is_err());
        std::fs::rename(&root,t.path().join("moved")).unwrap(); std::os::unix::fs::symlink(t.path(),&root).unwrap();
        assert!(r.call("repo_read",&json!({"path":"secret"})).is_err());
    }
    #[test]
    fn large_results_are_bounded_and_special_files_refused() {
        let t=tempfile::tempdir().unwrap(); let r=RepoReads::new(t.path()).unwrap();
        std::fs::write(t.path().join("large"),"x\n".repeat(MAX_FILE)).unwrap();
        assert_eq!(r.call("repo_read",&json!({"path":"large"})).unwrap()["truncated"],true);
        assert!(r.call("repo_read",&json!({"path":"."})).is_err());
        let fifo=std::ffi::CString::new(t.path().join("fifo").to_str().unwrap()).unwrap();
        assert_eq!(unsafe{libc::mkfifo(fifo.as_ptr(),0o600)},0);
        assert!(r.call("repo_read",&json!({"path":"fifo"})).is_err());
    }
}
