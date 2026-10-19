//! Minimal PNG codec for `imread`/`imwrite` — `flate2` supplies the zlib
//! layer, everything else is the PNG spec implemented directly (all
//! MIT/Apache-clean). Scope: 8-bit and 16-bit gray / RGB / palette /
//! gray+alpha / RGBA, non-interlaced. 16-bit data keeps its high byte;
//! alpha is decoded but dropped (imread RGB convention), low-bit-depth
//! gray/palette indices are unpacked and scaled to 0–255.

use std::io::Read;

use crate::LangError;

const SIG: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

fn bad(msg: impl Into<String>) -> LangError {
    LangError::new(format!("imread: {}", msg.into()), 0)
}

fn u32be(b: &[u8]) -> u32 {
    let mut a = [0u8; 4];
    a.copy_from_slice(b.get(..4).unwrap_or(&[]));
    u32::from_be_bytes(a)
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let (a, b, c) = (a as i32, b as i32, c as i32);
    let p = a + b - c;
    let (pa, pb, pc) = ((p - a).abs(), (p - b).abs(), (p - c).abs());
    if pa <= pb && pa <= pc {
        a as u8
    } else if pb <= pc {
        b as u8
    } else {
        c as u8
    }
}

/// Decode `bytes` into `(width, height, pixels, channels)` where
/// `channels` is 1 (gray) or 3 (RGB); pixels are row-major `0..=255`.
pub(crate) fn decode(bytes: &[u8]) -> Result<(usize, usize, Vec<u8>, usize), LangError> {
    if bytes.len() < 8 || &bytes[..8] != SIG {
        return Err(bad("not a PNG file"));
    }
    let (mut w, mut h, mut depth, mut ctype) = (0usize, 0usize, 0u8, 0u8);
    let mut interlace = 0u8;
    let mut idat: Vec<u8> = Vec::new();
    let mut plte: Vec<u8> = Vec::new();
    let mut pos = 8usize;
    while pos + 12 <= bytes.len() {
        let len = u32be(&bytes[pos..]) as usize;
        let tag = &bytes[pos + 4..pos + 8];
        let data = bytes.get(pos + 8..pos + 8 + len).ok_or_else(|| bad("truncated chunk"))?;
        match tag {
            b"IHDR" => {
                if data.len() < 13 {
                    return Err(bad("bad IHDR"));
                }
                w = u32be(data) as usize;
                h = u32be(&data[4..]) as usize;
                depth = data[8];
                ctype = data[9];
                if data[10] != 0 || data[11] != 0 {
                    return Err(bad("unsupported compression/filter method"));
                }
                interlace = data[12];
            }
            b"PLTE" => plte = data.to_vec(),
            b"IDAT" => idat.extend_from_slice(data),
            b"IEND" => break,
            _ => {}
        }
        pos += 12 + len;
    }
    if w == 0 || h == 0 {
        return Err(bad("missing IHDR"));
    }
    if interlace != 0 {
        return Err(bad("interlaced PNGs are not supported"));
    }
    // Channels and valid bit depths per the spec table.
    let ch = match ctype {
        0 | 3 => 1usize,
        2 => 3,
        4 => 2,
        6 => 4,
        _ => return Err(bad(format!("unknown colour type {ctype}"))),
    };
    let depth_ok = match ctype {
        0 => matches!(depth, 1 | 2 | 4 | 8 | 16),
        3 => matches!(depth, 1 | 2 | 4 | 8),
        2 | 4 | 6 => matches!(depth, 8 | 16),
        _ => false,
    };
    if !depth_ok {
        return Err(bad(format!("unsupported bit depth {depth} for colour type {ctype}")));
    }
    if ctype == 3 && plte.is_empty() {
        return Err(bad("palette image with no PLTE chunk"));
    }
    let bpp_bits = ch * depth as usize;
    let stride = (w * bpp_bits).div_ceil(8);
    let bpp = bpp_bits.div_ceil(8).max(1); // filter unit in bytes
    let raw = {
        let mut d = flate2::read::ZlibDecoder::new(&idat[..]);
        let mut out = Vec::new();
        d.read_to_end(&mut out).map_err(|e| bad(format!("zlib decode failed: {e}")))?;
        out
    };
    if raw.len() < (stride + 1) * h {
        return Err(bad("image data is truncated"));
    }
    // Unfilter scanlines into `px`.
    let mut px = vec![0u8; stride * h];
    for r in 0..h {
        let line = &raw[r * (stride + 1) + 1..r * (stride + 1) + 1 + stride];
        let filt = raw[r * (stride + 1)];
        let (done, rest) = px.split_at_mut(r * stride);
        let out = &mut rest[..stride];
        let prev = if r == 0 { &[0u8; 0][..] } else { &done[stride * (r - 1)..stride * r] };
        for (i, &x) in line.iter().enumerate() {
            let a = if i >= bpp { out[i - bpp] } else { 0 };
            let b = prev.get(i).copied().unwrap_or(0);
            let c = if i >= bpp { prev.get(i - bpp).copied().unwrap_or(0) } else { 0 };
            out[i] = match filt {
                0 => x,
                1 => x.wrapping_add(a),
                2 => x.wrapping_add(b),
                3 => x.wrapping_add(((a as u32 + b as u32) / 2) as u8),
                4 => x.wrapping_add(paeth(a, b, c)),
                _ => return Err(bad(format!("unknown filter {filt}"))),
            };
        }
    }
    // Expand to 8-bit gray or RGB.
    let lo_unpack = |row: &[u8], i: usize| -> usize {
        // Sub-byte index at pixel i (big-endian bit order per PNG).
        let bit = i * depth as usize;
        let byte = row.get(bit / 8).copied().unwrap_or(0) as usize;
        let shift = 8 - depth as usize - (bit % 8);
        (byte >> shift) & ((1 << depth) - 1)
    };
    let scale_lo = |v: usize| -> u8 { (v * 255 / ((1 << depth) - 1)) as u8 };
    let mut out = Vec::with_capacity(w * h * if ctype == 0 || ctype == 4 { 1 } else { 3 });
    for r in 0..h {
        let row = &px[r * stride..(r + 1) * stride];
        for x in 0..w {
            match ctype {
                0 => {
                    let g = match depth {
                        16 => row.get(2 * x).copied().unwrap_or(0),
                        8 => row.get(x).copied().unwrap_or(0),
                        _ => scale_lo(lo_unpack(row, x)),
                    };
                    out.push(g);
                }
                4 => {
                    let g = if depth == 16 { row.get(4 * x).copied().unwrap_or(0) } else { row.get(2 * x).copied().unwrap_or(0) };
                    out.push(g);
                }
                3 => {
                    let idx = if depth == 8 { row.get(x).copied().unwrap_or(0) as usize } else { lo_unpack(row, x) };
                    let base = 3 * idx;
                    out.push(plte.get(base).copied().unwrap_or(0));
                    out.push(plte.get(base + 1).copied().unwrap_or(0));
                    out.push(plte.get(base + 2).copied().unwrap_or(0));
                }
                2 | 6 => {
                    let step = ch * (depth as usize / 8);
                    let base = x * step;
                    for c in 0..3 {
                        out.push(row.get(base + c * (depth as usize / 8)).copied().unwrap_or(0));
                    }
                }
                _ => {}
            }
        }
    }
    Ok((w, h, out, if ctype == 0 || ctype == 4 { 1 } else { 3 }))
}

/// CRC-32 (PNG's polynomial), table-driven.
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in bytes {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 == 1 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

fn chunk(out: &mut Vec<u8>, tag: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(tag);
    out.extend_from_slice(data);
    let mut c = Vec::with_capacity(4 + data.len());
    c.extend_from_slice(tag);
    c.extend_from_slice(data);
    out.extend_from_slice(&crc32(&c).to_be_bytes());
}

/// Encode `pixels` (row-major, `channels` 1 = gray or 3 = RGB) as an
/// 8-bit non-interlaced PNG with filter-0 rows.
pub(crate) fn encode(w: usize, h: usize, pixels: &[u8], channels: usize) -> Result<Vec<u8>, LangError> {
    if channels != 1 && channels != 3 {
        return Err(LangError::new("imwrite: only gray and RGB images are supported", 0));
    }
    if w == 0 || h == 0 || pixels.len() < w * h * channels {
        return Err(LangError::new("imwrite: empty image", 0));
    }
    let mut raw = Vec::with_capacity((w * channels + 1) * h);
    for r in 0..h {
        raw.push(0); // filter: none
        raw.extend_from_slice(&pixels[r * w * channels..(r + 1) * w * channels]);
    }
    let compressed = {
        use std::io::Write;
        let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        e.write_all(&raw).map_err(|err| LangError::new(format!("imwrite: zlib failed: {err}"), 0))?;
        e.finish().map_err(|err| LangError::new(format!("imwrite: zlib failed: {err}"), 0))?
    };
    let mut out = Vec::with_capacity(compressed.len() + 64);
    out.extend_from_slice(SIG);
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&(w as u32).to_be_bytes());
    ihdr.extend_from_slice(&(h as u32).to_be_bytes());
    ihdr.push(8); // bit depth
    ihdr.push(if channels == 1 { 0 } else { 2 });
    ihdr.extend_from_slice(&[0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &compressed);
    chunk(&mut out, b"IEND", &[]);
    Ok(out)
}
