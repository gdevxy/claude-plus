use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::Path;

use anyhow::Result;

pub struct Line {
    pub text: String,
    pub end_offset: u64,
}

pub fn complete_lines(path: &Path, offset: u64) -> Result<impl Iterator<Item = Result<Line>>> {
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(offset))?;
    let mut reader = BufReader::new(file);
    let mut position = offset;
    Ok(std::iter::from_fn(move || {
        let mut buffer = Vec::new();
        match reader.read_until(b'\n', &mut buffer) {
            Ok(0) => None,
            Ok(_) if buffer.last() != Some(&b'\n') => None,
            Ok(read) => {
                position += read as u64;
                buffer.pop();
                if buffer.last() == Some(&b'\r') {
                    buffer.pop();
                }
                Some(Ok(Line {
                    text: String::from_utf8_lossy(&buffer).into_owned(),
                    end_offset: position,
                }))
            }
            Err(error) => Some(Err(error.into())),
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn yields_only_complete_lines_from_offset() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(b"one\ntwo\nthr").unwrap();
        let lines: Vec<(String, u64)> = complete_lines(file.path(), 0)
            .unwrap()
            .map(|l| l.unwrap())
            .map(|l| (l.text, l.end_offset))
            .collect();
        assert_eq!(lines, vec![("one".to_string(), 4), ("two".to_string(), 8)]);
        let rest: Vec<String> = complete_lines(file.path(), 4)
            .unwrap()
            .map(|l| l.unwrap().text)
            .collect();
        assert_eq!(rest, vec!["two"]);
    }

    #[test]
    fn invalid_utf8_is_replaced() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(b"a\xffb\n").unwrap();
        let lines: Vec<String> = complete_lines(file.path(), 0)
            .unwrap()
            .map(|l| l.unwrap().text)
            .collect();
        assert_eq!(lines, vec!["a\u{fffd}b"]);
    }

    #[test]
    fn strips_carriage_returns() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(b"x\r\n").unwrap();
        let lines: Vec<String> = complete_lines(file.path(), 0)
            .unwrap()
            .map(|l| l.unwrap().text)
            .collect();
        assert_eq!(lines, vec!["x"]);
    }
}
