wit_bindgen::generate!({ world: "output", path: "wit" });

const MAX_INPUT_BYTES: usize = 96 * 1024;
const MAX_CANDIDATE_BYTES: usize = 320;

struct Component;

impl Guest for Component {
    async fn output(mut input: wit_bindgen::StreamReader<u8>) -> Result<Vec<u8>, String> {
        let mut redactor = Redactor::default();
        let mut buffer = Vec::with_capacity(64 * 1024);
        loop {
            let (status, returned) = input.read(buffer).await;
            buffer = returned;
            redactor.push(&buffer)?;
            buffer.clear();
            match status {
                wit_bindgen::StreamResult::Complete(_) => {}
                wit_bindgen::StreamResult::Dropped => return redactor.finish(),
                wit_bindgen::StreamResult::Cancelled => {
                    return Err("text input stream was cancelled".to_owned());
                }
            }
        }
    }
}

#[derive(Default)]
struct Redactor {
    input_bytes: usize,
    utf8_tail: Vec<u8>,
    candidate: Vec<u8>,
    output: Vec<u8>,
    previous_cr: bool,
}

impl Redactor {
    fn push(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.input_bytes = self
            .input_bytes
            .checked_add(bytes.len())
            .filter(|size| *size <= MAX_INPUT_BYTES)
            .ok_or_else(|| "text input exceeds the 96 KiB limit".to_owned())?;
        self.utf8_tail.extend_from_slice(bytes);
        let valid = match std::str::from_utf8(&self.utf8_tail) {
            Ok(_) => self.utf8_tail.len(),
            Err(error) if error.error_len().is_none() => error.valid_up_to(),
            Err(_) => return Err("text input is not valid UTF-8".to_owned()),
        };
        let valid_bytes: Vec<u8> = self.utf8_tail.drain(..valid).collect();
        for byte in valid_bytes {
            self.push_normalized(byte)?;
        }
        Ok(())
    }

    fn finish(mut self) -> Result<Vec<u8>, String> {
        if !self.utf8_tail.is_empty() {
            return Err("text input is not valid UTF-8".to_owned());
        }
        if self.previous_cr {
            self.push_token_byte(b'\n')?;
        }
        self.flush_candidate();
        Ok(self.output)
    }

    fn push_normalized(&mut self, byte: u8) -> Result<(), String> {
        if self.previous_cr {
            self.previous_cr = false;
            if byte == b'\n' {
                return self.push_token_byte(b'\n');
            }
            self.push_token_byte(b'\n')?;
        }
        if byte == b'\r' {
            self.previous_cr = true;
            Ok(())
        } else {
            self.push_token_byte(byte)
        }
    }

    fn push_token_byte(&mut self, byte: u8) -> Result<(), String> {
        if token_byte(byte) {
            if self.candidate.len() == MAX_CANDIDATE_BYTES {
                self.flush_candidate();
            }
            self.candidate.push(byte);
        } else {
            self.flush_candidate();
            self.output.push(byte);
        }
        Ok(())
    }

    fn flush_candidate(&mut self) {
        if self.candidate.is_empty() {
            return;
        }
        if email_like(&self.candidate) || phone_like(&self.candidate) {
            self.output.extend_from_slice(b"[REDACTED]");
        } else {
            self.output.extend_from_slice(&self.candidate);
        }
        self.candidate.clear();
    }
}

fn token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'+' | b'-' | b'@')
}

fn phone_like(value: &[u8]) -> bool {
    value.len() == 10 && value.iter().all(u8::is_ascii_digit)
}

fn email_like(value: &[u8]) -> bool {
    let Some(at) = value.iter().position(|byte| *byte == b'@') else {
        return false;
    };
    at > 0
        && at + 3 < value.len()
        && value[at + 1..].contains(&b'.')
        && value.iter().filter(|byte| **byte == b'@').count() == 1
        && !value.starts_with(b".")
        && !value.ends_with(b".")
}

export!(Component);
