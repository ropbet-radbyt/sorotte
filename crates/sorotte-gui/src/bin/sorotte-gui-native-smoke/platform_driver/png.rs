use std::{fs, path::Path};

const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
const ADLER_MODULUS: u32 = 65_521;
const STORED_DEFLATE_BLOCK_LEN: usize = u16::MAX as usize;

pub(super) fn write_bgrx_png(
    output_path: &Path,
    width: u32,
    height: u32,
    bgrx_pixels: &[u8],
) -> Result<(), String> {
    let pixel_count = (width as usize)
        .checked_mul(height as usize)
        .ok_or_else(|| "captured window dimensions overflowed addressable memory".to_owned())?;
    let expected_len = pixel_count
        .checked_mul(4)
        .ok_or_else(|| "captured window pixel buffer length overflowed".to_owned())?;
    if width == 0 || height == 0 {
        return Err("captured window dimensions must be non-zero".to_owned());
    }
    if bgrx_pixels.len() != expected_len {
        return Err(format!(
            "captured window pixel buffer had {} bytes; expected {expected_len}",
            bgrx_pixels.len()
        ));
    }

    let scanline_len = (width as usize)
        .checked_mul(3)
        .and_then(|value| value.checked_add(1))
        .ok_or_else(|| "PNG scanline length overflowed".to_owned())?;
    let mut filtered = Vec::with_capacity(
        scanline_len
            .checked_mul(height as usize)
            .ok_or_else(|| "PNG image buffer length overflowed".to_owned())?,
    );
    for row in bgrx_pixels.chunks_exact(width as usize * 4) {
        filtered.push(0);
        for pixel in row.chunks_exact(4) {
            filtered.extend_from_slice(&[pixel[2], pixel[1], pixel[0]]);
        }
    }

    let mut png = Vec::with_capacity(filtered.len() + 256);
    png.extend_from_slice(PNG_SIGNATURE);

    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    append_chunk(&mut png, b"IHDR", &ihdr)?;

    let compressed = zlib_store(&filtered);
    append_chunk(&mut png, b"IDAT", &compressed)?;
    append_chunk(&mut png, b"IEND", &[])?;

    if let Some(parent) = output_path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "failed to create screenshot output directory {}: {error}",
                parent.display()
            )
        })?;
    }
    fs::write(output_path, png).map_err(|error| {
        format!(
            "failed to write native window screenshot {}: {error}",
            output_path.display()
        )
    })
}

fn append_chunk(output: &mut Vec<u8>, chunk_type: &[u8; 4], data: &[u8]) -> Result<(), String> {
    let len = u32::try_from(data.len())
        .map_err(|_| "PNG chunk exceeded the format's 32-bit length limit".to_owned())?;
    output.extend_from_slice(&len.to_be_bytes());
    output.extend_from_slice(chunk_type);
    output.extend_from_slice(data);
    let mut crc_input = Vec::with_capacity(chunk_type.len() + data.len());
    crc_input.extend_from_slice(chunk_type);
    crc_input.extend_from_slice(data);
    output.extend_from_slice(&crc32(&crc_input).to_be_bytes());
    Ok(())
}

fn zlib_store(input: &[u8]) -> Vec<u8> {
    let block_count = input.len().div_ceil(STORED_DEFLATE_BLOCK_LEN).max(1);
    let mut output = Vec::with_capacity(input.len() + block_count * 5 + 6);
    output.extend_from_slice(&[0x78, 0x01]);

    if input.is_empty() {
        output.extend_from_slice(&[1, 0, 0, 0xff, 0xff]);
    } else {
        let mut blocks = input.chunks(STORED_DEFLATE_BLOCK_LEN).peekable();
        while let Some(block) = blocks.next() {
            output.push(u8::from(blocks.peek().is_none()));
            let len = block.len() as u16;
            output.extend_from_slice(&len.to_le_bytes());
            output.extend_from_slice(&(!len).to_le_bytes());
            output.extend_from_slice(block);
        }
    }
    output.extend_from_slice(&adler32(input).to_be_bytes());
    output
}

fn adler32(input: &[u8]) -> u32 {
    let mut first = 1u32;
    let mut second = 0u32;
    for &byte in input {
        first = (first + u32::from(byte)) % ADLER_MODULUS;
        second = (second + first) % ADLER_MODULUS;
    }
    (second << 16) | first
}

fn crc32(input: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for &byte in input {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_encoder_emits_rgb_header_and_bgr_channel_order() {
        let unique = std::process::id();
        let path = std::env::temp_dir().join(format!("sorotte-window-capture-{unique}.png"));
        write_bgrx_png(&path, 1, 1, &[0x11, 0x22, 0x33, 0]).expect("PNG should encode");
        let bytes = fs::read(&path).expect("PNG should be readable");
        let _ = fs::remove_file(path);

        assert_eq!(&bytes[..8], PNG_SIGNATURE);
        assert_eq!(&bytes[12..16], b"IHDR");
        assert_eq!(&bytes[16..20], &1u32.to_be_bytes());
        assert_eq!(&bytes[20..24], &1u32.to_be_bytes());
        assert_eq!(&bytes[24..29], &[8, 2, 0, 0, 0]);

        let idat_offset = 33;
        assert_eq!(&bytes[idat_offset + 4..idat_offset + 8], b"IDAT");
        let idat_len = u32::from_be_bytes(
            bytes[idat_offset..idat_offset + 4]
                .try_into()
                .expect("IDAT length"),
        ) as usize;
        let idat = &bytes[idat_offset + 8..idat_offset + 8 + idat_len];
        assert_eq!(&idat[..2], &[0x78, 0x01]);
        assert_eq!(&idat[7..11], &[0, 0x33, 0x22, 0x11]);
    }

    #[test]
    fn png_encoder_rejects_mismatched_pixel_buffer() {
        let error = write_bgrx_png(Path::new("unused.png"), 2, 1, &[0; 4])
            .expect_err("short capture must fail");
        assert!(error.contains("expected 8"));
    }
}
