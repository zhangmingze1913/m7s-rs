//! Size-based log rotation.
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;
pub struct RotatingWriter { path: PathBuf, max: u64, written: u64, file: File }
impl RotatingWriter {
    pub fn new(path: impl Into<PathBuf>, max: u64) -> io::Result<Self> {
        let path = path.into();
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let written = file.metadata()?.len();
        Ok(Self { path, max: max.max(1), written, file })
    }
    fn open(&mut self) -> io::Result<()> {
        let mut n = 1;
        loop {
            let p = self.path.with_extension(format!("{n}.log"));
            if !p.exists() { self.file = File::create(&p)?; self.path = p; self.written = 0; return Ok(()); }
            n += 1;
        }
    }
    pub fn write_line(&mut self, line: &str) -> io::Result<()> {
        if self.written >= self.max { self.open()?; }
        writeln!(self.file, "{line}")?;
        self.written += line.len() as u64 + 1;
        Ok(())
    }
}
