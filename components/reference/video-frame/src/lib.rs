wit_bindgen::generate!({ world: "output", path: "wit" });

use std::io::Cursor;

use oxideav_core::{Demuxer, MediaType, NullCodecResolver};
use rusty_h264_decoder::Decoder;

const MAX_MP4_BYTES: usize = 6 * 1024 * 1024;
const MAX_WIDTH: usize = 1280;
const MAX_HEIGHT: usize = 720;
const MAX_SAMPLED_FRAMES: usize = 24;
const CONTACT_COLUMNS: usize = 3;
const CONTACT_ROWS: usize = 2;
const THUMBNAIL_WIDTH: usize = 320;
const THUMBNAIL_HEIGHT: usize = 180;
const GUTTER: usize = 4;

struct Component;

impl Guest for Component {
    async fn output(mut input: wit_bindgen::StreamReader<u8>) -> Result<Vec<u8>, String> {
        let mut bytes = Vec::new();
        let mut buffer = Vec::with_capacity(64 * 1024);
        loop {
            let (status, returned) = input.read(buffer).await;
            buffer = returned;
            append(&mut bytes, &buffer)?;
            buffer.clear();
            match status {
                wit_bindgen::StreamResult::Complete(_) => {}
                wit_bindgen::StreamResult::Dropped => return contact_sheet_png(&bytes),
                wit_bindgen::StreamResult::Cancelled => {
                    return Err("video input stream was cancelled".to_owned());
                }
            }
        }
    }
}

fn append(output: &mut Vec<u8>, bytes: &[u8]) -> Result<(), String> {
    if output.len().saturating_add(bytes.len()) > MAX_MP4_BYTES {
        return Err("MP4 input exceeds the 6 MiB limit".to_owned());
    }
    output.extend_from_slice(bytes);
    Ok(())
}

fn contact_sheet_png(bytes: &[u8]) -> Result<Vec<u8>, String> {
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
    let width = usize::try_from(
        stream
            .params
            .width
            .ok_or_else(|| "MP4 video width is missing".to_owned())?,
    )
    .map_err(|_| "MP4 video width is too large".to_owned())?;
    let height = usize::try_from(
        stream
            .params
            .height
            .ok_or_else(|| "MP4 video height is missing".to_owned())?,
    )
    .map_err(|_| "MP4 video height is too large".to_owned())?;
    dimensions(width, height)?;
    let stream_index = stream.index;
    let extradata = stream.params.extradata.clone();
    let length_size = extradata
        .get(4)
        .map(|byte| usize::from(byte & 3) + 1)
        .ok_or_else(|| "MP4 AVC configuration is invalid".to_owned())?;
    demuxer.set_active_streams(&[stream_index]);
    let mut decoder = Decoder::new();
    decoder
        .decode(&config_to_annex_b(&extradata)?)
        .map_err(|error| format!("unsupported H.264 format: {error}"))?;
    let mut thumbnails = Vec::with_capacity(MAX_SAMPLED_FRAMES);
    while thumbnails.len() < MAX_SAMPLED_FRAMES {
        let packet = match demuxer.next_packet() {
            Ok(packet) => packet,
            Err(oxideav_core::Error::Eof) => break,
            Err(error) => return Err(format!("MP4 container is malformed: {error}")),
        };
        if packet.stream_index != stream_index {
            continue;
        }
        let frame = decoder
            .decode(&sample_to_annex_b(&packet.data, length_size)?)
            .map_err(|error| format!("H.264 decode failed: {error}"))?;
        let Some(frame) = frame else { continue };
        dimensions(frame.width, frame.height)?;
        if frame.width != width || frame.height != height {
            return Err("MP4 decoded dimensions do not match its video track".to_owned());
        }
        thumbnails.push(thumbnail(&frame.y, width, height)?);
    }
    if thumbnails.is_empty() {
        return Err("MP4 contains no decodable H.264 frames".to_owned());
    }
    encode_png(
        &compose_contact_sheet(&thumbnails),
        contact_width(),
        contact_height(),
    )
}

fn thumbnail(gray: &[u8], width: usize, height: usize) -> Result<Vec<u8>, String> {
    if gray.len() != width.saturating_mul(height) {
        return Err("H.264 decoder returned an invalid luma plane".to_owned());
    }
    let (scaled_width, scaled_height) =
        if width.saturating_mul(THUMBNAIL_HEIGHT) > height.saturating_mul(THUMBNAIL_WIDTH) {
            (THUMBNAIL_WIDTH, height * THUMBNAIL_WIDTH / width)
        } else {
            (width * THUMBNAIL_HEIGHT / height, THUMBNAIL_HEIGHT)
        };
    let mut output = vec![0; THUMBNAIL_WIDTH * THUMBNAIL_HEIGHT];
    let left = (THUMBNAIL_WIDTH - scaled_width) / 2;
    let top = (THUMBNAIL_HEIGHT - scaled_height) / 2;
    for y in 0..scaled_height {
        let source_y = y * height / scaled_height;
        for x in 0..scaled_width {
            let source_x = x * width / scaled_width;
            output[(top + y) * THUMBNAIL_WIDTH + left + x] = gray[source_y * width + source_x];
        }
    }
    Ok(output)
}

fn compose_contact_sheet(thumbnails: &[Vec<u8>]) -> Vec<u8> {
    let mut output = vec![0; contact_width() * contact_height()];
    let count = thumbnails.len().min(CONTACT_COLUMNS * CONTACT_ROWS);
    for slot in 0..count {
        let index = sample_index(slot, count, thumbnails.len());
        let column = slot % CONTACT_COLUMNS;
        let row = slot / CONTACT_COLUMNS;
        let left = column * (THUMBNAIL_WIDTH + GUTTER);
        let top = row * (THUMBNAIL_HEIGHT + GUTTER);
        for y in 0..THUMBNAIL_HEIGHT {
            let destination = (top + y) * contact_width() + left;
            let source = y * THUMBNAIL_WIDTH;
            output[destination..destination + THUMBNAIL_WIDTH]
                .copy_from_slice(&thumbnails[index][source..source + THUMBNAIL_WIDTH]);
        }
    }
    output
}

fn sample_index(slot: usize, count: usize, available: usize) -> usize {
    if count <= 1 {
        return 0;
    }
    (slot * (available - 1) + (count - 1) / 2) / (count - 1)
}

fn contact_width() -> usize {
    CONTACT_COLUMNS * THUMBNAIL_WIDTH + (CONTACT_COLUMNS - 1) * GUTTER
}

fn contact_height() -> usize {
    CONTACT_ROWS * THUMBNAIL_HEIGHT + (CONTACT_ROWS - 1) * GUTTER
}

fn dimensions(width: usize, height: usize) -> Result<(), String> {
    if width == 0 || height == 0 || width > MAX_WIDTH || height > MAX_HEIGHT {
        return Err(format!(
            "video dimensions {width}x{height} exceed the 1280x720 limit"
        ));
    }
    Ok(())
}

fn encode_png(gray: &[u8], width: usize, height: usize) -> Result<Vec<u8>, String> {
    if gray.len() != width.saturating_mul(height) {
        return Err("H.264 decoder returned an invalid luma plane".to_owned());
    }
    let mut output = Vec::new();
    let mut encoder = png::Encoder::new(&mut output, width as u32, height as u32);
    encoder.set_color(png::ColorType::Grayscale);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder
        .write_header()
        .map_err(|error| format!("PNG encoder failed: {error}"))?;
    writer
        .write_image_data(gray)
        .map_err(|error| format!("PNG encoder failed: {error}"))?;
    drop(writer);
    Ok(output)
}

fn config_to_annex_b(data: &[u8]) -> Result<Vec<u8>, String> {
    if data.len() < 7 {
        return Err("MP4 AVC configuration is invalid".to_owned());
    }
    let mut position = 6;
    let mut output = Vec::new();
    for _ in 0..usize::from(data[5] & 0x1f) {
        append_nal(data, &mut position, &mut output)?;
    }
    let count = usize::from(
        *data
            .get(position)
            .ok_or_else(|| "MP4 AVC configuration is invalid".to_owned())?,
    );
    position += 1;
    for _ in 0..count {
        append_nal(data, &mut position, &mut output)?;
    }
    Ok(output)
}

fn append_nal(data: &[u8], position: &mut usize, output: &mut Vec<u8>) -> Result<(), String> {
    let length_bytes = data
        .get(*position..position.saturating_add(2))
        .ok_or_else(|| "MP4 AVC configuration is truncated".to_owned())?;
    let length = usize::from(u16::from_be_bytes([length_bytes[0], length_bytes[1]]));
    *position = position.saturating_add(2);
    let nal = data
        .get(*position..position.saturating_add(length))
        .ok_or_else(|| "MP4 AVC configuration is truncated".to_owned())?;
    output.extend_from_slice(&[0, 0, 0, 1]);
    output.extend_from_slice(nal);
    *position = position.saturating_add(length);
    Ok(())
}

fn sample_to_annex_b(data: &[u8], length_size: usize) -> Result<Vec<u8>, String> {
    let mut output = Vec::with_capacity(data.len().saturating_add(16));
    let mut position = 0;
    while position < data.len() {
        let length_bytes = data
            .get(position..position.saturating_add(length_size))
            .ok_or_else(|| "MP4 video sample is truncated".to_owned())?;
        let mut length = 0_usize;
        for byte in length_bytes {
            length = length
                .checked_mul(256)
                .and_then(|value| value.checked_add(usize::from(*byte)))
                .ok_or_else(|| "MP4 video sample length overflowed".to_owned())?;
        }
        position = position.saturating_add(length_size);
        let nal = data
            .get(position..position.saturating_add(length))
            .ok_or_else(|| "MP4 video sample is truncated".to_owned())?;
        output.extend_from_slice(&[0, 0, 0, 1]);
        output.extend_from_slice(nal);
        position = position.saturating_add(length);
    }
    Ok(output)
}

export!(Component);
