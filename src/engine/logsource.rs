//! Reads complete lines from a live log file starting at its checkpoint, or from a
//! gzip archive (backfill) — `WebPulse\Log\LogSource`.
//!
//! Checkpoint rules (live files):
//! * no checkpoint → start at 0
//! * inode changed → start at 0 of the new file (`inode`)
//! * size < stored offset → file truncated, start at 0 (`truncated`)
//! * first bytes differ from the stored fingerprint (`len:sha1`) → `replaced`
//! * otherwise → continue at the stored offset
//!
//! The file is read in 1 MiB chunks, never as a whole; a trailing line without
//! `\n` is left for the next run. [`LogSource::offset`] only advances over lines
//! handed out, so the caller stores it after its own transaction commits. Files are
//! opened non-blocking and must be regular files (a FIFO cannot hang the collector);
//! archives must also keep the device/inode seen by `lstat()`. Archives are
//! decompressed in 8 KiB input slices with 1 MiB output reads, so a decompression
//! bomb stays in bounded memory.

use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::os::unix::fs::{FileExt, MetadataExt};
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use flate2::bufread::GzDecoder;
use sha1::{Digest, Sha1};

use crate::util::fs;

/// Bytes read per chunk.
pub const CHUNK: usize = 1_048_576;
/// Bytes hashed for the rotation fingerprint.
pub const FINGERPRINT_BYTES: u64 = 256;
/// A "line" longer than two chunks is cut to this length (and the rest skipped).
pub const MAX_LINE: usize = 65536;
/// Compressed bytes handed to the inflater at a time.
pub const INFLATE_SLICE: usize = 8192;

/// The stored checkpoint of a live log (`checkpoints` row).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Checkpoint {
    /// Inode of the file at the last run.
    pub inode: i64,
    /// Bytes consumed.
    pub offset: i64,
    /// Size at the last run.
    pub size: i64,
    /// `len:sha1` of the first bytes.
    pub first_line: Option<String>,
    /// Timestamp of the last processed line.
    pub last_ts: Option<i64>,
}

struct GzStream {
    dec: Option<GzDecoder<BufReader<File>>>,
    done: bool,
    /// The current member starts with the gzip magic (a truncated member is then
    /// treated as the end, like zlib's streaming inflate; garbage is corrupt).
    magic: bool,
}

/// True when the buffered input starts like a gzip member (`1f 8b`).
fn starts_with_magic(r: &mut BufReader<File>) -> bool {
    match r.fill_buf() {
        Ok(b) => !b.is_empty() && b[0] == 0x1f && (b.len() < 2 || b[1] == 0x8b),
        Err(_) => false,
    }
}

impl GzStream {
    fn new(f: File) -> GzStream {
        let mut r = BufReader::with_capacity(INFLATE_SLICE, f);
        let magic = starts_with_magic(&mut r);
        GzStream {
            dec: Some(GzDecoder::new(r)),
            done: false,
            magic,
        }
    }

    /// Up to `CHUNK` decompressed bytes; `None` at the end of the last member.
    fn read_chunk(&mut self, path: &Path) -> Result<Option<Vec<u8>>> {
        let corrupt = || anyhow!("corrupt gzip data in {}", path.display());
        let mut out = vec![0u8; CHUNK];
        let mut n = 0;
        while n < CHUNK && !self.done {
            let Some(dec) = self.dec.as_mut() else {
                self.done = true;
                break;
            };
            match dec.read(&mut out[n..]) {
                Ok(0) => {
                    // end of a member: zero padding or EOF ends the stream, anything else is the next member
                    let mut inner = match self.dec.take() {
                        Some(d) => d.into_inner(),
                        None => break,
                    };
                    loop {
                        let buf = inner.fill_buf().map_err(|_| corrupt())?;
                        if buf.is_empty() {
                            self.done = true;
                            break;
                        }
                        match buf.iter().position(|&b| b != 0) {
                            Some(0) => {
                                self.magic = starts_with_magic(&mut inner);
                                self.dec = Some(GzDecoder::new(inner));
                                break;
                            }
                            Some(p) => inner.consume(p),
                            None => {
                                let l = buf.len();
                                inner.consume(l);
                            }
                        }
                    }
                }
                Ok(k) => n += k,
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof && self.magic => {
                    // truncated archive (still being written): keep what was inflated
                    self.done = true;
                }
                Err(_) => return Err(corrupt()),
            }
        }
        out.truncate(n);
        Ok(if n == 0 { None } else { Some(out) })
    }
}

/// A log file being read.
pub struct LogSource {
    path: PathBuf,
    https: bool,
    checkpoint: Option<Checkpoint>,
    gzip: bool,
    file: Option<File>,
    gz: Option<GzStream>,
    buf: Vec<u8>,
    pos: usize,
    eof: bool,
    skipping: bool,
    offset: u64,
    start: u64,
    inode: u64,
    size: u64,
    mtime: i64,
    reset_reason: Option<&'static str>,
}

impl std::fmt::Debug for LogSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LogSource")
            .field("path", &self.path)
            .field("offset", &self.offset)
            .finish()
    }
}

fn sha1_hex(b: &[u8]) -> String {
    fs::hex(&Sha1::digest(b))
}

impl LogSource {
    /// A live log (`gzip == false`, with its checkpoint) or an archive.
    pub fn new(path: &Path, https: bool, checkpoint: Option<Checkpoint>, gzip: bool) -> LogSource {
        LogSource {
            path: path.to_path_buf(),
            https,
            checkpoint,
            gzip,
            file: None,
            gz: None,
            buf: Vec::new(),
            pos: 0,
            eof: false,
            skipping: false,
            offset: 0,
            start: 0,
            inode: 0,
            size: 0,
            mtime: 0,
            reset_reason: None,
        }
    }

    /// Path as discovered.
    pub fn path(&self) -> &Path {
        &self.path
    }
    /// True for `-ssl_log` files.
    pub fn https(&self) -> bool {
        self.https
    }
    /// True for archives.
    pub fn is_gzip(&self) -> bool {
        self.gzip
    }

    /// Opens the file and positions it according to the checkpoint.
    pub fn open(&mut self) -> Result<()> {
        let p = self.path.display().to_string();
        let lst = std::fs::symlink_metadata(&self.path).map_err(|_| anyhow!("cannot stat {p}"))?;
        if self.gzip && !lst.file_type().is_file() {
            return Err(anyhow!("{p} is not a regular file"));
        }
        let f = fs::open_nonblocking(&self.path).map_err(|_| anyhow!("cannot open {p}"))?;
        let st = f
            .metadata()
            .map_err(|_| anyhow!("{p} is not a regular file"))?;
        if !st.file_type().is_file() {
            return Err(anyhow!("{p} is not a regular file"));
        }
        if self.gzip && (st.ino() != lst.ino() || st.dev() != lst.dev()) {
            return Err(anyhow!("{p} changed while opening"));
        }
        self.inode = st.ino();
        self.size = st.len();
        self.mtime = st.mtime();
        if self.gzip {
            self.gz = Some(GzStream::new(f));
            return Ok(());
        }
        self.file = Some(f);
        let mut start = 0u64;
        if let Some(cp) = self.checkpoint.clone() {
            let off = cp.offset;
            if cp.inode != self.inode as i64 {
                self.reset_reason = Some("inode");
            } else if (self.size as i64) < off {
                self.reset_reason = Some("truncated");
            } else if !self.fingerprint_matches(cp.first_line.as_deref().unwrap_or("")) {
                self.reset_reason = Some("replaced");
            } else {
                start = off.max(0) as u64;
            }
        }
        if start > 0
            && let Some(f) = self.file.as_mut()
            && f.seek(SeekFrom::Start(start)).is_err()
        {
            start = 0;
            self.reset_reason = Some("seek");
            let _ = f.seek(SeekFrom::Start(0));
        }
        self.start = start;
        self.offset = start;
        Ok(())
    }

    /// Why the stored checkpoint was discarded (inode | truncated | replaced | seek).
    pub fn reset_reason(&self) -> Option<&'static str> {
        self.reset_reason
    }
    /// Offset the read started from.
    pub fn start_offset(&self) -> u64 {
        self.start
    }
    /// Bytes consumed up to the end of the last line returned.
    pub fn offset(&self) -> u64 {
        self.offset
    }
    /// Inode at open time.
    pub fn inode(&self) -> u64 {
        self.inode
    }
    /// Size at open time (compressed size for archives).
    pub fn size(&self) -> u64 {
        self.size
    }
    /// Modification time at open time.
    pub fn mtime(&self) -> i64 {
        self.mtime
    }

    /// `"<len>:<sha1 of the first len bytes>"` with len = min(256, current size);
    /// read with pread (the position is preserved). Empty for archives.
    pub fn fingerprint(&self) -> String {
        let Some(f) = self.file.as_ref().filter(|_| !self.gzip) else {
            return String::new();
        };
        let size = f.metadata().map(|m| m.len()).unwrap_or(0);
        let len = FINGERPRINT_BYTES.min(size) as usize;
        if len == 0 {
            return "0:".into();
        }
        let mut head = vec![0u8; len];
        let n = read_at_full(f, &mut head);
        head.truncate(n);
        format!("{}:{}", head.len(), sha1_hex(&head))
    }

    fn fingerprint_matches(&self, stored: &str) -> bool {
        let Some((len, hash)) = stored.split_once(':') else {
            return true; // no fingerprint recorded: trust inode + size
        };
        let len = crate::config::php_str_int(len);
        if len <= 0 {
            return true;
        }
        if (self.size as i64) < len {
            return false;
        }
        let Some(f) = self.file.as_ref() else {
            return false;
        };
        let mut head = vec![0u8; len as usize];
        let n = read_at_full(f, &mut head);
        head.truncate(n);
        sha1_hex(&head) == hash
    }

    fn read_plain(&mut self) -> Result<Option<Vec<u8>>> {
        let Some(f) = self.file.as_mut() else {
            return Ok(None);
        };
        let mut chunk = vec![0u8; CHUNK];
        let mut n = 0;
        while n < CHUNK {
            match f.read(&mut chunk[n..]) {
                Ok(0) => break,
                Ok(k) => n += k,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => return Err(anyhow!("read error on {}", self.path.display())),
            }
        }
        chunk.truncate(n);
        Ok(if n == 0 { None } else { Some(chunk) })
    }

    /// Copies the next complete line (without its terminator) into `out`; false when
    /// no complete line is left. Errors on read / decompression failures.
    pub fn next_line(&mut self, out: &mut Vec<u8>) -> Result<bool> {
        loop {
            if let Some(nl) = memchr::memchr(b'\n', &self.buf[self.pos..]) {
                let start = self.pos;
                let end = start + nl;
                self.offset += (nl + 1) as u64;
                self.pos = end + 1;
                if self.skipping {
                    self.skipping = false;
                    continue;
                }
                let mut line = &self.buf[start..end];
                if line.last() == Some(&b'\r') {
                    line = &line[..line.len() - 1];
                }
                out.clear();
                out.extend_from_slice(line);
                return Ok(true);
            }
            if self.pos > 0 {
                self.buf.drain(..self.pos);
                self.pos = 0;
            }
            if self.buf.len() >= 2 * CHUNK {
                // absurdly long line: hand out a cut version once, skip the rest
                let was = self.skipping;
                self.offset += self.buf.len() as u64;
                self.skipping = true;
                if !was {
                    out.clear();
                    out.extend_from_slice(&self.buf[..MAX_LINE]);
                    self.buf.clear();
                    return Ok(true);
                }
                self.buf.clear();
                continue;
            }
            if self.eof {
                if self.gzip && !self.buf.is_empty() {
                    // archives: the last line may lack "\n"
                    let mut end = self.buf.len();
                    while end > 0 && self.buf[end - 1] == b'\r' {
                        end -= 1;
                    }
                    self.offset += self.buf.len() as u64;
                    let skipping = self.skipping;
                    if !skipping {
                        out.clear();
                        out.extend_from_slice(&self.buf[..end]);
                    }
                    self.buf.clear();
                    return Ok(!skipping);
                }
                return Ok(false);
            }
            let chunk = if self.gzip {
                let path = self.path.clone();
                match self.gz.as_mut() {
                    Some(g) => g.read_chunk(&path)?,
                    None => None,
                }
            } else {
                self.read_plain()?
            };
            match chunk {
                None => self.eof = true,
                Some(c) => {
                    if self.buf.is_empty() {
                        self.buf = c;
                    } else {
                        self.buf.extend_from_slice(&c);
                    }
                }
            }
        }
    }

    /// Next line as an owned vector (convenience for tests and tools).
    pub fn read_line(&mut self) -> Result<Option<Vec<u8>>> {
        let mut v = Vec::new();
        Ok(if self.next_line(&mut v)? {
            Some(v)
        } else {
            None
        })
    }

    /// Closes the file.
    pub fn close(&mut self) {
        self.file = None;
        self.gz = None;
    }
}

fn read_at_full(f: &File, buf: &mut [u8]) -> usize {
    let mut n = 0;
    while n < buf.len() {
        match f.read_at(&mut buf[n..], n as u64) {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
    n
}
