//! Log writer that redacts PII line-by-line before writing.

use std::io::{self, Write};

use super::pii::redact_text;

pub struct RedactingWriter<W: Write> {
    inner: W,
    buffer: Vec<u8>,
}

impl<W: Write> RedactingWriter<W> {
    pub fn new(inner: W) -> Self {
        Self {
            inner,
            buffer: Vec::with_capacity(1024),
        }
    }
}

impl<W: Write> Write for RedactingWriter<W> {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.buffer.extend_from_slice(data);
        let mut start = 0;
        for index in 0..self.buffer.len() {
            if self.buffer[index] == b'\n' {
                let line = String::from_utf8_lossy(&self.buffer[start..=index]);
                self.inner.write_all(redact_text(&line).as_bytes())?;
                start = index + 1;
            }
        }
        if start > 0 {
            self.buffer.drain(..start);
        }
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if !self.buffer.is_empty() {
            let line = String::from_utf8_lossy(&self.buffer);
            self.inner.write_all(redact_text(&line).as_bytes())?;
            self.buffer.clear();
        }
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writer_masks_complete_and_partial_lines() {
        let mut output = Vec::new();
        {
            let mut writer = RedactingWriter::new(&mut output);
            writer.write_all(b"call 13800138000").unwrap();
            writer.write_all(b" now\nnext\n").unwrap();
            writer.flush().unwrap();
        }
        let text = String::from_utf8(output).unwrap();
        assert!(text.contains("138****8000"));
        assert!(text.contains("next"));
    }
}
