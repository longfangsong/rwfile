#![deny(missing_docs)]

//! A crate which provides multiple reader single writer for a file
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::{io, thread};

/// Read Guard for a file
/// Each read guard has its own file descriptor
/// and its own cursor offset
pub struct FileReader<'a> {
    file: File,
    belong_to: &'a RWFile,
}

impl<'a> Read for FileReader<'a> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.file.read(buf)
    }
}

impl<'a> Seek for FileReader<'a> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.file.seek(pos)
    }
}

impl<'a> Drop for FileReader<'a> {
    fn drop(&mut self) {
        self.belong_to.meta.lock().unwrap().reader_count -= 1;
    }
}

/// Write guard for a file
pub struct FileWriter<'a> {
    file: File,
    belong_to: &'a RWFile,
}

impl<'a> io::Write for FileWriter<'a> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.file.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

impl<'a> Drop for FileWriter<'a> {
    fn drop(&mut self) {
        self.belong_to.meta.lock().unwrap().writing = false;
    }
}

struct RWFileMeta {
    reader_count: usize,
    writing: bool,
}

/// A readable and writeable file
pub struct RWFile {
    path: PathBuf,
    meta: Mutex<RWFileMeta>,
}

impl RWFile {
    /// create a `RWFile` object at `path`
    pub fn new<P: AsRef<Path>>(path: P) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
            meta: Mutex::new(RWFileMeta {
                reader_count: 0,
                writing: false,
            }),
        }
    }

    /// create a reader for file
    pub fn reader(&self) -> FileReader {
        loop {
            let mut guard = self.meta.lock().unwrap();
            if !guard.writing {
                guard.reader_count += 1;
                break;
            }
            drop(guard);
            thread::yield_now();
        }
        FileReader {
            file: OpenOptions::new()
                .read(true)
                .open(self.path.clone())
                .unwrap(),
            belong_to: &self,
        }
    }

    /// create a writer for file
    pub fn writer(&self) -> FileWriter {
        loop {
            let mut guard = self.meta.lock().unwrap();
            if !guard.writing && guard.reader_count == 0 {
                guard.writing = true;
                break;
            }
            drop(guard);
            thread::yield_now();
        }
        FileWriter {
            file: OpenOptions::new()
                .write(true)
                .create(true)
                .open(self.path.clone())
                .unwrap(),
            belong_to: &self,
        }
    }
}

unsafe impl Sync for RWFile {}

#[cfg(test)]
mod tests {
    use crate::RWFile;
    use rand::Rng;
    use std::io::{Read, Seek, SeekFrom, Write};
    use std::sync::Arc;
    use std::thread;
    use tempfile::NamedTempFile;

    #[test]
    fn it_works() {
        let file = NamedTempFile::new().unwrap();
        let path = file.into_temp_path();
        let rwfile = Arc::new(RWFile::new(&path));
        let mut spawned = vec![];
        for _ in 0..5 {
            let rwfile = rwfile.clone();
            spawned.push(thread::spawn(move || {
                for _ in 0..1000 {
                    rwfile.writer().write_all(b"Hello world").unwrap();
                }
            }));
        }
        for _ in 0..10 {
            let rwfile = rwfile.clone();
            spawned.push(thread::spawn(move || {
                let mut rng = rand::thread_rng();
                for _ in 0..1000 {
                    let mut guard = rwfile.reader();
                    let len = guard.file.metadata().unwrap().len() / 11;
                    let offset = rng.gen_range(0, len);
                    guard.seek(SeekFrom::Start(offset)).unwrap();
                    let mut readed = [0u8; 11];
                    guard.read_exact(&mut readed).unwrap();
                    assert_eq!(&readed, b"Hello world")
                }
            }));
        }
        for s in spawned {
            s.join().unwrap();
        }
    }
}
