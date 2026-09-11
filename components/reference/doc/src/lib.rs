wit_bindgen::generate!({ world: "consume-metrics", path: "wit" });

mod docx;
mod text;

const MAX_CONTAINER_BYTES: usize = 2 * 1024 * 1024;

struct Component;

impl Guest for Component {
    async fn consume(mut input: wit_bindgen::StreamReader<u8>) -> Result<Vec<Metric>, String> {
        let mut document = Input::Detect(Vec::new());
        let mut buffer = Vec::with_capacity(64 * 1024);
        loop {
            let (status, returned) = input.read(buffer).await;
            buffer = returned;
            document.push(&buffer)?;
            buffer.clear();
            match status {
                wit_bindgen::StreamResult::Complete(_) => {}
                wit_bindgen::StreamResult::Dropped => return document.finish(),
                wit_bindgen::StreamResult::Cancelled => {
                    return Err("document input stream was cancelled".to_owned());
                }
            }
        }
    }
}

enum Input {
    Detect(Vec<u8>),
    Text(text::Analyzer),
    Docx(Vec<u8>),
}

impl Input {
    fn push(&mut self, bytes: &[u8]) -> Result<(), String> {
        match self {
            Self::Text(analyzer) => analyzer.push(bytes),
            Self::Docx(input) => append_container(input, bytes),
            Self::Detect(prefix) => {
                prefix.extend_from_slice(bytes);
                if prefix.len() < 8 {
                    return Ok(());
                }
                let buffered = std::mem::take(prefix);
                if buffered.starts_with(b"%PDF-") {
                    return Err(
                        "PDF extraction is unavailable in the current WASI Component build"
                            .to_owned(),
                    );
                } else if buffered.starts_with(b"PK\x03\x04") {
                    *self = Self::Docx(buffered);
                } else {
                    let mut analyzer = text::Analyzer::default();
                    analyzer.push(&buffered)?;
                    *self = Self::Text(analyzer);
                }
                Ok(())
            }
        }
    }

    fn finish(self) -> Result<Vec<Metric>, String> {
        let stats = match self {
            Self::Detect(bytes) => text::Analyzer::from_bytes(&bytes)?.finish()?,
            Self::Text(analyzer) => analyzer.finish()?,
            Self::Docx(bytes) => text::analyze_extracted(&docx::extract(&bytes)?)?,
        };
        Ok(vec![
            metric("lines", stats.lines),
            metric("words", stats.words),
            metric("characters", stats.characters),
            metric("paragraphs", stats.paragraphs),
        ])
    }
}

fn append_container(input: &mut Vec<u8>, bytes: &[u8]) -> Result<(), String> {
    if input.len().saturating_add(bytes.len()) > MAX_CONTAINER_BYTES {
        return Err("DOCX input exceeds the 2 MiB compressed-size limit".to_owned());
    }
    input.extend_from_slice(bytes);
    Ok(())
}

fn metric(name: &str, value: u64) -> Metric {
    Metric {
        name: name.to_owned(),
        value,
    }
}

export!(Component);
