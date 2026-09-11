wit_bindgen::generate!({ world: "consume-metrics", path: "wit" });

const MAX_INPUT: usize = 96 * 1024;

struct Component;

impl Guest for Component {
    async fn consume(mut input: wit_bindgen::StreamReader<u8>) -> Result<Vec<Metric>, String> {
        let mut analyzer = Analyzer::default();
        let mut buffer = Vec::with_capacity(64 * 1024);
        loop {
            let (status, returned) = input.read(buffer).await;
            buffer = returned;
            analyzer.push(&buffer)?;
            buffer.clear();
            match status {
                wit_bindgen::StreamResult::Complete(_) => {}
                wit_bindgen::StreamResult::Dropped => return analyzer.finish(),
                wit_bindgen::StreamResult::Cancelled => {
                    return Err("document input stream was cancelled".to_owned());
                }
            }
        }
    }
}

#[derive(Default)]
struct Analyzer {
    prefix: Vec<u8>,
    prefix_checked: bool,
    utf8_tail: Vec<u8>,
    bytes: usize,
    lines: u64,
    words: u64,
    characters: u64,
    paragraphs: u64,
    in_word: bool,
    in_paragraph: bool,
    line_has_text: bool,
    ended_with_newline: bool,
}

impl Analyzer {
    fn push(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.bytes = self
            .bytes
            .checked_add(bytes.len())
            .filter(|size| *size <= MAX_INPUT)
            .ok_or_else(|| "document input exceeds the 96 KiB limit".to_owned())?;
        let needed = if self.prefix_checked {
            0
        } else {
            8_usize.saturating_sub(self.prefix.len())
        };
        let split = needed.min(bytes.len());
        self.prefix.extend_from_slice(&bytes[..split]);
        if self.prefix.len() == 8 {
            self.consume_prefix()?;
        }
        self.consume_utf8(&bytes[split..])
    }

    fn finish(mut self) -> Result<Vec<Metric>, String> {
        self.consume_prefix()?;
        if !self.utf8_tail.is_empty() {
            return Err("document is not valid UTF-8 text".to_owned());
        }
        if self.characters > 0 && !self.ended_with_newline {
            self.lines = self.lines.saturating_add(1);
        }
        Ok(vec![
            Metric {
                name: "lines".to_owned(),
                value: self.lines,
            },
            Metric {
                name: "words".to_owned(),
                value: self.words,
            },
            Metric {
                name: "characters".to_owned(),
                value: self.characters,
            },
            Metric {
                name: "paragraphs".to_owned(),
                value: self.paragraphs,
            },
        ])
    }

    fn consume_prefix(&mut self) -> Result<(), String> {
        if self.prefix_checked {
            return Ok(());
        }
        self.prefix_checked = true;
        if self.prefix.starts_with(b"%PDF-") {
            return Err("PDF text extraction is not enabled in this build".to_owned());
        }
        if self.prefix.starts_with(b"PK\x03\x04") {
            return Err("DOCX text extraction is not enabled in this build".to_owned());
        }
        let prefix = std::mem::take(&mut self.prefix);
        self.consume_utf8(&prefix)
    }

    fn consume_utf8(&mut self, bytes: &[u8]) -> Result<(), String> {
        if bytes
            .iter()
            .any(|byte| *byte < b' ' && !matches!(*byte, b'\n' | b'\r' | b'\t'))
        {
            return Err("document is binary data; supported: UTF-8 text".to_owned());
        }
        self.utf8_tail.extend_from_slice(bytes);
        let data = std::mem::take(&mut self.utf8_tail);
        let valid = match std::str::from_utf8(&data) {
            Ok(_) => data.len(),
            Err(error) if error.error_len().is_none() => error.valid_up_to(),
            Err(_) => return Err("document is not valid UTF-8 text".to_owned()),
        };
        let text = std::str::from_utf8(&data[..valid])
            .map_err(|_| "document is not valid UTF-8 text".to_owned())?;
        for character in text.chars() {
            self.consume_character(character);
        }
        self.utf8_tail.extend_from_slice(&data[valid..]);
        Ok(())
    }

    fn consume_character(&mut self, character: char) {
        self.characters = self.characters.saturating_add(1);
        let whitespace = character.is_whitespace();
        if whitespace {
            self.in_word = false;
        } else {
            if !self.in_word {
                self.words = self.words.saturating_add(1);
            }
            self.in_word = true;
            self.line_has_text = true;
            if !self.in_paragraph {
                self.paragraphs = self.paragraphs.saturating_add(1);
                self.in_paragraph = true;
            }
        }
        self.ended_with_newline = character == '\n';
        if self.ended_with_newline {
            self.lines = self.lines.saturating_add(1);
            if !self.line_has_text {
                self.in_paragraph = false;
            }
            self.line_has_text = false;
        }
    }
}

export!(Component);
