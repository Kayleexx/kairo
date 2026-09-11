use std::io::Cursor;

use office_oxide::docx::DocxDocument;
use zip::ZipArchive;

const MAX_ENTRIES: usize = 256;
const MAX_PART_BYTES: u64 = 2 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 8 * 1024 * 1024;

pub(super) fn extract(bytes: &[u8]) -> Result<String, String> {
    validate_archive(bytes)?;
    let document = DocxDocument::from_reader(Cursor::new(bytes.to_vec()))
        .map_err(|error| format!("DOCX input is invalid or unsupported: {error}"))?;
    let text = document.plain_text();
    if text.len() > 1024 * 1024 {
        return Err("extracted DOCX text exceeds the 1 MiB limit".to_owned());
    }
    Ok(text)
}

fn validate_archive(bytes: &[u8]) -> Result<(), String> {
    let mut archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|error| format!("DOCX ZIP container is invalid: {error}"))?;
    if archive.len() > MAX_ENTRIES {
        return Err("DOCX contains more than the 256-part limit".to_owned());
    }
    let mut total = 0_u64;
    for index in 0..archive.len() {
        let file = archive
            .by_index(index)
            .map_err(|error| format!("DOCX ZIP entry is invalid: {error}"))?;
        if file.size() > MAX_PART_BYTES {
            return Err("DOCX part exceeds the 2 MiB expanded-size limit".to_owned());
        }
        total = total
            .checked_add(file.size())
            .filter(|value| *value <= MAX_TOTAL_BYTES)
            .ok_or_else(|| "DOCX exceeds the 8 MiB expanded-size limit".to_owned())?;
    }
    Ok(())
}
