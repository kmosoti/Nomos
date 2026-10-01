//! The durable log: one append-only file of records (ADR 0017 §3).
//!
//! A record is one appended batch:
//!
//! | Bytes | Content |
//! | --- | --- |
//! | 4 | The payload's length, little-endian |
//! | 4 | The first four bytes of the SHA-256 digest of the length |
//! | the length | The payload: a 4-byte count, then each value as a 4-byte length and its bytes |
//! | 32 | The SHA-256 digest of the length and the payload |
//!
//! A record is written with one `write` and one `fdatasync` before `append`
//! returns, so an acknowledged batch is on disk, and a batch interrupted by
//! a crash is at most a record cut short at the end of the file. The check
//! of the length tells the two apart: a record whose header is whole and
//! checks, and whose payload runs past the end of the file, was cut short;
//! a header that does not check is corruption, wherever it is, so a
//! damaged length never makes whole records look like a torn tail.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use nomos_canon::sha256;
use nomos_store::{EventLog, Full, Record};
use rustix::fs::OFlags;

use crate::state::{self, Kind, StateError};

/// Why a log could not be opened. The Cell does not start on any of these:
/// skipping a record would forget what it recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogError {
    /// A record before the last is malformed, or the last one is whole but
    /// its digest does not match, at this byte offset.
    Corrupt {
        /// Where the record starts.
        offset: u64,
    },
    /// A record is well formed but a value in it does not decode.
    Undecodable {
        /// Where the record starts.
        offset: u64,
    },
    /// The file is not private to the Cell: a link, not a regular file, owned
    /// by another user, or open to group or others. It is not read.
    Unsafe(String),
    /// The file could not be read or repaired.
    Io(String),
}

impl std::fmt::Display for LogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LogError::Corrupt { offset } => write!(f, "the log is corrupt at byte {offset}"),
            LogError::Undecodable { offset } => {
                write!(f, "the log's record at byte {offset} does not decode")
            }
            LogError::Unsafe(e) => write!(f, "the log: {e}"),
            LogError::Io(e) => write!(f, "the log: {e}"),
        }
    }
}

impl std::error::Error for LogError {}

/// What opening the log found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Recovery {
    /// How many records were read.
    pub records: usize,
    /// How many bytes of a final record cut short were truncated away.
    pub truncated: u64,
}

/// A durable log of `E` in one file.
#[derive(Debug)]
pub struct FileLog<E> {
    file: File,
    values: Vec<E>,
    length: u64,
    /// Set when a failed append could not be undone; every later append is
    /// refused, since writing after a partial record would corrupt the log.
    poisoned: bool,
}

/// What a read-only look at the log found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inspection<E> {
    /// Every value of every whole record.
    pub values: Vec<E>,
    /// How many whole records there were.
    pub records: usize,
    /// How many bytes at the end are a record cut short, left in place.
    pub torn: u64,
}

/// A log read back and never written: the journal as a read-only command
/// sees it. Every append is refused.
#[derive(Debug, Clone)]
pub struct Replayed<E>(Vec<E>);

impl<E> Replayed<E> {
    /// The log holding `values`.
    pub fn new(values: Vec<E>) -> Self {
        Replayed(values)
    }
}

impl<E: Clone> EventLog<E> for Replayed<E> {
    fn append(&mut self, _: &[E]) -> Result<(), Full> {
        Err(Full)
    }

    fn events(&self) -> Vec<E> {
        self.0.clone()
    }
}

/// `path` opened for the log, without following a link, and checked to be
/// private to the Cell. `None` when it is absent and `create` is not set.
fn open_checked(
    path: &Path,
    options: &mut OpenOptions,
    create: bool,
) -> Result<Option<File>, LogError> {
    options
        .custom_flags((OFlags::NOFOLLOW | OFlags::CLOEXEC).bits() as i32)
        .mode(0o600);
    let file = match options.open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && !create => return Ok(None),
        Err(e)
            if e.raw_os_error()
                .is_some_and(|n| state::is_link(rustix::io::Errno::from_raw_os_error(n))) =>
        {
            return Err(LogError::Unsafe(
                StateError::Unsafe {
                    path: path.to_path_buf(),
                    why: "a symbolic link",
                }
                .to_string(),
            ));
        }
        Err(e) => return Err(LogError::Io(e.to_string())),
    };
    state::check(&file, path, Kind::File).map_err(|e| match e {
        StateError::Io(m) => LogError::Io(m),
        other => LogError::Unsafe(other.to_string()),
    })?;
    Ok(Some(file))
}

fn digest(length: &[u8], payload: &[u8]) -> [u8; 32] {
    let mut h = sha256::Sha256::new();
    h.update(length);
    h.update(payload);
    h.finish()
}

fn length_check(length: &[u8]) -> [u8; 4] {
    let d = sha256::digest(length);
    [d[0], d[1], d[2], d[3]]
}

/// The bytes before a record's payload.
const HEADER: usize = 8;
/// The bytes a record adds to its payload.
const OVERHEAD: usize = HEADER + 32;

/// The record of `values`.
pub fn record(values: &[Vec<u8>]) -> Vec<u8> {
    let mut payload = Vec::new();
    payload.extend_from_slice(&(values.len() as u32).to_le_bytes());
    for v in values {
        payload.extend_from_slice(&(v.len() as u32).to_le_bytes());
        payload.extend_from_slice(v);
    }
    let length = (payload.len() as u32).to_le_bytes();
    let mut out = Vec::with_capacity(payload.len() + OVERHEAD);
    out.extend_from_slice(&length);
    out.extend_from_slice(&length_check(&length));
    out.extend_from_slice(&payload);
    out.extend_from_slice(&digest(&length, &payload));
    out
}

fn le32(bytes: &[u8]) -> Option<usize> {
    Some(u32::from_le_bytes(bytes.get(..4)?.try_into().ok()?) as usize)
}

/// The values of a payload, or `None` when it is not a well-formed one.
fn values(payload: &[u8]) -> Option<Vec<&[u8]>> {
    let count = le32(payload)?;
    let mut at = 4;
    let mut out = Vec::new();
    for _ in 0..count {
        let len = le32(payload.get(at..)?)?;
        at += 4;
        out.push(payload.get(at..at.checked_add(len)?)?);
        at += len;
    }
    (at == payload.len()).then_some(out)
}

/// How a scan of the file ends.
enum Tail {
    /// Every byte is part of a whole record.
    Clean,
    /// A final record is cut short from this offset.
    Torn(u64),
}

/// The records of `bytes`, decoded, and how the file ends.
fn scan<E: Record>(bytes: &[u8]) -> Result<(Vec<E>, usize, Tail), LogError> {
    let mut at = 0usize;
    let mut out = Vec::new();
    let mut records = 0;
    while at < bytes.len() {
        let offset = at as u64;
        let rest = &bytes[at..];
        if rest.len() < HEADER {
            return Ok((out, records, Tail::Torn(offset)));
        }
        if rest[4..HEADER] != length_check(&rest[..4]) {
            return Err(LogError::Corrupt { offset });
        }
        let len = le32(rest).ok_or(LogError::Corrupt { offset })?;
        let Some(end) = len.checked_add(OVERHEAD) else {
            return Err(LogError::Corrupt { offset });
        };
        if rest.len() < end {
            return Ok((out, records, Tail::Torn(offset)));
        }
        let (length, payload, sum) = (
            &rest[..4],
            &rest[HEADER..HEADER + len],
            &rest[HEADER + len..end],
        );
        if digest(length, payload) != sum {
            return Err(LogError::Corrupt { offset });
        }
        let vals = values(payload).ok_or(LogError::Corrupt { offset })?;
        for v in vals {
            out.push(E::decode(v).ok_or(LogError::Undecodable { offset })?);
        }
        records += 1;
        at += end;
    }
    Ok((out, records, Tail::Clean))
}

impl<E: Record + Clone> FileLog<E> {
    /// The log at `path`, created if it does not exist. A final record cut
    /// short is truncated away and reported; anything else malformed fails.
    pub fn open(path: &Path) -> Result<(Self, Recovery), LogError> {
        let io = |e: std::io::Error| LogError::Io(e.to_string());
        let mut file = open_checked(
            path,
            OpenOptions::new().read(true).append(true).create(true),
            true,
        )?
        .ok_or_else(|| LogError::Io("absent after creation".into()))?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).map_err(io)?;
        let (values, records, tail) = scan::<E>(&bytes)?;
        let mut length = bytes.len() as u64;
        let mut truncated = 0;
        if let Tail::Torn(offset) = tail {
            file.set_len(offset).map_err(io)?;
            file.sync_data().map_err(io)?;
            truncated = length - offset;
            length = offset;
        }
        file.seek(SeekFrom::End(0)).map_err(io)?;
        Ok((
            FileLog {
                file,
                values,
                length,
                poisoned: false,
            },
            Recovery { records, truncated },
        ))
    }
}

impl<E: Record> FileLog<E> {
    /// The log at `path` read and never written: not created if absent
    /// (`None`), and a final record cut short is reported, not truncated.
    /// Anything else malformed fails, as it does in [`FileLog::open`].
    pub fn inspect(path: &Path) -> Result<Option<Inspection<E>>, LogError> {
        let Some(mut file) = open_checked(path, OpenOptions::new().read(true), false)? else {
            return Ok(None);
        };
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .map_err(|e| LogError::Io(e.to_string()))?;
        let (values, records, tail) = scan::<E>(&bytes)?;
        let torn = match tail {
            Tail::Clean => 0,
            Tail::Torn(offset) => bytes.len() as u64 - offset,
        };
        Ok(Some(Inspection {
            values,
            records,
            torn,
        }))
    }
}

impl<E: Record + Clone> EventLog<E> for FileLog<E> {
    fn append(&mut self, batch: &[E]) -> Result<(), Full> {
        if self.poisoned {
            return Err(Full);
        }
        let encoded: Vec<Vec<u8>> = batch.iter().map(Record::encode).collect();
        let bytes = record(&encoded);
        let written = self
            .file
            .write_all(&bytes)
            .and_then(|()| self.file.sync_data());
        match written {
            Ok(()) => {
                self.length += bytes.len() as u64;
                self.values.extend(batch.iter().cloned());
                Ok(())
            }
            Err(_) => {
                // Undo a partial record, so that the next append does not
                // follow it; if that fails too, refuse every later append.
                let undone = self
                    .file
                    .set_len(self.length)
                    .and_then(|()| self.file.sync_data());
                if undone.is_err() {
                    self.poisoned = true;
                }
                Err(Full)
            }
        }
    }

    fn events(&self) -> Vec<E> {
        self.values.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Text(String);

    impl Record for Text {
        fn encode(&self) -> Vec<u8> {
            self.0.as_bytes().to_vec()
        }
        fn decode(bytes: &[u8]) -> Option<Self> {
            let s = String::from_utf8(bytes.to_vec()).ok()?;
            (!s.contains('!')).then_some(Text(s))
        }
    }

    fn t(s: &str) -> Text {
        Text(s.into())
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        crate::scratch::dir("log", name).join("journal")
    }

    /// What is appended is read back after reopening, batch for batch.
    #[test]
    fn appended_batches_survive_a_reopen() {
        let path = scratch("reopen");
        let (mut log, r) = FileLog::<Text>::open(&path).unwrap();
        assert_eq!(
            r,
            Recovery {
                records: 0,
                truncated: 0
            }
        );
        log.append(&[t("a"), t("b")]).unwrap();
        log.append(&[]).unwrap();
        log.append(&[t("c")]).unwrap();
        drop(log);
        let (log, r) = FileLog::<Text>::open(&path).unwrap();
        assert_eq!(
            r,
            Recovery {
                records: 3,
                truncated: 0
            }
        );
        assert_eq!(log.events(), vec![t("a"), t("b"), t("c")]);
    }

    /// ADR 0017 acceptance: a final record cut short at any byte is
    /// truncated and reported, and the log keeps every whole record before
    /// it and accepts appends after.
    #[test]
    fn a_torn_final_record_is_truncated_at_every_length() {
        let path = scratch("torn");
        let (mut log, _) = FileLog::<Text>::open(&path).unwrap();
        log.append(&[t("kept")]).unwrap();
        drop(log);
        let whole = std::fs::read(&path).unwrap();
        let next = record(&[b"lost".to_vec(), b"too".to_vec()]);
        for cut in 1..next.len() {
            let mut bytes = whole.clone();
            bytes.extend_from_slice(&next[..cut]);
            std::fs::write(&path, &bytes).unwrap();
            let (mut log, r) = FileLog::<Text>::open(&path).unwrap();
            assert_eq!(r.truncated, cut as u64, "cut at {cut}");
            assert_eq!(log.events(), vec![t("kept")]);
            log.append(&[t("after")]).unwrap();
            drop(log);
            let (log, _) = FileLog::<Text>::open(&path).unwrap();
            assert_eq!(log.events(), vec![t("kept"), t("after")], "cut at {cut}");
        }
    }

    /// ADR 0017 acceptance: a changed byte anywhere in a whole record, or
    /// a value that does not decode, fails the open.
    #[test]
    fn a_corrupted_record_fails_the_open() {
        let path = scratch("corrupt");
        let (mut log, _) = FileLog::<Text>::open(&path).unwrap();
        log.append(&[t("first")]).unwrap();
        log.append(&[t("second")]).unwrap();
        drop(log);
        let good = std::fs::read(&path).unwrap();
        let first = record(&[b"first".to_vec()]).len();
        for at in 0..good.len() {
            let mut bytes = good.clone();
            bytes[at] ^= 0x01;
            std::fs::write(&path, &bytes).unwrap();
            let opened = FileLog::<Text>::open(&path).map(|(l, r)| (l.events(), r));
            let offset = if at < first { 0 } else { first as u64 };
            match opened {
                Err(LogError::Corrupt { offset: o }) => assert_eq!(o, offset, "byte {at}"),
                Ok(_) => panic!("byte {at}: a changed log was opened"),
                Err(e) => panic!("byte {at}: {e}"),
            }
        }
        std::fs::write(&path, record(&[b"no!".to_vec()])).unwrap();
        assert_eq!(
            FileLog::<Text>::open(&path).err(),
            Some(LogError::Undecodable { offset: 0 })
        );
    }
}
