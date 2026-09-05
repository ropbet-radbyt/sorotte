//! Resource limits shared by update ingress and the independent installed updater.

use std::{
    io::{Read, Seek, SeekFrom, Write},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use zip::ZipArchive;

pub(super) const METADATA_BYTES: u64 = 2 * 1024 * 1024;
pub(super) const ARCHIVE_BYTES: u64 = 512 * 1024 * 1024;
const ENTRY_BYTES: u64 = 512 * 1024 * 1024;
const EXTRACTED_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const ARCHIVE_ENTRIES: usize = 4096;
const DIRECTORY_BYTES: u64 = 8 * 1024 * 1024;

pub(super) fn check_cancelled(cancelled: &AtomicBool) -> Result<(), String> {
    if cancelled.load(Ordering::Acquire) {
        Err("Update download cancelled.".to_owned())
    } else {
        Ok(())
    }
}

pub(super) fn read_limited(mut reader: impl Read, limit: u64) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    let mut buffer = [0; 16 * 1024];
    loop {
        let remaining = limit.saturating_sub(output.len() as u64);
        let available = buffer.len().min(remaining.saturating_add(1) as usize);
        let count = reader
            .read(&mut buffer[..available])
            .map_err(|error| format!("failed reading bounded update input: {error}"))?;
        if count == 0 {
            return Ok(output);
        }
        if count as u64 > remaining {
            return Err("Update input exceeded its byte budget.".to_owned());
        }
        output.extend_from_slice(&buffer[..count]);
    }
}

/// Check the tiny end record before ZipArchive can allocate its central-directory inventory.
pub(super) fn open_bounded_zip<R: Read + Seek>(
    mut reader: R,
) -> Result<ZipArchive<ArchiveReader<R>>, String> {
    let length = reader.seek(SeekFrom::End(0)).map_err(zip_io_error)?;
    if length > ARCHIVE_BYTES {
        return Err("Update archive exceeded its byte budget.".to_owned());
    }
    let tail_length = length.min(65_557) as usize;
    reader
        .seek(SeekFrom::End(-(tail_length as i64)))
        .map_err(zip_io_error)?;
    let mut tail = vec![0; tail_length];
    reader.read_exact(&mut tail).map_err(zip_io_error)?;
    let offset = (0..tail.len().saturating_sub(21))
        .rev()
        .find(|&index| {
            tail[index..].starts_with(b"PK\x05\x06")
                && index + 22 + u16::from_le_bytes([tail[index + 20], tail[index + 21]]) as usize
                    == tail.len()
        })
        .ok_or_else(|| "Update archive has no valid end record.".to_owned())?;
    let end = &tail[offset..];
    let end_position = length - tail_length as u64 + offset as u64;
    let u16_at = |offset| u16::from_le_bytes([end[offset], end[offset + 1]]);
    let u32_at = |offset| u32::from_le_bytes(end[offset..offset + 4].try_into().unwrap());
    if u16_at(4) != 0 || u16_at(6) != 0 || u16_at(8) != u16_at(10) {
        return Err("Multi-disk update archives are unsupported.".to_owned());
    }
    let mut entries = u16_at(10) as u64;
    let mut directory_bytes = u32_at(12) as u64;
    let mut directory_offset = u32_at(16) as u64;
    let mut zip64_record_position = None;
    let has_zip64_locator = offset >= 20 && tail[offset - 20..offset - 16] == *b"PK\x06\x07";
    if has_zip64_locator
        || entries == u16::MAX as u64
        || directory_bytes == u32::MAX as u64
        || u32_at(16) == u32::MAX
    {
        let locator_position = end_position
            .checked_sub(20)
            .ok_or_else(|| "Invalid ZIP64 locator.".to_owned())?;
        reader
            .seek(SeekFrom::Start(locator_position))
            .map_err(zip_io_error)?;
        let mut locator = [0; 20];
        reader.read_exact(&mut locator).map_err(zip_io_error)?;
        if !locator.starts_with(b"PK\x06\x07")
            || locator[4..8] != [0; 4]
            || locator[16..20] != [1, 0, 0, 0]
        {
            return Err("Invalid ZIP64 update locator.".to_owned());
        }
        let record_position = u64::from_le_bytes(locator[8..16].try_into().unwrap());
        zip64_record_position = Some(record_position);
        if record_position
            .checked_add(56)
            .is_none_or(|position| position > locator_position)
        {
            return Err("Invalid ZIP64 update record position.".to_owned());
        }
        reader
            .seek(SeekFrom::Start(record_position))
            .map_err(zip_io_error)?;
        let mut record = [0; 56];
        reader.read_exact(&mut record).map_err(zip_io_error)?;
        if !record.starts_with(b"PK\x06\x06")
            || record[16..24] != [0; 8]
            || record[24..32] != record[32..40]
        {
            return Err("Invalid ZIP64 update record.".to_owned());
        }
        let record_size = u64::from_le_bytes(record[4..12].try_into().unwrap());
        if !(44..=DIRECTORY_BYTES).contains(&record_size)
            || record_position
                .checked_add(record_size)
                .and_then(|position| position.checked_add(12))
                .is_none_or(|position| position > locator_position)
        {
            return Err("ZIP64 update record exceeded its byte budget.".to_owned());
        }
        let zip64_entries = u64::from_le_bytes(record[32..40].try_into().unwrap());
        let zip64_directory_bytes = u64::from_le_bytes(record[40..48].try_into().unwrap());
        if (entries != u16::MAX as u64 && entries != zip64_entries)
            || (directory_bytes != u32::MAX as u64 && directory_bytes != zip64_directory_bytes)
        {
            return Err("Inconsistent ZIP64 update inventory.".to_owned());
        }
        entries = zip64_entries;
        directory_bytes = zip64_directory_bytes;
        directory_offset = u64::from_le_bytes(record[48..56].try_into().unwrap());
    }
    if entries > ARCHIVE_ENTRIES as u64 {
        return Err("Update archive exceeded its entry-count budget.".to_owned());
    }
    if directory_bytes > DIRECTORY_BYTES {
        return Err("Update archive directory exceeded its byte budget.".to_owned());
    }
    // zip's constructor retries earlier EOCD records if the chosen directory is
    // malformed. Give it only the admitted index while it builds its inventory,
    // preventing embedded records in a binary payload from supplying new limits.
    let index_length = length
        .checked_sub(directory_offset)
        .filter(|&size| size <= DIRECTORY_BYTES * 2 + 65_557)
        .ok_or_else(|| "Update archive index exceeded its byte budget.".to_owned())?;
    if directory_offset
        .checked_add(directory_bytes)
        .is_none_or(|position| position > zip64_record_position.unwrap_or(end_position))
    {
        return Err("Update archive directory overlaps its end records.".to_owned());
    }
    reader
        .seek(SeekFrom::Start(directory_offset))
        .map_err(zip_io_error)?;
    let mut index_bytes = vec![0; index_length as usize];
    reader.read_exact(&mut index_bytes).map_err(zip_io_error)?;
    for (offset, signature) in index_bytes.windows(4).enumerate() {
        let position = directory_offset + offset as u64;
        let expected = match signature {
            b"PK\x05\x06" => Some(end_position),
            b"PK\x06\x06" => zip64_record_position,
            b"PK\x06\x07" => zip64_record_position.map(|_| end_position - 20),
            _ => continue,
        };
        if expected != Some(position) {
            return Err("Update archive index contains an ambiguous end record.".to_owned());
        }
    }
    reader.seek(SeekFrom::Start(0)).map_err(zip_io_error)?;
    let loading_index = Arc::new(AtomicBool::new(true));
    let reader = ArchiveReader {
        inner: reader,
        position: 0,
        index_start: directory_offset,
        loading_index: loading_index.clone(),
        remaining_index_reads: DIRECTORY_BYTES * 4,
    };
    let archive = ZipArchive::with_config(
        zip::read::Config {
            archive_offset: zip::read::ArchiveOffset::Known(0),
        },
        reader,
    )
    .map_err(|error| format!("failed opening update archive: {error}"))?;
    loading_index.store(false, Ordering::Release);
    if archive.len() as u64 != entries {
        return Err(
            "Update archive contains duplicate entries or an inconsistent entry count.".to_owned(),
        );
    }
    Ok(archive)
}

/// Payload bytes are hidden only during inventory construction; extraction reads
/// the original file unchanged after successful admission. This is not a copy.
pub(super) struct ArchiveReader<R> {
    inner: R,
    position: u64,
    index_start: u64,
    loading_index: Arc<AtomicBool>,
    remaining_index_reads: u64,
}

impl<R> std::fmt::Debug for ArchiveReader<R> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ArchiveReader")
            .finish_non_exhaustive()
    }
}

impl<R: Read> Read for ArchiveReader<R> {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        let loading = self.loading_index.load(Ordering::Acquire);
        if loading && output.len() as u64 > self.remaining_index_reads {
            return Err(std::io::Error::other(
                "Update archive index read budget exceeded",
            ));
        }
        let count = self.inner.read(output)?;
        if loading {
            self.remaining_index_reads -= count as u64;
            let hidden = self
                .index_start
                .saturating_sub(self.position)
                .min(count as u64) as usize;
            output[..hidden].fill(0);
        }
        self.position += count as u64;
        Ok(count)
    }
}

impl<R: Seek> Seek for ArchiveReader<R> {
    fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
        self.position = self.inner.seek(position)?;
        Ok(self.position)
    }
}

fn zip_io_error(error: std::io::Error) -> String {
    format!("failed reading update archive: {error}")
}

pub(super) fn unambiguous_output_component(component: &str) -> bool {
    let stem = component
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    !component.contains(':')
        && !component.ends_with(['.', ' '])
        && component.len() <= 255
        && !matches!(
            stem.as_str(),
            "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
        )
        && !(stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'))
}

pub(super) struct ExtractionBudget {
    remaining_entries: usize,
    remaining_bytes: u64,
    entry_bytes: u64,
}

impl Default for ExtractionBudget {
    fn default() -> Self {
        Self {
            remaining_entries: ARCHIVE_ENTRIES,
            remaining_bytes: EXTRACTED_BYTES,
            entry_bytes: ENTRY_BYTES,
        }
    }
}

impl ExtractionBudget {
    #[cfg(test)]
    pub fn with_limits(entries: usize, bytes: u64, entry_bytes: u64) -> Self {
        Self {
            remaining_entries: entries,
            remaining_bytes: bytes,
            entry_bytes,
        }
    }
    pub fn admit_archive<R: Read + Seek>(
        &mut self,
        archive: &mut ZipArchive<R>,
    ) -> Result<(), String> {
        if archive.len() > self.remaining_entries {
            return Err("Update archive exceeded its entry-count budget.".to_owned());
        }
        let mut declared_total = 0u64;
        for index in 0..archive.len() {
            let entry = archive
                .by_index(index)
                .map_err(|error| format!("failed reading update archive entry: {error}"))?;
            if entry.size() > self.entry_bytes {
                return Err("Update archive entry exceeded its byte budget.".to_owned());
            }
            declared_total = declared_total
                .checked_add(entry.size())
                .ok_or_else(|| "Update extraction byte budget overflow.".to_owned())?;
            if declared_total > self.remaining_bytes {
                return Err("Update archive exceeded its decompressed byte budget.".to_owned());
            }
        }
        self.remaining_entries -= archive.len();
        Ok(())
    }

    pub fn copy_entry(
        &mut self,
        mut input: impl Read,
        mut output: impl Write,
        cancelled: &AtomicBool,
    ) -> Result<u64, String> {
        let mut written = 0u64;
        let mut buffer = [0; 64 * 1024];
        loop {
            check_cancelled(cancelled)?;
            let available = (self.entry_bytes - written).min(self.remaining_bytes);
            let count_limit = buffer.len().min(available.saturating_add(1) as usize);
            let count = input
                .read(&mut buffer[..count_limit])
                .map_err(zip_io_error)?;
            if count == 0 {
                return Ok(written);
            }
            if count as u64 > available {
                return Err("Update extraction exceeded its decompressed byte budget.".to_owned());
            }
            check_cancelled(cancelled)?;
            output
                .write_all(&buffer[..count])
                .map_err(|error| format!("failed writing update entry: {error}"))?;
            written += count as u64;
            self.remaining_bytes -= count as u64;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn one_file_zip(payload: &[u8]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .start_file("payload.bin", zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(payload).unwrap();
        writer.finish().unwrap().into_inner()
    }

    #[test]
    fn malformed_final_directory_cannot_fall_back_to_an_earlier_payload_inventory() {
        let mut bytes = one_file_zip(b"old payload");
        let offset = bytes.len() as u32;
        bytes.extend([0; 46]);
        let mut end = [0; 22];
        end[..4].copy_from_slice(b"PK\x05\x06");
        end[8..10].copy_from_slice(&1u16.to_le_bytes());
        end[10..12].copy_from_slice(&1u16.to_le_bytes());
        end[12..16].copy_from_slice(&46u32.to_le_bytes());
        end[16..20].copy_from_slice(&offset.to_le_bytes());
        bytes.extend(end);
        assert!(
            ZipArchive::new(Cursor::new(&bytes)).is_ok(),
            "fixture must exercise zip's fallback behavior"
        );
        assert!(open_bounded_zip(Cursor::new(&bytes)).is_err());
    }

    #[test]
    fn archive_admission_preserves_payload_bytes_and_supports_a_bounded_zip64_index() {
        let payload = b"binary payload containing PK\x05\x06 and PK\x06\x06 markers";
        let bytes = one_file_zip(payload);
        let mut archive = open_bounded_zip(Cursor::new(&bytes)).unwrap();
        let mut extracted = Vec::new();
        archive
            .by_index(0)
            .unwrap()
            .read_to_end(&mut extracted)
            .unwrap();
        assert_eq!(extracted, payload);

        let end_position = bytes.len() - 22;
        let mut end = bytes[end_position..].to_vec();
        let directory_size = u32::from_le_bytes(end[12..16].try_into().unwrap());
        let directory_offset = u32::from_le_bytes(end[16..20].try_into().unwrap());
        let mut zip64 = bytes[..end_position].to_vec();
        let mut record = [0; 56];
        record[..4].copy_from_slice(b"PK\x06\x06");
        record[4..12].copy_from_slice(&44u64.to_le_bytes());
        record[12..14].copy_from_slice(&45u16.to_le_bytes());
        record[14..16].copy_from_slice(&45u16.to_le_bytes());
        record[24..32].copy_from_slice(&1u64.to_le_bytes());
        record[32..40].copy_from_slice(&1u64.to_le_bytes());
        record[40..48].copy_from_slice(&(directory_size as u64).to_le_bytes());
        record[48..56].copy_from_slice(&(directory_offset as u64).to_le_bytes());
        zip64.extend(record);
        let mut locator = [0; 20];
        locator[..4].copy_from_slice(b"PK\x06\x07");
        locator[8..16].copy_from_slice(&(end_position as u64).to_le_bytes());
        locator[16..20].copy_from_slice(&1u32.to_le_bytes());
        zip64.extend(locator);
        end[8..20].fill(255);
        zip64.extend(end);
        let mut archive = open_bounded_zip(Cursor::new(&zip64)).unwrap();
        let mut extracted = Vec::new();
        archive
            .by_index(0)
            .unwrap()
            .read_to_end(&mut extracted)
            .unwrap();
        assert_eq!(extracted, payload);
    }

    #[test]
    fn actual_extraction_bytes_cannot_cross_entry_or_shared_nested_budgets() {
        let mut budget = ExtractionBudget::with_limits(3, 6, 4);
        let cancelled = AtomicBool::new(false);
        let mut output = Vec::new();
        budget
            .copy_entry(Cursor::new(b"abcd"), &mut output, &cancelled)
            .unwrap();
        assert_eq!(output, b"abcd");
        let mut nested = Vec::new();
        assert!(
            budget
                .copy_entry(Cursor::new(b"efg"), &mut nested, &cancelled)
                .is_err()
        );
        assert!(nested.len() <= 2);
        let mut budget = ExtractionBudget {
            remaining_entries: 3,
            remaining_bytes: 100,
            entry_bytes: 4,
        };
        let mut oversized = Vec::new();
        assert!(
            budget
                .copy_entry(Cursor::new(b"12345"), &mut oversized, &cancelled)
                .is_err()
        );
        assert!(oversized.len() <= 4);
    }

    #[test]
    fn bounded_reads_accept_exact_limit_and_reject_the_next_byte() {
        assert_eq!(read_limited(Cursor::new(b"abcd"), 4).unwrap(), b"abcd");
        assert!(read_limited(Cursor::new(b"abcde"), 4).is_err());
    }

    #[test]
    fn extraction_cancellation_stops_before_output() {
        let mut output = Vec::new();
        assert!(
            ExtractionBudget::default()
                .copy_entry(Cursor::new(b"payload"), &mut output, &AtomicBool::new(true))
                .is_err()
        );
        assert!(output.is_empty());
    }

    #[test]
    fn central_directory_quota_is_checked_before_zip_inventory_allocation() {
        let mut bytes = vec![0; 22];
        bytes[..4].copy_from_slice(b"PK\x05\x06");
        bytes[8..10].copy_from_slice(&4097u16.to_le_bytes());
        bytes[10..12].copy_from_slice(&4097u16.to_le_bytes());
        assert!(
            open_bounded_zip(Cursor::new(&bytes))
                .unwrap_err()
                .contains("entry-count")
        );
        bytes[8..12].fill(0);
        bytes[12..16].copy_from_slice(&(DIRECTORY_BYTES as u32 + 1).to_le_bytes());
        assert!(
            open_bounded_zip(Cursor::new(&bytes))
                .unwrap_err()
                .contains("directory")
        );
        assert!(open_bounded_zip(Cursor::new(b"PK\x05\x06")).is_err());
    }

    #[test]
    fn zip64_inventory_cannot_override_small_ordinary_end_record() {
        let mut bytes = vec![0; 56 + 20 + 22];
        bytes[..4].copy_from_slice(b"PK\x06\x06");
        bytes[4..12].copy_from_slice(&44u64.to_le_bytes());
        bytes[24..32].copy_from_slice(&4097u64.to_le_bytes());
        bytes[32..40].copy_from_slice(&4097u64.to_le_bytes());
        bytes[56..60].copy_from_slice(b"PK\x06\x07");
        bytes[72..76].copy_from_slice(&1u32.to_le_bytes());
        bytes[76..80].copy_from_slice(b"PK\x05\x06");
        assert!(
            open_bounded_zip(Cursor::new(&bytes))
                .unwrap_err()
                .contains("Inconsistent ZIP64")
        );
        bytes[84..88].fill(255);
        assert!(
            open_bounded_zip(Cursor::new(&bytes))
                .unwrap_err()
                .contains("entry-count")
        );
    }

    #[test]
    fn ambiguous_windows_output_aliases_are_rejected() {
        for name in [
            "file.", "file ", "CON", "con.txt", "COM1.dat", "LPT9", "NUL", "A:stream",
        ] {
            assert!(!unambiguous_output_component(name));
        }
        assert!(unambiguous_output_component("console.txt"));
    }
}
