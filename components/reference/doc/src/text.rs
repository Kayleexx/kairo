const MAX_TEXT_BYTES: usize = 96 * 1024;
const MAX_EXTRACTED_BYTES: usize = 1024 * 1024;

#[derive(Default)]
pub(super) struct Analyzer {
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

pub(super) struct Stats {
    pub(super) lines: u64,
    pub(super) words: u64,
    pub(super) characters: u64,
    pub(super) paragraphs: u64,
}

impl Analyzer {
    pub(super) fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        let mut analyzer = Self::default();
        analyzer.push(bytes)?;
        Ok(analyzer)
    }

    pub(super) fn push(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.bytes = self
            .bytes
            .checked_add(bytes.len())
            .filter(|size| *size <= MAX_TEXT_BYTES)
            .ok_or_else(|| "text input exceeds the 96 KiB limit".to_owned())?;
        if bytes
            .iter()
            .any(|byte| *byte < b' ' && !matches!(*byte, b'\n' | b'\r' | b'\t'))
        {
            return Err("document is binary data; supported: UTF-8 text or DOCX".to_owned());
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
            self.consume(character);
        }
        self.utf8_tail.extend_from_slice(&data[valid..]);
        Ok(())
    }

    pub(super) fn finish(mut self) -> Result<Stats, String> {
        if !self.utf8_tail.is_empty() {
            return Err("document is not valid UTF-8 text".to_owned());
        }
        if self.characters > 0 && !self.ended_with_newline {
            self.lines = self.lines.saturating_add(1);
        }
        Ok(Stats {
            lines: self.lines,
            words: self.words,
            characters: self.characters,
            paragraphs: self.paragraphs,
        })
    }

    fn consume(&mut self, character: char) {
        self.characters = self.characters.saturating_add(1);
        if character.is_whitespace() {
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

pub(super) fn analyze_extracted(text: &str) -> Result<Stats, String> {
    if text.is_empty() {
        return Err("document contains no extractable text; OCR is not supported".to_owned());
    }
    if text.len() > MAX_EXTRACTED_BYTES {
        return Err("extracted document text exceeds the 1 MiB limit".to_owned());
    }
    let mut analyzer = Analyzer::default();
    for character in text.chars() {
        analyzer.consume(character);
    }
    analyzer.finish()
}
