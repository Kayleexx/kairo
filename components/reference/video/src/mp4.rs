use std::io::Cursor;

use oxideav_core::{Demuxer, MediaType, NullCodecResolver};
use rusty_h264_decoder::Decoder;

use super::VideoResult;

const MAX_ANALYZED_FRAMES: u64 = 24;
const MAX_WIDTH: u64 = 1280;
const MAX_HEIGHT: u64 = 720;

pub(super) fn analyze(bytes: &[u8]) -> Result<VideoResult, String> {
    let mut demuxer =
        oxideav_mp4::demux::open_typed(Box::new(Cursor::new(bytes.to_vec())), &NullCodecResolver)
            .map_err(|error| format!("MP4 container is malformed: {error}"))?;
    let stream = demuxer
        .streams()
        .iter()
        .find(|stream| stream.params.media_type == MediaType::Video)
        .ok_or_else(|| "MP4 contains no video track".to_owned())?;
    if stream.params.codec_id.as_str() != "h264" {
        return Err(format!(
            "unsupported MP4 codec: {}; supported: H.264/AVC",
            stream.params.codec_id
        ));
    }
    let width = u64::from(
        stream
            .params
            .width
            .ok_or_else(|| "MP4 video width is missing".to_owned())?,
    );
    let height = u64::from(
        stream
            .params
            .height
            .ok_or_else(|| "MP4 video height is missing".to_owned())?,
    );
    validate_dimensions(width, height)?;
    let stream_index = stream.index;
    let extradata = stream.params.extradata.clone();
    let length_size = extradata
        .get(4)
        .map(|byte| usize::from(byte & 3) + 1)
        .ok_or_else(|| "MP4 AVC configuration is invalid".to_owned())?;
    demuxer.set_active_streams(&[stream_index]);

    let mut decoder = Decoder::new();
    let config = avcc_config_to_annex_b(&extradata)?;
    decoder
        .decode(&config)
        .map_err(|error| format!("unsupported H.264 format: {error}"))?;
    let mut result = VideoResult {
        frames: 0,
        width,
        height,
        average_luma: 0,
    };
    let mut luma_sum = 0_u64;
    let mut luma_samples = 0_u64;
    loop {
        if result.frames == MAX_ANALYZED_FRAMES {
            break;
        }
        let packet = match demuxer.next_packet() {
            Ok(packet) => packet,
            Err(oxideav_core::Error::Eof) => break,
            Err(error) => return Err(format!("MP4 container is malformed: {error}")),
        };
        if packet.stream_index != stream_index {
            continue;
        }
        let access_unit = avcc_sample_to_annex_b(&packet.data, length_size)?;
        let frame = decoder
            .decode(&access_unit)
            .map_err(|error| format!("H.264 decode failed: {error}"))?;
        let Some(frame) = frame else { continue };
        result.frames = result.frames.saturating_add(1);
        validate_dimensions(frame.width as u64, frame.height as u64)?;
        if frame.width as u64 != width || frame.height as u64 != height {
            return Err("MP4 decoded dimensions do not match its video track".to_owned());
        }
        luma_sum = luma_sum
            .checked_add(frame.y.iter().map(|value| u64::from(*value)).sum())
            .ok_or_else(|| "MP4 luma statistics overflowed".to_owned())?;
        luma_samples = luma_samples.saturating_add(frame.y.len() as u64);
    }
    if result.frames == 0 || luma_samples == 0 {
        return Err("MP4 contains no decodable H.264 frames".to_owned());
    }
    result.average_luma = luma_sum / luma_samples;
    Ok(result)
}

fn validate_dimensions(width: u64, height: u64) -> Result<(), String> {
    if width == 0 || height == 0 || width > MAX_WIDTH || height > MAX_HEIGHT {
        return Err(format!(
            "video dimensions {width}x{height} exceed the 1280x720 limit"
        ));
    }
    Ok(())
}

fn avcc_config_to_annex_b(data: &[u8]) -> Result<Vec<u8>, String> {
    if data.len() < 7 {
        return Err("MP4 AVC configuration is invalid".to_owned());
    }
    let mut position = 6;
    let sps_count = usize::from(data[5] & 0x1f);
    let mut output = Vec::new();
    for _ in 0..sps_count {
        append_config_nal(data, &mut position, &mut output)?;
    }
    let pps_count = usize::from(
        *data
            .get(position)
            .ok_or_else(|| "MP4 AVC configuration is invalid".to_owned())?,
    );
    position += 1;
    for _ in 0..pps_count {
        append_config_nal(data, &mut position, &mut output)?;
    }
    Ok(output)
}

fn append_config_nal(
    data: &[u8],
    position: &mut usize,
    output: &mut Vec<u8>,
) -> Result<(), String> {
    let end = position.saturating_add(2);
    let length_bytes = data
        .get(*position..end)
        .ok_or_else(|| "MP4 AVC configuration is truncated".to_owned())?;
    let length = usize::from(u16::from_be_bytes([length_bytes[0], length_bytes[1]]));
    *position = end;
    let end = position.saturating_add(length);
    let nal = data
        .get(*position..end)
        .ok_or_else(|| "MP4 AVC configuration is truncated".to_owned())?;
    output.extend_from_slice(&[0, 0, 0, 1]);
    output.extend_from_slice(nal);
    *position = end;
    Ok(())
}

fn avcc_sample_to_annex_b(data: &[u8], length_size: usize) -> Result<Vec<u8>, String> {
    let mut output = Vec::with_capacity(data.len().saturating_add(16));
    let mut position = 0;
    while position < data.len() {
        let end = position.saturating_add(length_size);
        let length_bytes = data
            .get(position..end)
            .ok_or_else(|| "MP4 video sample is truncated".to_owned())?;
        let mut length = 0_usize;
        for byte in length_bytes {
            length = length
                .checked_mul(256)
                .and_then(|value| value.checked_add(usize::from(*byte)))
                .ok_or_else(|| "MP4 video sample length overflowed".to_owned())?;
        }
        position = end;
        let end = position.saturating_add(length);
        let nal = data
            .get(position..end)
            .ok_or_else(|| "MP4 video sample is truncated".to_owned())?;
        output.extend_from_slice(&[0, 0, 0, 1]);
        output.extend_from_slice(nal);
        position = end;
    }
    Ok(output)
}
