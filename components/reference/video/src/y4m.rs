use super::VideoResult;

const MAX_HEADER: usize = 1024;
const MAX_DIMENSION: usize = 4096;

#[derive(Default)]
pub(super) struct Analyzer {
    header: Vec<u8>,
    width: usize,
    height: usize,
    frame_bytes: usize,
    luma_bytes: usize,
    payload_remaining: usize,
    luma_remaining: usize,
    frames: u64,
    luma_sum: u64,
    luma_samples: u64,
    previous_luma: Vec<u8>,
    luma_change_sum: u64,
    luma_change_samples: u64,
}

impl Analyzer {
    pub(super) fn push(&mut self, mut bytes: &[u8]) -> Result<(), String> {
        while !bytes.is_empty() {
            if self.payload_remaining > 0 {
                let count = self.payload_remaining.min(bytes.len());
                let luma = self.luma_remaining.min(count);
                let batch_sum = bytes[..luma].iter().map(|byte| u64::from(*byte)).sum();
                self.luma_sum = self
                    .luma_sum
                    .checked_add(batch_sum)
                    .ok_or_else(|| "Y4M luma statistics overflowed".to_owned())?;
                self.record_luma_change(&bytes[..luma])?;
                self.luma_samples = self.luma_samples.saturating_add(luma as u64);
                self.luma_remaining -= luma;
                self.payload_remaining -= count;
                bytes = &bytes[count..];
                if self.payload_remaining == 0 {
                    self.frames = self.frames.saturating_add(1);
                }
                continue;
            }
            let newline = bytes.iter().position(|byte| *byte == b'\n');
            let count = newline.map_or(bytes.len(), |position| position + 1);
            if self.header.len().saturating_add(count) > MAX_HEADER {
                return Err("Y4M header exceeds the 1 KiB limit".to_owned());
            }
            self.header.extend_from_slice(&bytes[..count]);
            bytes = &bytes[count..];
            if newline.is_some() {
                self.parse_header()?;
            }
        }
        Ok(())
    }

    pub(super) fn finish(self) -> Result<VideoResult, String> {
        if self.width == 0 {
            return Err("Y4M file header is missing".to_owned());
        }
        if self.payload_remaining != 0 || !self.header.is_empty() {
            return Err("Y4M input is truncated".to_owned());
        }
        if self.frames == 0 || self.luma_samples == 0 {
            return Err("Y4M input contains no complete frames".to_owned());
        }
        Ok(VideoResult {
            frames: self.frames,
            width: self.width as u64,
            height: self.height as u64,
            average_luma: self.luma_sum / self.luma_samples,
            average_luma_change: self
                .luma_change_sum
                .checked_div(self.luma_change_samples)
                .map_or(0, |average| average),
        })
    }

    fn record_luma_change(&mut self, luma: &[u8]) -> Result<(), String> {
        let offset = self.luma_bytes - self.luma_remaining;
        if self.frames == 0 {
            self.previous_luma.extend_from_slice(luma);
        } else {
            let previous = self
                .previous_luma
                .get_mut(offset..offset + luma.len())
                .ok_or_else(|| "Y4M frame dimensions changed unexpectedly".to_owned())?;
            let change: u64 = previous
                .iter_mut()
                .zip(luma)
                .map(|(before, after)| {
                    let change = before.abs_diff(*after);
                    *before = *after;
                    u64::from(change)
                })
                .sum();
            self.luma_change_sum = self
                .luma_change_sum
                .checked_add(change)
                .ok_or_else(|| "Y4M luma-change statistics overflowed".to_owned())?;
            self.luma_change_samples = self.luma_change_samples.saturating_add(luma.len() as u64);
        }
        Ok(())
    }

    fn parse_header(&mut self) -> Result<(), String> {
        let header = std::str::from_utf8(&self.header[..self.header.len() - 1])
            .map_err(|_| "Y4M header is not valid ASCII".to_owned())?;
        if self.width == 0 {
            let (width, height, monochrome) = parse_file_header(header)?;
            self.width = width;
            self.height = height;
            self.luma_bytes = width
                .checked_mul(height)
                .ok_or_else(|| "Y4M dimensions overflow".to_owned())?;
            self.frame_bytes = if monochrome {
                self.luma_bytes
            } else {
                let chroma = width
                    .div_ceil(2)
                    .checked_mul(height.div_ceil(2))
                    .and_then(|value| value.checked_mul(2))
                    .ok_or_else(|| "Y4M dimensions overflow".to_owned())?;
                self.luma_bytes
                    .checked_add(chroma)
                    .ok_or_else(|| "Y4M dimensions overflow".to_owned())?
            };
        } else {
            if header != "FRAME" && !header.starts_with("FRAME ") {
                return Err("Y4M frame header is invalid".to_owned());
            }
            self.payload_remaining = self.frame_bytes;
            self.luma_remaining = self.luma_bytes;
        }
        self.header.clear();
        Ok(())
    }
}

fn parse_file_header(header: &str) -> Result<(usize, usize, bool), String> {
    let mut tokens = header.split_ascii_whitespace();
    if tokens.next() != Some("YUV4MPEG2") {
        return Err("Y4M file header is invalid".to_owned());
    }
    let mut width = None;
    let mut height = None;
    let mut chroma = "420";
    for token in tokens {
        match token.as_bytes().first() {
            Some(b'W') => width = token[1..].parse().ok(),
            Some(b'H') => height = token[1..].parse().ok(),
            Some(b'C') => chroma = &token[1..],
            _ => {}
        }
    }
    let width = width
        .filter(|value| *value > 0 && *value <= MAX_DIMENSION)
        .ok_or_else(|| "Y4M width is missing or exceeds 4096 pixels".to_owned())?;
    let height = height
        .filter(|value| *value > 0 && *value <= MAX_DIMENSION)
        .ok_or_else(|| "Y4M height is missing or exceeds 4096 pixels".to_owned())?;
    let monochrome = chroma == "mono";
    if !monochrome && !matches!(chroma, "420" | "420jpeg" | "420mpeg2" | "420paldv") {
        return Err("unsupported Y4M chroma; supported: 4:2:0 or mono".to_owned());
    }
    Ok((width, height, monochrome))
}
