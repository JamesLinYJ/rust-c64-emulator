// +-------------------------------------------------------------------------
//
//   Rust C64 Emulator - bounded JSON trace request reader
//
//   File:       json_line.rs
//
//   Created:    2026-08-12
//   Author:     OpenAI Codex
// --------------------------------------------------------------------------

use std::error::Error;
use std::io::{self, BufRead};

fn read_json_line<T, R>(reader: &mut R) -> Result<T, Box<dyn Error>>
where
    T: serde::de::DeserializeOwned,
    R: BufRead,
{
    let mut input = String::new();
    if reader.read_line(&mut input)? == 0 {
        return Err(
            io::Error::new(io::ErrorKind::UnexpectedEof, "missing JSON trace request").into(),
        );
    }
    Ok(serde_json::from_str(&input)?)
}

pub fn read_stdin_json_line<T>() -> Result<T, Box<dyn Error>>
where
    T: serde::de::DeserializeOwned,
{
    read_json_line(&mut io::stdin().lock())
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, Cursor};

    use super::read_json_line;

    #[test]
    fn framed_request_does_not_wait_for_end_of_stream() {
        let mut input = Cursor::new(b"[1,2,3]\ntrailing stream data".as_slice());

        let values: Vec<u8> = read_json_line(&mut input).unwrap();

        assert_eq!(values, [1, 2, 3]);
        assert_eq!(input.fill_buf().unwrap(), b"trailing stream data");
    }
}
