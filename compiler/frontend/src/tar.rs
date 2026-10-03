use diagnostics::{Code, Diagnostic};
use std::io::Write;

pub struct TarWriter<W: Write> {
    writer: W,
}

fn octal_into(buf: &mut [u8; 512], off: usize, len: usize, value: u64) {
    let text = format!("{:o}", value);
    let digits = text.as_bytes();
    let pad = len - 1 - digits.len();
    for i in 0..pad {
        buf[off + i] = b'0';
    }
    buf[off + pad..off + pad + digits.len()].copy_from_slice(digits);
    buf[off + len - 1] = 0;
}

impl<W: Write> TarWriter<W> {
    pub fn new(writer: W) -> TarWriter<W> {
        TarWriter { writer }
    }

    fn header(&mut self, rel: &str, size: u64, dir: bool) -> Result<(), Diagnostic> {
        let bytes = rel.as_bytes();
        if bytes.len() > 100 || bytes.contains(&0) {
            return Err(Diagnostic::new(
                Code::E108,
                format!("tar entry name too long: `{rel}`"),
            ));
        }
        let mut head = [0u8; 512];
        head[0..bytes.len()].copy_from_slice(bytes);
        let mode = if dir { 0o755 } else { 0o644 };
        octal_into(&mut head, 100, 8, mode);
        octal_into(&mut head, 108, 8, 0);
        octal_into(&mut head, 116, 8, 0);
        octal_into(&mut head, 124, 12, size);
        octal_into(&mut head, 136, 12, 0);
        for b in head.iter_mut().take(156).skip(148) {
            *b = b' ';
        }
        head[156] = if dir { b'5' } else { b'0' };
        head[257..263].copy_from_slice(b"ustar\0");
        head[263..265].copy_from_slice(b"00");
        head[265..268].copy_from_slice(b"rnx");
        head[297..300].copy_from_slice(b"rnx");
        let sum: u32 = head.iter().map(|b| *b as u32).sum();
        let text = format!("{:06o}", sum);
        head[148..154].copy_from_slice(text.as_bytes());
        head[154] = 0;
        head[155] = b' ';
        self.writer.write_all(&head).map_err(|e| {
            Diagnostic::new(Code::E108, format!("cannot write tar header: {e}"))
        })?;
        Ok(())
    }

    pub fn add_dir(&mut self, rel_path: &str) -> Result<(), Diagnostic> {
        let name =
            if rel_path.ends_with('/') { rel_path.to_string() } else { format!("{rel_path}/") };
        self.header(&name, 0, true)
    }

    pub fn add_file(&mut self, rel_path: &str, content: &[u8]) -> Result<(), Diagnostic> {
        self.header(rel_path, content.len() as u64, false)?;
        self.writer.write_all(content).map_err(|e| {
            Diagnostic::new(Code::E108, format!("cannot write tar body: {e}"))
        })?;
        let pad = (512 - content.len() % 512) % 512;
        if pad > 0 {
            self.writer.write_all(&vec![0u8; pad]).map_err(|e| {
                Diagnostic::new(Code::E108, format!("cannot write tar padding: {e}"))
            })?;
        }
        Ok(())
    }

    pub fn finish(mut self) -> Result<(), Diagnostic> {
        self.writer.write_all(&[0u8; 1024]).map_err(|e| {
            Diagnostic::new(Code::E108, format!("cannot write tar trailer: {e}"))
        })?;
        Ok(())
    }
}
