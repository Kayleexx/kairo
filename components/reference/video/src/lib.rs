wit_bindgen::generate!({ world: "consume-metrics", path: "wit" });

mod mp4;
mod y4m;

const MAX_MP4_BYTES: usize = 6 * 1024 * 1024;

struct Component;

impl Guest for Component {
    async fn consume(mut input: wit_bindgen::StreamReader<u8>) -> Result<Vec<Metric>, String> {
        let mut analyzer = Input::Detect(Vec::new());
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
                    return Err("video input stream was cancelled".to_owned());
                }
            }
        }
    }
}

enum Input {
    Detect(Vec<u8>),
    Y4m(y4m::Analyzer),
    Mp4(Vec<u8>),
}

impl Input {
    fn push(&mut self, bytes: &[u8]) -> Result<(), String> {
        match self {
            Self::Y4m(analyzer) => analyzer.push(bytes),
            Self::Mp4(input) => append_mp4(input, bytes),
            Self::Detect(prefix) => {
                prefix.extend_from_slice(bytes);
                if prefix.starts_with(b"YUV4MPEG2") {
                    let buffered = std::mem::take(prefix);
                    let mut analyzer = y4m::Analyzer::default();
                    analyzer.push(&buffered)?;
                    *self = Self::Y4m(analyzer);
                } else if prefix.len() >= 12 {
                    if prefix.get(4..8) != Some(b"ftyp") {
                        return Err("video input is neither Y4M nor MP4 content".to_owned());
                    }
                    if prefix.len() > MAX_MP4_BYTES {
                        return Err("MP4 input exceeds the 6 MiB limit".to_owned());
                    }
                    *self = Self::Mp4(std::mem::take(prefix));
                }
                Ok(())
            }
        }
    }

    fn finish(self) -> Result<Vec<Metric>, String> {
        let result = match self {
            Self::Y4m(analyzer) => analyzer.finish()?,
            Self::Mp4(input) => mp4::analyze(&input)?,
            Self::Detect(_) => return Err("video input is too short".to_owned()),
        };
        Ok(vec![
            metric("frames-analyzed", result.frames),
            metric("average-luma", result.average_luma),
            metric("average-luma-change", result.average_luma_change),
            metric("width", result.width),
            metric("height", result.height),
        ])
    }
}

fn append_mp4(input: &mut Vec<u8>, bytes: &[u8]) -> Result<(), String> {
    if input.len().saturating_add(bytes.len()) > MAX_MP4_BYTES {
        return Err("MP4 input exceeds the 6 MiB limit".to_owned());
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

struct VideoResult {
    frames: u64,
    width: u64,
    height: u64,
    average_luma: u64,
    average_luma_change: u64,
}

export!(Component);
