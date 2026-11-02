//! Pure-Rust HDF5 reader/writer for numeric datasets.
//!
//! Writer: superblock v2 (default) or v3, v2 object headers with link
//! messages for the root group, chunked little-endian f64 datasets and
//! fixed-array chunk indexes; `write_bytes_v0` keeps the older
//! symbol-table layout available for reference. Reader: superblocks
//! v0/v1/v2/v3, object headers v1/v2, symbol tables and link messages,
//! compact/contiguous/chunked data, fixed-array/extensible-array/B-tree
//! chunk indexes, virtual datasets and common filters.

use crate::LangError;

const UNDEF: u64 = u64::MAX;
/// Fixed group B-tree leaf size for K = 4 (HDF5 default).
const BTREE_NODE_SIZE: usize = 544;
/// Fixed symbol-table node size for K = 4.
const SNOD_NODE_SIZE: usize = 8 + 2 * 4 * 40;

#[derive(Clone, Debug)]
pub(crate) struct H5Dataset {
    pub name: String,
    pub dims: Vec<usize>,
    /// Row-major numeric payload.
    pub data: Vec<f64>,
}

#[derive(Clone, Debug)]
pub(crate) struct H5File {
    pub datasets: Vec<H5Dataset>,
    /// Superblock format to write: 2 (default, widest compatibility) or 3.
    pub superblock_version: u8,
}

impl Default for H5File {
    fn default() -> Self {
        H5File { datasets: Vec::new(), superblock_version: 2 }
    }
}

fn u16_at(b: &[u8], p: usize) -> Option<u16> {
    let s = b.get(p..p + 2)?;
    Some(u16::from_le_bytes([*s.first()?, *s.get(1)?]))
}

fn u32_at(b: &[u8], p: usize) -> Option<u32> {
    let s = b.get(p..p + 4)?;
    Some(u32::from_le_bytes([*s.first()?, *s.get(1)?, *s.get(2)?, *s.get(3)?]))
}

fn u64_at(b: &[u8], p: usize) -> Option<u64> {
    let s = b.get(p..p + 8)?;
    let mut a = [0u8; 8];
    for (i, slot) in a.iter_mut().enumerate() {
        *slot = *s.get(i)?;
    }
    Some(u64::from_le_bytes(a))
}

/// Jenkins lookup3 checksum used by HDF5 metadata blocks.
fn lookup3(data: &[u8]) -> u32 {
    fn mix(a: &mut u32, b: &mut u32, c: &mut u32) {
        *a = a.wrapping_sub(*c);
        *a ^= c.rotate_left(4);
        *c = c.wrapping_add(*b);
        *b = b.wrapping_sub(*a);
        *b ^= a.rotate_left(6);
        *a = a.wrapping_add(*c);
        *c = c.wrapping_sub(*b);
        *c ^= b.rotate_left(8);
        *b = b.wrapping_add(*a);
        *a = a.wrapping_sub(*c);
        *a ^= c.rotate_left(16);
        *c = c.wrapping_add(*b);
        *b = b.wrapping_sub(*a);
        *b ^= a.rotate_left(19);
        *a = a.wrapping_add(*c);
        *c = c.wrapping_sub(*b);
        *c ^= b.rotate_left(4);
        *b = b.wrapping_add(*a);
    }
    fn final_mix(a: &mut u32, b: &mut u32, c: &mut u32) {
        *c ^= *b;
        *c = c.wrapping_sub(b.rotate_left(14));
        *a ^= *c;
        *a = a.wrapping_sub(c.rotate_left(11));
        *b ^= *a;
        *b = b.wrapping_sub(a.rotate_left(25));
        *c ^= *b;
        *c = c.wrapping_sub(b.rotate_left(16));
        *a ^= *c;
        *a = a.wrapping_sub(c.rotate_left(4));
        *b ^= *a;
        *b = b.wrapping_sub(a.rotate_left(14));
        *c ^= *b;
        *c = c.wrapping_sub(b.rotate_left(24));
    }
    let n = data.len();
    let mut a = 0xdead_beefu32.wrapping_add(n as u32);
    let mut b = a;
    let mut c = a;
    let mut i = 0usize;
    while n.saturating_sub(i) > 12 {
        a = a.wrapping_add(u32_at(data, i).unwrap_or(0));
        b = b.wrapping_add(u32_at(data, i + 4).unwrap_or(0));
        c = c.wrapping_add(u32_at(data, i + 8).unwrap_or(0));
        mix(&mut a, &mut b, &mut c);
        i += 12;
    }
    for (j, &byte) in data.get(i..).unwrap_or(&[]).iter().enumerate() {
        match j {
            0..=3 => a = a.wrapping_add((byte as u32) << (8 * j)),
            4..=7 => b = b.wrapping_add((byte as u32) << (8 * (j - 4))),
            _ => c = c.wrapping_add((byte as u32) << (8 * (j - 8))),
        }
    }
    final_mix(&mut a, &mut b, &mut c);
    c
}

fn put_u16(b: &mut [u8], p: usize, v: u16) {
    if let Some(s) = b.get_mut(p..p + 2) {
        s.copy_from_slice(&v.to_le_bytes());
    }
}

fn put_u32(b: &mut [u8], p: usize, v: u32) {
    if let Some(s) = b.get_mut(p..p + 4) {
        s.copy_from_slice(&v.to_le_bytes());
    }
}

fn put_u64(b: &mut [u8], p: usize, v: u64) {
    if let Some(s) = b.get_mut(p..p + 8) {
        s.copy_from_slice(&v.to_le_bytes());
    }
}

fn pad8(n: usize) -> usize {
    (n + 7) & !7
}

fn dataspace_size(rank: usize) -> usize {
    8 + 16 * rank
}

fn object_header_size(rank: usize) -> usize {
    16 + (8 + pad8(dataspace_size(rank))) + (8 + 24) + (8 + 24) + 16 + (8 + 120)
}

fn write_object_header(buf: &mut [u8], addr: usize, messages: &[(u16, Vec<u8>)]) {
    let total: usize = messages.iter().map(|(_, d)| 8 + pad8(d.len())).sum();
    buf[addr] = 1;
    buf[addr + 1] = 0;
    put_u16(buf, addr + 2, messages.len() as u16);
    put_u32(buf, addr + 4, 1);
    put_u32(buf, addr + 8, total as u32);
    put_u32(buf, addr + 12, 0);
    let mut p = addr + 16;
    for (typ, data) in messages {
        let size = pad8(data.len());
        put_u16(buf, p, *typ);
        put_u16(buf, p + 2, size as u16);
        if let Some(slot) = buf.get_mut(p + 4) {
            *slot = if *typ == 0x0003 || *typ == 0x0005 { 1 } else { 0 };
        }
        for i in 0..3 {
            if let Some(slot) = buf.get_mut(p + 5 + i) {
                *slot = 0;
            }
        }
        for (i, &v) in data.iter().enumerate() {
            if let Some(slot) = buf.get_mut(p + 8 + i) {
                *slot = v;
            }
        }
        p += 8 + size;
    }
}

fn dataspace_message(dims: &[usize]) -> Vec<u8> {
    let rank = dims.len();
    let mut v = vec![0u8; dataspace_size(rank)];
    v[0] = 1;
    v[1] = rank as u8;
    v[2] = 1; // max dimensions present
    for (i, &d) in dims.iter().enumerate() {
        let p = 8 + i * 8;
        v.get_mut(p..p + 8).unwrap_or(&mut []).copy_from_slice(&(d as u64).to_le_bytes());
        let q = 8 + rank * 8 + i * 8;
        v.get_mut(q..q + 8).unwrap_or(&mut []).copy_from_slice(&(d as u64).to_le_bytes());
    }
    v
}

fn datatype_message() -> Vec<u8> {
    let mut v = vec![0u8; 24];
    v[0] = 0x11; // class floating-point, version 1
    v[1] = 0x20;
    v[2] = 0x3f;
    v[4] = 8;
    // bit offset 0, precision 64, exponent location 52, size 11,
    // mantissa location 0, size 52, bias 1023.
    let p = 8;
    v.get_mut(p..p + 2).unwrap_or(&mut []).copy_from_slice(&0u16.to_le_bytes());
    v.get_mut(p + 2..p + 4).unwrap_or(&mut []).copy_from_slice(&64u16.to_le_bytes());
    if let Some(slot) = v.get_mut(p + 4) {
        *slot = 52;
    }
    if let Some(slot) = v.get_mut(p + 5) {
        *slot = 11;
    }
    if let Some(slot) = v.get_mut(p + 6) {
        *slot = 0;
    }
    if let Some(slot) = v.get_mut(p + 7) {
        *slot = 52;
    }
    v.get_mut(p + 8..p + 12).unwrap_or(&mut []).copy_from_slice(&1023u32.to_le_bytes());
    v
}

fn layout_message(address: usize, size: usize) -> Vec<u8> {
    let mut v = vec![0u8; 24];
    v[0] = 3;
    v[1] = 1; // contiguous
    v.get_mut(2..10).unwrap_or(&mut []).copy_from_slice(&(address as u64).to_le_bytes());
    v.get_mut(10..18).unwrap_or(&mut []).copy_from_slice(&(size as u64).to_le_bytes());
    v
}

pub(crate) fn write(path: &std::path::Path, file: &H5File) -> Result<(), LangError> {
    let bytes = write_bytes(file)?;
    crate::write_file(path, &bytes).map_err(|e| LangError::new(format!("hdf5: cannot write {}: {e}", path.display()), 0))
}

fn append_uint(out: &mut Vec<u8>, v: u64, size: usize) {
    for i in 0..size {
        out.push(((v >> (8 * i)) & 0xff) as u8);
    }
}

fn place(out: &mut [u8], addr: usize, data: &[u8]) -> Result<(), LangError> {
    let dst = out.get_mut(addr..addr.saturating_add(data.len())).ok_or_else(|| LangError::new("hdf5: internal writer address overflow", 0))?;
    dst.copy_from_slice(data);
    Ok(())
}

fn build_oh_v2(messages: &[(u8, Vec<u8>)]) -> Result<Vec<u8>, LangError> {
    let payload: usize = messages.iter().map(|(_, d)| 4usize.saturating_add(d.len())).sum();
    if payload > u32::MAX as usize {
        return Err(LangError::new("hdf5: object header is too large", 0));
    }
    let mut out = Vec::with_capacity(10 + payload + 4);
    out.extend_from_slice(b"OHDR");
    out.push(2);
    out.push(0x02); // chunk0 size encoded in 4 bytes
    append_uint(&mut out, payload as u64, 4);
    for (typ, data) in messages {
        if data.len() > u16::MAX as usize {
            return Err(LangError::new("hdf5: object header message is too large", 0));
        }
        out.push(*typ);
        append_uint(&mut out, data.len() as u64, 2);
        out.push(0); // message flags
        out.extend_from_slice(data);
    }
    let checksum = lookup3(&out);
    append_uint(&mut out, checksum as u64, 4);
    Ok(out)
}

fn link_message(name: &str, addr: u64) -> Vec<u8> {
    let mut data = Vec::with_capacity(11 + name.len());
    data.push(1); // link message version
    data.push(0); // flags: hard link, default charset, 1-byte name length
    data.push(name.len().min(255) as u8);
    data.extend_from_slice(name.as_bytes());
    append_uint(&mut data, addr, 8);
    data
}

fn root_oh_v2(links: &[(String, u64)]) -> Result<Vec<u8>, LangError> {
    let mut msgs: Vec<(u8, Vec<u8>)> = Vec::new();
    // Link info: version 0, flags 0, undefined fractal heap and name index.
    let mut info = vec![0, 0];
    append_uint(&mut info, UNDEF, 8);
    append_uint(&mut info, UNDEF, 8);
    msgs.push((0x0002, info));
    // Group info: version 0, flags 0.
    msgs.push((0x000a, vec![0, 0]));
    for (name, addr) in links {
        msgs.push((0x0006, link_message(name, *addr)));
    }
    build_oh_v2(&msgs)
}

struct ChunkPlan {
    chunk_dims: Vec<usize>,
    blobs: Vec<Vec<u8>>,
}

fn plan_chunks(ds: &H5Dataset) -> Result<ChunkPlan, LangError> {
    let rank = ds.dims.len();
    if rank == 0 || rank > 2 {
        return Err(LangError::new("hdf5: only 1-D and 2-D datasets are supported", 0));
    }
    if ds.dims.contains(&0) {
        return Err(LangError::new("hdf5: empty datasets cannot be chunked", 0));
    }
    let count = ds.dims.iter().fold(1usize, |a, &d| a.saturating_mul(d));
    if count > 50_000_000 {
        return Err(LangError::new("hdf5: dataset is too large for the in-memory writer", 0));
    }
    let mut chunk_dims: Vec<usize> = ds.dims.iter().map(|&d| d.clamp(1, 64)).collect();
    loop {
        let chunks = chunk_dims.iter().zip(&ds.dims).fold(1usize, |a, (&c, &d)| a.saturating_mul(d.div_ceil(c)));
        if chunks <= 65_536 {
            break;
        }
        let mut grown = false;
        for i in 0..chunk_dims.len() {
            let c = chunk_dims.get(i).copied().unwrap_or(1);
            let d = ds.dims.get(i).copied().unwrap_or(1);
            if c < d {
                if let Some(slot) = chunk_dims.get_mut(i) {
                    *slot = (c.saturating_mul(2)).min(d);
                }
                grown = true;
                break;
            }
        }
        if !grown {
            return Err(LangError::new("hdf5: dataset needs more than 65536 chunks", 0));
        }
    }
    let chunk_elems = chunk_dims.iter().fold(1usize, |a, &d| a.saturating_mul(d));
    let grid: Vec<usize> = chunk_dims.iter().zip(&ds.dims).map(|(&c, &d)| d.div_ceil(c)).collect();
    let total_chunks = grid.iter().fold(1usize, |a, &d| a.saturating_mul(d));
    let mut blobs = Vec::with_capacity(total_chunks);
    for index in 0..total_chunks {
        let mut rem = index;
        let mut offsets = vec![0usize; rank];
        for d in (0..rank).rev() {
            let n = grid.get(d).copied().unwrap_or(1).max(1);
            offsets[d] = (rem % n).saturating_mul(chunk_dims.get(d).copied().unwrap_or(1));
            rem /= n;
        }
        let mut raw = Vec::with_capacity(chunk_elems.saturating_mul(8));
        for li in 0..chunk_elems {
            let mut r = li;
            let mut local = vec![0usize; rank];
            for d in (0..rank).rev() {
                let cd = chunk_dims.get(d).copied().unwrap_or(1).max(1);
                local[d] = r % cd;
                r /= cd;
            }
            let mut value = 0.0f64;
            let mut in_bounds = true;
            let mut flat = 0usize;
            for d in 0..rank {
                let global = offsets.get(d).copied().unwrap_or(0).saturating_add(local.get(d).copied().unwrap_or(0));
                if global >= ds.dims.get(d).copied().unwrap_or(0) {
                    in_bounds = false;
                    break;
                }
                flat = flat.saturating_mul(ds.dims.get(d).copied().unwrap_or(0)).saturating_add(global);
            }
            if in_bounds {
                value = ds.data.get(flat).copied().unwrap_or(0.0);
            }
            raw.extend_from_slice(&value.to_le_bytes());
        }
        use std::io::Write;
        let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(&raw).map_err(|e| LangError::new(format!("hdf5: deflate error: {e}"), 0))?;
        let compressed = enc.finish().map_err(|e| LangError::new(format!("hdf5: deflate error: {e}"), 0))?;
        blobs.push(compressed);
    }
    Ok(ChunkPlan { chunk_dims, blobs })
}

fn dataset_oh_v2(ds: &H5Dataset, chunk_dims: &[usize], index_addr: u64) -> Result<Vec<u8>, LangError> {
    let rank = ds.dims.len();
    let mut dataspace = vec![2, rank as u8, 0, 1];
    for &d in &ds.dims {
        append_uint(&mut dataspace, d as u64, 8);
    }
    let datatype = datatype_message();
    let filter = vec![2, 1, 1, 0, 1, 0, 1, 0, 4, 0, 0, 0];
    // HDF5 2.0 layout v5: ndims includes the trailing element-size
    // dimension, matching the chunk keys written by libhdf5.
    let max_dim = chunk_dims.iter().copied().max().unwrap_or(1).max(8);
    let enc = if max_dim <= 0xff {
        1usize
    } else if max_dim <= 0xffff {
        2
    } else if max_dim <= 0xffff_ffff {
        4
    } else {
        8
    };
    let mut layout = vec![5, 2, 0, (rank + 1) as u8, enc as u8];
    for &c in chunk_dims {
        append_uint(&mut layout, c as u64, enc);
    }
    append_uint(&mut layout, 8, enc);
    layout.push(3); // fixed-array chunk index
    layout.push(16); // max data-block page elements bits (one page up to 65536 entries)
    append_uint(&mut layout, index_addr, 8);
    build_oh_v2(&[(0x0001, dataspace), (0x0003, datatype), (0x000b, filter), (0x0008, layout)])
}

pub(crate) fn write_bytes(file: &H5File) -> Result<Vec<u8>, LangError> {
    if file.datasets.len() > 64 {
        return Err(LangError::new("hdf5: the writer supports at most 64 root datasets", 0));
    }
    if file.datasets.iter().any(|d| d.name.len() > 255) {
        return Err(LangError::new("hdf5: dataset names are limited to 255 bytes", 0));
    }
    let plans: Vec<ChunkPlan> = file.datasets.iter().map(plan_chunks).collect::<Result<_, _>>()?;
    let root_size = root_oh_v2(&file.datasets.iter().map(|d| (d.name.clone(), 0)).collect::<Vec<_>>())?.len();
    let mut cursor = 48usize.saturating_add(root_size);
    while cursor % 8 != 0 {
        cursor += 1;
    }
    let mut oh_addrs = Vec::with_capacity(file.datasets.len());
    for (ds, plan) in file.datasets.iter().zip(&plans) {
        oh_addrs.push(cursor);
        let oh = dataset_oh_v2(ds, &plan.chunk_dims, 0)?;
        cursor = cursor.saturating_add(oh.len());
        while cursor % 8 != 0 {
            cursor += 1;
        }
    }
    let mut fahd_addrs = Vec::with_capacity(file.datasets.len());
    let mut fadb_addrs = Vec::with_capacity(file.datasets.len());
    for plan in &plans {
        fahd_addrs.push(cursor);
        cursor = cursor.saturating_add(28);
        while cursor % 8 != 0 {
            cursor += 1;
        }
        fadb_addrs.push(cursor);
        let entries = plan.blobs.len().saturating_mul(20);
        cursor = cursor.saturating_add(18 + entries);
        while cursor % 8 != 0 {
            cursor += 1;
        }
    }
    let mut chunk_addrs: Vec<Vec<u64>> = Vec::with_capacity(file.datasets.len());
    for plan in &plans {
        let mut addrs = Vec::with_capacity(plan.blobs.len());
        for blob in &plan.blobs {
            addrs.push(cursor as u64);
            cursor = cursor.saturating_add(blob.len());
            while cursor % 8 != 0 {
                cursor += 1;
            }
        }
        chunk_addrs.push(addrs);
    }
    let total = cursor.max(48);
    let mut out = vec![0u8; total];
    // Superblock v2 or v3 — identical fixed fields; the version selects
    // the HDF5 library-version semantics.
    let sb_version = if file.superblock_version == 3 { 3 } else { 2 };
    out.get_mut(0..8).unwrap_or(&mut []).copy_from_slice(b"\x89HDF\r\n\x1a\n");
    out[8] = sb_version;
    out[9] = 8;
    out[10] = 8;
    out[11] = 0;
    put_u64(&mut out, 12, 0);
    put_u64(&mut out, 20, UNDEF);
    put_u64(&mut out, 28, total as u64);
    put_u64(&mut out, 36, 48);
    let sb_sum = lookup3(&out[..44]);
    put_u32(&mut out, 44, sb_sum);
    // Root group.
    let links: Vec<(String, u64)> = file.datasets.iter().zip(&oh_addrs).map(|(d, &a)| (d.name.clone(), a as u64)).collect();
    let root = root_oh_v2(&links)?;
    place(&mut out, 48, &root)?;
    // Dataset headers.
    for (i, (ds, plan)) in file.datasets.iter().zip(&plans).enumerate() {
        let addr = oh_addrs.get(i).copied().unwrap_or(0);
        let oh = dataset_oh_v2(ds, &plan.chunk_dims, fahd_addrs.get(i).copied().unwrap_or(0) as u64)?;
        place(&mut out, addr, &oh)?;
    }
    // Fixed-array indexes.
    for (i, plan) in plans.iter().enumerate() {
        let fahd = fahd_addrs.get(i).copied().unwrap_or(0);
        let fadb = fadb_addrs.get(i).copied().unwrap_or(0);
        let mut header = Vec::with_capacity(28);
        header.extend_from_slice(b"FAHD");
        header.extend_from_slice(&[0, 1, 20, 16]);
        append_uint(&mut header, plan.blobs.len() as u64, 8);
        append_uint(&mut header, fadb as u64, 8);
        let sum = lookup3(&header);
        append_uint(&mut header, sum as u64, 4);
        place(&mut out, fahd, &header)?;
        let mut block = Vec::with_capacity(18 + plan.blobs.len() * 20);
        block.extend_from_slice(b"FADB");
        block.extend_from_slice(&[0, 1]);
        append_uint(&mut block, fahd as u64, 8);
        for (j, blob) in plan.blobs.iter().enumerate() {
            append_uint(&mut block, chunk_addrs.get(i).and_then(|v| v.get(j)).copied().unwrap_or(0), 8);
            append_uint(&mut block, blob.len() as u64, 8);
            append_uint(&mut block, 0, 4);
        }
        let sum = lookup3(&block);
        append_uint(&mut block, sum as u64, 4);
        place(&mut out, fadb, &block)?;
    }
    // Chunk payloads.
    for (i, plan) in plans.iter().enumerate() {
        for (j, blob) in plan.blobs.iter().enumerate() {
            let addr = chunk_addrs.get(i).and_then(|v| v.get(j)).copied().unwrap_or(0) as usize;
            place(&mut out, addr, blob)?;
        }
    }
    Ok(out)
}

#[allow(dead_code)]
fn write_bytes_v0(file: &H5File) -> Result<Vec<u8>, LangError> {
    if file.datasets.len() > 8 {
        return Err(LangError::new("hdf5: the minimal writer supports at most 8 root datasets", 0));
    }
    let root_oh = 96usize;
    let btree_addr = root_oh + 40;
    let heap_addr = btree_addr + BTREE_NODE_SIZE;
    let heap_data_addr = heap_addr + 32;
    // Local heap: offset 0 is the empty link name, names follow, then one
    // free block (next = 1 / H5HL_FREE_NULL, size = remaining bytes).
    let mut heap = vec![0u8; 8];
    let mut name_offsets = Vec::new();
    for ds in &file.datasets {
        name_offsets.push(heap.len() as u64);
        heap.extend_from_slice(ds.name.as_bytes());
        heap.push(0);
    }
    let free_off = pad8(heap.len());
    let heap_size = free_off.saturating_add(16).max(88);
    heap.resize(heap_size, 0);
    put_u64(&mut heap, free_off, 1);
    put_u64(&mut heap, free_off + 8, (heap_size - free_off) as u64);
    let mut oh_addrs = Vec::with_capacity(file.datasets.len());
    let mut cursor = heap_data_addr + heap_size;
    for ds in &file.datasets {
        let rank = ds.dims.len();
        if rank == 0 || rank > 2 {
            return Err(LangError::new("hdf5: only 1-D and 2-D datasets are supported", 0));
        }
        oh_addrs.push(cursor);
        cursor = cursor.saturating_add(object_header_size(rank));
    }
    let snod_addr = cursor;
    let data_base = snod_addr + SNOD_NODE_SIZE;
    let mut data_addrs = Vec::new();
    cursor = data_base;
    for ds in &file.datasets {
        data_addrs.push(cursor);
        cursor = cursor.saturating_add(ds.data.len().saturating_mul(8));
    }
    let total = cursor.max(data_base);
    let mut buf = vec![0u8; total];
    // Superblock v0.
    buf.get_mut(0..8).unwrap_or(&mut []).copy_from_slice(b"\x89HDF\r\n\x1a\n");
    buf[8] = 0;
    buf[9] = 0;
    buf[10] = 0;
    buf[11] = 0;
    buf[12] = 0;
    buf[13] = 8;
    buf[14] = 8;
    buf[15] = 0;
    put_u16(&mut buf, 16, 4);
    put_u16(&mut buf, 18, 16);
    put_u32(&mut buf, 20, 0);
    put_u64(&mut buf, 24, 0);
    put_u64(&mut buf, 32, UNDEF);
    put_u64(&mut buf, 40, total as u64);
    put_u64(&mut buf, 48, UNDEF);
    put_u64(&mut buf, 56, 0);
    put_u64(&mut buf, 64, root_oh as u64);
    put_u32(&mut buf, 72, 0);
    put_u32(&mut buf, 76, 0);
    // Root object header: symbol table message only.
    write_object_header(
        &mut buf,
        root_oh,
        &[(0x0011, {
            let mut v = Vec::with_capacity(16);
            v.extend_from_slice(&(btree_addr as u64).to_le_bytes());
            v.extend_from_slice(&(heap_addr as u64).to_le_bytes());
            v
        })],
    );
    // Dataset object headers.
    for (i, ds) in file.datasets.iter().enumerate() {
        let messages = vec![
            (0x0001, dataspace_message(&ds.dims)),
            (0x0003, datatype_message()),
            (0x0005, vec![2, 2, 2, 1, 0, 0, 0, 0]),
            (0x0008, layout_message(data_addrs[i], ds.data.len().saturating_mul(8))),
            (0x0000, vec![0u8; 120]),
        ];
        write_object_header(&mut buf, oh_addrs[i], &messages);
        let start = data_addrs[i];
        for (j, v) in ds.data.iter().enumerate() {
            let p = start.saturating_add(j.saturating_mul(8));
            put_u64(&mut buf, p, v.to_bits());
        }
    }
    // Group B-tree (single leaf child).
    buf.get_mut(btree_addr..btree_addr + 4).unwrap_or(&mut []).copy_from_slice(b"TREE");
    buf[btree_addr + 4] = 0;
    buf[btree_addr + 5] = 0;
    put_u16(&mut buf, btree_addr + 6, 1);
    put_u64(&mut buf, btree_addr + 8, UNDEF);
    put_u64(&mut buf, btree_addr + 16, UNDEF);
    put_u64(&mut buf, btree_addr + 24, 0);
    put_u64(&mut buf, btree_addr + 32, snod_addr as u64);
    put_u64(&mut buf, btree_addr + 40, name_offsets.last().copied().unwrap_or(0));
    // Symbol table node.
    buf.get_mut(snod_addr..snod_addr + 4).unwrap_or(&mut []).copy_from_slice(b"SNOD");
    buf[snod_addr + 4] = 1;
    buf[snod_addr + 5] = 0;
    put_u16(&mut buf, snod_addr + 6, file.datasets.len() as u16);
    for i in 0..file.datasets.len() {
        let p = snod_addr + 8 + i * 40;
        put_u64(&mut buf, p, name_offsets[i]);
        put_u64(&mut buf, p + 8, oh_addrs[i] as u64);
        put_u32(&mut buf, p + 16, 0);
        put_u32(&mut buf, p + 20, 0);
    }
    // Local heap.
    buf.get_mut(heap_addr..heap_addr + 4).unwrap_or(&mut []).copy_from_slice(b"HEAP");
    buf[heap_addr + 4] = 0;
    put_u64(&mut buf, heap_addr + 8, heap_size as u64);
    put_u64(&mut buf, heap_addr + 16, free_off as u64);
    put_u64(&mut buf, heap_addr + 24, heap_data_addr as u64);
    for (i, &v) in heap.iter().enumerate() {
        if let Some(slot) = buf.get_mut(heap_data_addr + i) {
            *slot = v;
        }
    }
    Ok(buf)
}

#[derive(Clone, Copy)]
struct H5Ctx {
    offset_size: usize,
    length_size: usize,
}

impl Default for H5Ctx {
    fn default() -> Self {
        H5Ctx { offset_size: 8, length_size: 8 }
    }
}

#[derive(Clone)]
struct H5Filter {
    id: u16,
    client: Vec<u32>,
}

enum H5Layout {
    Compact(Vec<u8>),
    Contiguous {
        addr: usize,
        size: usize,
    },
    Chunked {
        rank: usize,
        chunk_dims: Vec<usize>,
        elem_size: usize,
        index: H5ChunkIndex,
    },
    /// Virtual storage: a global-heap address plus object index holding the
    /// mapping list.
    Virtual {
        addr: usize,
        index: usize,
    },
}

enum H5ChunkIndex {
    FixedArray { addr: usize },
    BTreeV1 { addr: usize },
    Single { addr: usize, filtered_size: Option<usize>, mask: u32 },
    Implicit { addr: usize },
    ExtensibleArray { addr: usize },
    BTreeV2 { addr: usize, version: u8 },
    Unsupported(u8),
}

struct H5ChunkRecord {
    offset: Vec<usize>,
    addr: usize,
    nbytes: usize,
    mask: u32,
}

fn read_uint(b: &[u8], p: usize, size: usize) -> Option<u64> {
    if size == 0 || size > 8 {
        return None;
    }
    let s = b.get(p..p + size)?;
    let mut v = 0u64;
    for (i, &x) in s.iter().enumerate() {
        v |= (x as u64) << (8 * i);
    }
    Some(v)
}

fn off_at(b: &[u8], p: usize, ctx: H5Ctx) -> usize {
    read_uint(b, p, ctx.offset_size).unwrap_or(0) as usize
}

fn len_at(b: &[u8], p: usize, ctx: H5Ctx) -> usize {
    read_uint(b, p, ctx.length_size).unwrap_or(0) as usize
}

pub(crate) fn read(path: &std::path::Path) -> Result<H5File, LangError> {
    let bytes = std::fs::read(path).map_err(|e| LangError::new(format!("hdf5: cannot read {}: {e}", path.display()), 0))?;
    let base = path.parent().map(|p| p.to_path_buf());
    read_bytes_with(&bytes, base, 0)
}

#[cfg(test)]
pub(crate) fn read_bytes(b: &[u8]) -> Result<H5File, LangError> {
    read_bytes_with(b, None, 0)
}

#[derive(Clone, Default)]
struct H5ReadCtx {
    /// Directory used to resolve relative virtual-dataset source files.
    base: Option<std::path::PathBuf>,
    /// VDS resolution depth, so a self-referential mapping cannot recurse
    /// forever.
    depth: usize,
    /// When false, virtual layouts are skipped (used for same-file source
    /// lookups, which must not re-enter the VDS being resolved).
    follow_virtual: bool,
}

fn read_bytes_with(b: &[u8], base: Option<std::path::PathBuf>, depth: usize) -> Result<H5File, LangError> {
    read_bytes_opts(b, base, depth, true)
}

fn read_bytes_opts(b: &[u8], base: Option<std::path::PathBuf>, depth: usize, follow_virtual: bool) -> Result<H5File, LangError> {
    let (sb, version) = if b.get(0..8) == Some(b"\x89HDF\r\n\x1a\n") {
        (0usize, *b.get(8).ok_or_else(|| LangError::new("hdf5: truncated superblock", 0))?)
    } else if b.get(512..520) == Some(b"\x89HDF\r\n\x1a\n") {
        (512usize, *b.get(520).ok_or_else(|| LangError::new("hdf5: truncated superblock", 0))?)
    } else {
        return Err(LangError::new("hdf5: bad HDF5 signature", 0));
    };
    let rctx = H5ReadCtx { base, depth, follow_virtual };
    let mut file = match version {
        0 | 1 => {
            let ctx =
                H5Ctx { offset_size: b.get(sb + 13).copied().unwrap_or(8) as usize, length_size: b.get(sb + 14).copied().unwrap_or(8) as usize };
            let root_oh = off_at(b, sb + 64, ctx);
            read_root_v1(b, ctx, root_oh, &rctx)?
        }
        2 | 3 => {
            let ctx = H5Ctx { offset_size: b.get(sb + 9).copied().unwrap_or(8) as usize, length_size: b.get(sb + 10).copied().unwrap_or(8) as usize };
            let root_oh = off_at(b, sb + 36, ctx);
            read_root_v2(b, ctx, root_oh, &rctx)?
        }
        other => return Err(LangError::new(format!("hdf5: unsupported superblock version {other}"), 0)),
    };
    // v0/v1 files are rewritten with a v2 superblock; v2 and v3 are
    // preserved on save so a read/write round trip does not change format.
    file.superblock_version = if version == 3 { 3 } else { 2 };
    Ok(file)
}

fn read_root_v1(b: &[u8], ctx: H5Ctx, root_oh: usize, rctx: &H5ReadCtx) -> Result<H5File, LangError> {
    let mut file = H5File::default();
    let mut symbols = Vec::new();
    let mut heap_addr = 0usize;
    let mut btree_addr = 0usize;
    walk_object_header(b, root_oh, ctx, &mut |typ, data, _| {
        if typ == 0x0011 && data.len() >= 16 {
            btree_addr = off_at(b, data.start, ctx);
            heap_addr = off_at(b, data.start + ctx.offset_size, ctx);
        }
        Ok(())
    })?;
    if btree_addr == 0 || heap_addr == 0 {
        // Superblock v0 files can still use a v2-style root group.
        return read_root_v2(b, ctx, root_oh, rctx);
    }
    let entries = u16_at(b, btree_addr + 6).unwrap_or(0) as usize;
    for i in 0..entries {
        let child = off_at(b, btree_addr + 24 + i * (ctx.offset_size + 8) + ctx.offset_size, ctx);
        let nsyms = u16_at(b, child + 6).unwrap_or(0) as usize;
        for j in 0..nsyms {
            let p = child + 8 + j * 40;
            let name_off = u64_at(b, p).unwrap_or(0) as usize;
            let oh = off_at(b, p + 8, ctx);
            symbols.push((name_off, oh));
        }
    }
    let heap_data = off_at(b, heap_addr + 24, ctx);
    for (name_off, oh) in symbols {
        let start = heap_data.saturating_add(name_off);
        let mut end = start;
        while end < b.len() && b.get(end).copied().unwrap_or(0) != 0 {
            end += 1;
        }
        let name = String::from_utf8_lossy(b.get(start..end).unwrap_or(&[])).to_string();
        if let Some(ds) = parse_dataset(b, ctx, rctx, oh, name)? {
            file.datasets.push(ds);
        }
    }
    Ok(file)
}

fn read_root_v2(b: &[u8], ctx: H5Ctx, root_oh: usize, rctx: &H5ReadCtx) -> Result<H5File, LangError> {
    let mut file = H5File::default();
    let mut links: Vec<(String, usize)> = Vec::new();
    walk_object_header(b, root_oh, ctx, &mut |typ, data, _| {
        if typ == 0x0006 {
            if let Some((name, addr)) = parse_link(b, ctx, data.start, data.end) {
                links.push((name, addr));
            }
        }
        Ok(())
    })?;
    if links.is_empty() {
        return Err(LangError::new("hdf5: root group has no links", 0));
    }
    for (name, oh) in links {
        if let Some(ds) = parse_dataset(b, ctx, rctx, oh, name)? {
            file.datasets.push(ds);
        }
    }
    Ok(file)
}

fn parse_link(b: &[u8], ctx: H5Ctx, start: usize, end: usize) -> Option<(String, usize)> {
    if start >= end {
        return None;
    }
    let version = b.get(start).copied()?;
    if version != 1 {
        return None;
    }
    let flags = b.get(start + 1).copied()?;
    let mut p = start + 2;
    if flags & 0x08 != 0 {
        p += 1;
    }
    if flags & 0x04 != 0 {
        p += 8;
    }
    if flags & 0x10 != 0 {
        p += 1;
    }
    let name_len_size = 1usize << (flags & 0x03);
    let name_len = read_uint(b, p, name_len_size)? as usize;
    p += name_len_size;
    let name = String::from_utf8_lossy(b.get(p..p.checked_add(name_len)?).unwrap_or(&[])).to_string();
    p += name_len;
    let addr = off_at(b, p, ctx);
    if addr == 0 || addr == usize::MAX || p + ctx.offset_size > end {
        return None;
    }
    Some((name, addr))
}

fn walk_object_header(
    b: &[u8],
    addr: usize,
    ctx: H5Ctx,
    f: &mut impl FnMut(u16, std::ops::Range<usize>, usize) -> Result<(), LangError>,
) -> Result<(), LangError> {
    if b.get(addr..addr + 4) == Some(b"OHDR") {
        walk_object_header_v2(b, addr, ctx, f)
    } else {
        walk_object_header_v1(b, addr, f)
    }
}

fn walk_object_header_v1(
    b: &[u8],
    addr: usize,
    f: &mut impl FnMut(u16, std::ops::Range<usize>, usize) -> Result<(), LangError>,
) -> Result<(), LangError> {
    if b.get(addr) != Some(&1) {
        return Err(LangError::new("hdf5: unsupported object header", 0));
    }
    let n = u16_at(b, addr + 2).unwrap_or(0) as usize;
    let mut p = addr + 16;
    for _ in 0..n {
        let typ = u16_at(b, p).unwrap_or(0);
        let size = u16_at(b, p + 2).unwrap_or(0) as usize;
        let end = p.saturating_add(8).saturating_add(size);
        if end > b.len() {
            return Err(LangError::new("hdf5: truncated object header message", 0));
        }
        f(typ, p + 8..end, p)?;
        p = end;
    }
    Ok(())
}

fn walk_object_header_v2(
    b: &[u8],
    addr: usize,
    ctx: H5Ctx,
    f: &mut impl FnMut(u16, std::ops::Range<usize>, usize) -> Result<(), LangError>,
) -> Result<(), LangError> {
    if b.get(addr..addr + 4) != Some(b"OHDR") || b.get(addr + 4) != Some(&2) {
        return Err(LangError::new("hdf5: unsupported object header version", 0));
    }
    let flags = b.get(addr + 5).copied().unwrap_or(0);
    let mut p = addr + 6;
    if flags & 0x20 != 0 {
        p += 16;
    }
    if flags & 0x10 != 0 {
        p += 4;
    }
    let size_len = 1usize << (flags & 0x03);
    let chunk0 = read_uint(b, p, size_len).unwrap_or(0) as usize;
    p += size_len;
    let end = p.saturating_add(chunk0).min(b.len());
    walk_v2_messages(b, p, end, flags, ctx, f, 0)
}

fn walk_v2_messages(
    b: &[u8],
    start: usize,
    end: usize,
    oh_flags: u8,
    ctx: H5Ctx,
    f: &mut impl FnMut(u16, std::ops::Range<usize>, usize) -> Result<(), LangError>,
    depth: usize,
) -> Result<(), LangError> {
    if depth > 8 {
        return Err(LangError::new("hdf5: object header continuation nesting is too deep", 0));
    }
    let mut p = start;
    while p + 4 <= end {
        let typ = b.get(p).copied().unwrap_or(0) as u16;
        let size = u16_at(b, p + 1).unwrap_or(0) as usize;
        let hdr = 4 + if oh_flags & 0x04 != 0 { 2 } else { 0 };
        let ds = p + hdr;
        let de = ds.saturating_add(size);
        if de > end || de > b.len() {
            break;
        }
        if typ == 0x0010 {
            let coff = off_at(b, ds, ctx);
            let clen = len_at(b, ds + ctx.offset_size, ctx);
            if clen >= 8 && b.get(coff..coff + 4) == Some(b"OCHK") {
                walk_v2_messages(b, coff + 4, coff + clen - 4, oh_flags, ctx, f, depth + 1)?;
            }
        } else {
            f(typ, ds..de, p)?;
        }
        p = de;
    }
    Ok(())
}

fn parse_layout(data: &[u8], ctx: H5Ctx, expected_rank: usize) -> Result<H5Layout, LangError> {
    if data.len() < 2 {
        return Err(LangError::new("hdf5: layout message is truncated", 0));
    }
    let version = data[0];
    let class = data[1];
    match class {
        0 => {
            let n = u16_at(data, 2).unwrap_or(0) as usize;
            let bytes = data.get(4..4 + n).ok_or_else(|| LangError::new("hdf5: compact layout is truncated", 0))?.to_vec();
            Ok(H5Layout::Compact(bytes))
        }
        1 => {
            let addr = read_uint(data, 2, ctx.offset_size).unwrap_or(0) as usize;
            let size = read_uint(data, 2 + ctx.offset_size, ctx.length_size).unwrap_or(0) as usize;
            Ok(H5Layout::Contiguous { addr, size })
        }
        2 => {
            if version == 3 {
                let rank = data.get(2).copied().unwrap_or(0) as usize;
                if rank == 0 {
                    return Err(LangError::new("hdf5: chunked layout has zero rank", 0));
                }
                let addr = read_uint(data, 3, ctx.offset_size).unwrap_or(0) as usize;
                let mut dims = Vec::with_capacity(rank);
                for i in 0..rank {
                    let p = 3 + ctx.offset_size + i * 4;
                    dims.push(u32_at(data, p).unwrap_or(0) as usize);
                }
                let elem_size = dims.pop().unwrap_or(0);
                Ok(H5Layout::Chunked { rank: rank.saturating_sub(1), chunk_dims: dims, elem_size, index: H5ChunkIndex::BTreeV1 { addr } })
            } else if version == 4 || version == 5 {
                let flags = data.get(2).copied().unwrap_or(0);
                let rank = data.get(3).copied().unwrap_or(0) as usize;
                let enc = data.get(4).copied().unwrap_or(0) as usize;
                if rank == 0 || enc == 0 {
                    return Err(LangError::new("hdf5: chunked layout has invalid rank", 0));
                }
                let ndims = rank;
                let mut dims = Vec::with_capacity(ndims);
                for i in 0..ndims {
                    let p = 5 + i * enc;
                    dims.push(read_uint(data, p, enc).unwrap_or(0) as usize);
                }
                let has_elem = expected_rank == 0 || ndims > expected_rank;
                let elem_size = if has_elem { dims.pop().unwrap_or(0) } else { 0 };
                let rank = dims.len();
                let mut p = 5 + ndims * enc;
                let index_type = data.get(p).copied().unwrap_or(0);
                p += 1;
                let index = match index_type {
                    1 => {
                        let mut filtered_size = None;
                        let mut mask = 0u32;
                        if flags & 0x02 != 0 {
                            filtered_size = Some(read_uint(data, p, ctx.length_size).unwrap_or(0) as usize);
                            p += ctx.length_size;
                            mask = u32_at(data, p).unwrap_or(0);
                            p += 4;
                        }
                        H5ChunkIndex::Single { addr: read_uint(data, p, ctx.offset_size).unwrap_or(0) as usize, filtered_size, mask }
                    }
                    2 => H5ChunkIndex::Implicit { addr: read_uint(data, p, ctx.offset_size).unwrap_or(0) as usize },
                    3 => {
                        p += 1;
                        H5ChunkIndex::FixedArray { addr: read_uint(data, p, ctx.offset_size).unwrap_or(0) as usize }
                    }
                    4 => {
                        p += 5; // max bits, index elements, min pointers, min elements, page bits
                        H5ChunkIndex::ExtensibleArray { addr: read_uint(data, p, ctx.offset_size).unwrap_or(0) as usize }
                    }
                    5 => {
                        p += 6; // node size, split %, merge %
                        H5ChunkIndex::BTreeV2 { addr: read_uint(data, p, ctx.offset_size).unwrap_or(0) as usize, version }
                    }
                    _ => H5ChunkIndex::Unsupported(index_type),
                };
                Ok(H5Layout::Chunked { rank, chunk_dims: dims, elem_size, index })
            } else {
                Err(LangError::new(format!("hdf5: unsupported chunked layout version {version}"), 0))
            }
        }
        3 if version == 4 => {
            let addr = read_uint(data, 2, ctx.offset_size).unwrap_or(0) as usize;
            let index = u32_at(data, 2 + ctx.offset_size).unwrap_or(0) as usize;
            Ok(H5Layout::Virtual { addr, index })
        }
        _ => Err(LangError::new("hdf5: unsupported data layout", 0)),
    }
}

fn parse_filters(data: &[u8]) -> Result<Vec<H5Filter>, LangError> {
    if data.len() < 2 {
        return Ok(Vec::new());
    }
    let version = data[0];
    let n = data[1] as usize;
    let mut p = 2usize;
    if version == 1 {
        p += 6;
    }
    let mut filters = Vec::with_capacity(n);
    for _ in 0..n {
        let id = u16_at(data, p).unwrap_or(0);
        p += 2;
        let name_len = if version == 1 {
            let v = u16_at(data, p).unwrap_or(0) as usize;
            p += 2;
            v
        } else {
            0
        };
        let _flags = u16_at(data, p).unwrap_or(0);
        let cd_n = u16_at(data, p + 2).unwrap_or(0) as usize;
        p += 4;
        p += name_len;
        if version == 1 {
            p = (p + 7) & !7;
        }
        let mut client = Vec::with_capacity(cd_n);
        for _ in 0..cd_n {
            client.push(u32_at(data, p).unwrap_or(0));
            p += 4;
        }
        if version == 1 {
            p = (p + 7) & !7;
        }
        filters.push(H5Filter { id, client });
    }
    Ok(filters)
}

fn parse_dataset(b: &[u8], ctx: H5Ctx, rctx: &H5ReadCtx, addr: usize, name: String) -> Result<Option<H5Dataset>, LangError> {
    let mut dims: Vec<usize> = Vec::new();
    let mut max_dims: Vec<usize> = Vec::new();
    let mut kind = 0u8;
    let mut size = 0usize;
    let mut layout_raw: Option<Vec<u8>> = None;
    let mut filters: Vec<H5Filter> = Vec::new();
    walk_object_header(b, addr, ctx, &mut |typ, data, _| {
        if typ == 0x0001 {
            let version = b.get(data.start).copied().unwrap_or(1);
            let rank = b.get(data.start + 1).copied().unwrap_or(0) as usize;
            let flags = b.get(data.start + 2).copied().unwrap_or(0);
            let base = if version >= 2 { 4 } else { 8 };
            if rank > 32 {
                return Err(LangError::new("hdf5: dataspace rank is too large", 0));
            }
            for i in 0..rank {
                dims.push(read_uint(b, data.start + base + i * 8, 8).unwrap_or(0) as usize);
            }
            if flags & 0x01 != 0 {
                for i in 0..rank {
                    max_dims.push(read_uint(b, data.start + base + rank * 8 + i * 8, 8).unwrap_or(0) as usize);
                }
            }
        } else if typ == 0x0003 {
            kind = b.get(data.start).copied().unwrap_or(0) & 0x0F;
            size = u32_at(b, data.start + 4).unwrap_or(0) as usize;
        } else if typ == 0x0008 {
            layout_raw = Some(b.get(data).unwrap_or(&[]).to_vec());
        } else if typ == 0x000b {
            filters = parse_filters(b.get(data).unwrap_or(&[]))?;
        }
        Ok(())
    })?;
    if dims.is_empty() || size == 0 {
        return Ok(None);
    }
    let layout = match layout_raw.as_deref() {
        Some(raw) => Some(parse_layout(raw, ctx, dims.len())?),
        None => None,
    };
    let count = dims.iter().fold(1usize, |a, &d| a.saturating_mul(d));
    if count > 50_000_000 {
        return Err(LangError::new("hdf5: dataset is too large to read into memory", 0));
    }
    let data = match layout {
        Some(H5Layout::Compact(ref raw)) => decode_values(raw, count, kind, size),
        Some(H5Layout::Contiguous { addr, size: nbytes }) => {
            let raw = b.get(addr..addr.saturating_add(nbytes)).unwrap_or(&[]);
            decode_values(raw, count, kind, size)
        }
        Some(H5Layout::Chunked { rank, chunk_dims, elem_size, index }) => {
            let elem = if elem_size == 0 { size } else { elem_size };
            read_chunks(b, ctx, &dims, &max_dims, rank, &chunk_dims, elem, kind, size, &index, &filters)?
        }
        Some(H5Layout::Virtual { addr, index }) => {
            if !rctx.follow_virtual {
                return Ok(None);
            }
            read_virtual(b, ctx, rctx, addr, index, &dims)?
        }
        None => return Ok(None),
    };
    Ok(Some(H5Dataset { name, dims, data }))
}

fn decode_values(raw: &[u8], count: usize, kind: u8, size: usize) -> Vec<f64> {
    let mut data = Vec::with_capacity(count);
    for i in 0..count {
        data.push(value_at(raw, i.saturating_mul(size), kind, size));
    }
    data
}

fn value_at(raw: &[u8], off: usize, kind: u8, size: usize) -> f64 {
    match (kind, size) {
        (1, 8) => u64_at(raw, off).map(f64::from_bits).unwrap_or(f64::NAN),
        (1, 4) => u32_at(raw, off).map(|u| f32::from_bits(u) as f64).unwrap_or(f64::NAN),
        (0, 8) => u64_at(raw, off).map(|u| u as i64 as f64).unwrap_or(f64::NAN),
        (0, 4) => u32_at(raw, off).map(|u| u as i32 as f64).unwrap_or(f64::NAN),
        (0, 2) => u16_at(raw, off).map(|u| u as i16 as f64).unwrap_or(f64::NAN),
        (0, 1) => raw.get(off).map(|u| *u as i8 as f64).unwrap_or(f64::NAN),
        _ => f64::NAN,
    }
}

fn chunk_offsets(index: usize, rank: usize, dims: &[usize], chunk_dims: &[usize]) -> Vec<usize> {
    let mut rem = index;
    let mut offs = vec![0usize; rank];
    for d in (0..rank).rev() {
        let c = chunk_dims.get(d).copied().unwrap_or(0).max(1);
        let n = dims.get(d).copied().unwrap_or(0).div_ceil(c);
        if n > 0 {
            if let Some(slot) = offs.get_mut(d) {
                *slot = (rem % n).saturating_mul(c);
            }
            rem /= n;
        }
    }
    offs
}

#[allow(clippy::too_many_arguments)]
fn read_chunks(
    b: &[u8],
    ctx: H5Ctx,
    dims: &[usize],
    max_dims: &[usize],
    rank: usize,
    chunk_dims: &[usize],
    elem_size: usize,
    kind: u8,
    size: usize,
    index: &H5ChunkIndex,
    filters: &[H5Filter],
) -> Result<Vec<f64>, LangError> {
    if chunk_dims.len() != rank || rank != dims.len() {
        return Err(LangError::new("hdf5: chunk rank does not match the dataspace", 0));
    }
    let chunk_elems = chunk_dims.iter().fold(1usize, |a, &d| a.saturating_mul(d));
    let raw_size = chunk_elems.saturating_mul(elem_size);
    let grid = chunk_dims.iter().zip(dims).fold(1usize, |a, (&c, &d)| a.saturating_mul(d.div_ceil(c.max(1))));
    if grid > 10_000_000 {
        return Err(LangError::new("hdf5: chunk grid is too large to index", 0));
    }
    let mut records: Vec<H5ChunkRecord> = Vec::new();
    match index {
        H5ChunkIndex::FixedArray { addr } => collect_fixed_array(b, ctx, *addr, grid, rank, dims, chunk_dims, elem_size, raw_size, &mut records)?,
        H5ChunkIndex::BTreeV1 { addr } => collect_btree_v1(b, ctx, *addr, rank, &mut records)?,
        H5ChunkIndex::Single { addr, filtered_size, mask } => {
            records.push(H5ChunkRecord { offset: vec![0; rank], addr: *addr, nbytes: filtered_size.unwrap_or(raw_size), mask: *mask });
        }
        H5ChunkIndex::Implicit { addr } => {
            for i in 0..grid {
                records.push(H5ChunkRecord {
                    offset: chunk_offsets(i, rank, dims, chunk_dims),
                    addr: addr.saturating_add(i.saturating_mul(raw_size)),
                    nbytes: raw_size,
                    mask: 0,
                });
            }
        }
        H5ChunkIndex::ExtensibleArray { addr } => {
            collect_extensible_array(b, ctx, *addr, grid, rank, dims, max_dims, chunk_dims, raw_size, &mut records)?
        }
        H5ChunkIndex::BTreeV2 { addr, version } => collect_btree_v2(b, ctx, *addr, rank, *version, chunk_dims, raw_size, elem_size, &mut records)?,
        H5ChunkIndex::Unsupported(t) => return Err(LangError::new(format!("hdf5: chunk index type {t} is not supported"), 0)),
    }
    let count = dims.iter().fold(1usize, |a, &d| a.saturating_mul(d));
    let mut out = vec![0.0f64; count];
    for rec in records {
        if rec.addr == 0 || rec.addr == usize::MAX || rec.nbytes == 0 {
            continue;
        }
        let raw = b.get(rec.addr..rec.addr.saturating_add(rec.nbytes)).ok_or_else(|| LangError::new("hdf5: chunk data is truncated", 0))?;
        let decoded = decode_chunk(raw, rec.mask, filters, elem_size)?;
        if decoded.len() < raw_size {
            return Err(LangError::new("hdf5: decoded chunk is shorter than expected", 0));
        }
        for li in 0..chunk_elems {
            let mut rem = li;
            let mut local = vec![0usize; rank];
            for d in (0..rank).rev() {
                let cd = chunk_dims.get(d).copied().unwrap_or(1).max(1);
                local[d] = rem % cd;
                rem /= cd;
            }
            let mut flat = 0usize;
            let mut in_bounds = true;
            for d in 0..rank {
                let global = rec.offset.get(d).copied().unwrap_or(0).saturating_add(local.get(d).copied().unwrap_or(0));
                let dim = dims.get(d).copied().unwrap_or(0);
                if global >= dim {
                    in_bounds = false;
                    break;
                }
                flat = flat.saturating_mul(dim).saturating_add(global);
            }
            if in_bounds {
                if let Some(slot) = out.get_mut(flat) {
                    *slot = value_at(&decoded, li.saturating_mul(elem_size), kind, size);
                }
            }
        }
    }
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
fn collect_fixed_array(
    b: &[u8],
    ctx: H5Ctx,
    addr: usize,
    grid: usize,
    rank: usize,
    dims: &[usize],
    chunk_dims: &[usize],
    elem_size: usize,
    raw_size: usize,
    out: &mut Vec<H5ChunkRecord>,
) -> Result<(), LangError> {
    if b.get(addr..addr + 4) != Some(b"FAHD") {
        return Err(LangError::new("hdf5: fixed-array chunk index is missing", 0));
    }
    let client = b.get(addr + 5).copied().unwrap_or(0);
    let entry_size = b.get(addr + 6).copied().unwrap_or(ctx.offset_size as u8) as usize;
    let page_bits = b.get(addr + 7).copied().unwrap_or(10) as usize;
    let num = len_at(b, addr + 8, ctx);
    let data_addr = off_at(b, addr + 8 + ctx.length_size, ctx);
    if b.get(data_addr..data_addr + 4) != Some(b"FADB") {
        return Err(LangError::new("hdf5: fixed-array data block is missing", 0));
    }
    let page_nelmts = 1usize << page_bits.min(30);
    let paged = page_bits < 32 && num > page_nelmts;
    let header = data_addr + 6 + ctx.offset_size;
    let bitmap_len = if paged { num.div_ceil(page_nelmts).div_ceil(8) } else { 0 };
    let pages_base = header.saturating_add(bitmap_len).saturating_add(if paged { 4 } else { 0 });
    let page_size = page_nelmts.saturating_mul(entry_size).saturating_add(4);
    for i in 0..num.min(grid) {
        let elem_off = if paged {
            let page = i / page_nelmts;
            if b.get(header + page / 8).map(|v| v & (0x80u8 >> (page % 8)) == 0).unwrap_or(true) {
                continue;
            }
            pages_base.saturating_add(page.saturating_mul(page_size)).saturating_add((i % page_nelmts).saturating_mul(entry_size))
        } else {
            pages_base.saturating_add(i.saturating_mul(entry_size))
        };
        let mut p = elem_off;
        let chunk_addr = off_at(b, p, ctx);
        p += ctx.offset_size;
        let (nbytes, mask) = if client == 1 {
            let n = read_uint(b, p, entry_size.saturating_sub(ctx.offset_size + 4)).unwrap_or(0) as usize;
            p += entry_size.saturating_sub(ctx.offset_size + 4);
            let m = u32_at(b, p).unwrap_or(0);
            (n, m)
        } else {
            (raw_size, 0)
        };
        if chunk_addr != 0 && chunk_addr != usize::MAX {
            out.push(H5ChunkRecord { offset: chunk_offsets(i, rank, dims, chunk_dims), addr: chunk_addr, nbytes, mask });
        }
    }
    let _ = elem_size;
    Ok(())
}

#[derive(Clone)]
struct EaHeader {
    client: u8,
    elem_size: usize,
    max_bits: usize,
    idx_el: usize,
    min_el: usize,
    sup_min: usize,
    page_bits: usize,
    idx_addr: usize,
}

#[derive(Clone, Copy)]
struct EaSblk {
    ndblks: usize,
    dblk_nelmts: usize,
    start_idx: usize,
    start_dblk: usize,
}

struct EaGeometry {
    idx_el: usize,
    min_el: usize,
    page_nelmts: usize,
    iblock_nsblks: usize,
    ndblk_addrs: usize,
    nsblk_addrs: usize,
    sblk: Vec<EaSblk>,
}

impl EaGeometry {
    fn new(h: &EaHeader) -> Result<Self, LangError> {
        if h.min_el == 0 || !h.min_el.is_power_of_two() {
            return Err(LangError::new("hdf5: invalid extensible-array data-block size", 0));
        }
        if h.sup_min == 0 || !h.sup_min.is_power_of_two() {
            return Err(LangError::new("hdf5: invalid extensible-array super-block pointers", 0));
        }
        let min_bits = h.min_el.trailing_zeros() as usize;
        if h.max_bits < min_bits || h.max_bits > 64 || h.page_bits >= 64 {
            return Err(LangError::new("hdf5: invalid extensible-array header parameters", 0));
        }
        let nsblks = h.max_bits - min_bits + 1;
        let iblock_nsblks = 2 * h.sup_min.trailing_zeros() as usize;
        if iblock_nsblks > nsblks {
            return Err(LangError::new("hdf5: extensible-array super-block geometry is invalid", 0));
        }
        let mut sblk = Vec::with_capacity(nsblks);
        let mut start_idx = 0usize;
        let mut start_dblk = 0usize;
        for u in 0..nsblks {
            let ndblks = 1usize.checked_shl((u / 2) as u32).ok_or_else(|| LangError::new("hdf5: extensible-array is too large", 0))?;
            let factor = 1usize.checked_shl(u.div_ceil(2) as u32).ok_or_else(|| LangError::new("hdf5: extensible-array is too large", 0))?;
            let dblk_nelmts = factor.checked_mul(h.min_el).ok_or_else(|| LangError::new("hdf5: extensible-array is too large", 0))?;
            sblk.push(EaSblk { ndblks, dblk_nelmts, start_idx, start_dblk });
            start_idx = start_idx
                .checked_add(ndblks.checked_mul(dblk_nelmts).ok_or_else(|| LangError::new("hdf5: extensible-array is too large", 0))?)
                .ok_or_else(|| LangError::new("hdf5: extensible-array is too large", 0))?;
            start_dblk = start_dblk.checked_add(ndblks).ok_or_else(|| LangError::new("hdf5: extensible-array is too large", 0))?;
        }
        Ok(EaGeometry {
            idx_el: h.idx_el,
            min_el: h.min_el,
            page_nelmts: 1usize << h.page_bits.min(62),
            iblock_nsblks,
            ndblk_addrs: 2 * (h.sup_min - 1),
            nsblk_addrs: nsblks - iblock_nsblks,
            sblk,
        })
    }

    fn locate(&self, index: usize) -> Option<(usize, usize, usize, usize)> {
        if index < self.idx_el {
            return None;
        }
        let e = index - self.idx_el;
        let sblk_idx = (e / self.min_el).checked_add(1)?.ilog2() as usize;
        let s = self.sblk.get(sblk_idx)?;
        let elmt = e.checked_sub(s.start_idx)?;
        let local_dblk = elmt / s.dblk_nelmts.max(1);
        let offset = elmt % s.dblk_nelmts.max(1);
        (local_dblk < s.ndblks).then_some((sblk_idx, local_dblk, offset, s.dblk_nelmts))
    }
}

#[derive(Clone, Copy)]
struct EaElement {
    addr: usize,
    nbytes: usize,
    mask: u32,
}

fn read_ea_header(b: &[u8], ctx: H5Ctx, addr: usize) -> Result<EaHeader, LangError> {
    let s = b.get(addr..addr + 12).ok_or_else(|| LangError::new("hdf5: extensible-array header is truncated", 0))?;
    if s.get(0..4) != Some(b"EAHD") || s.get(4) != Some(&0) {
        return Err(LangError::new("hdf5: invalid extensible-array header", 0));
    }
    let h = EaHeader {
        client: s.get(5).copied().unwrap_or(0),
        elem_size: s.get(6).copied().unwrap_or(0) as usize,
        max_bits: s.get(7).copied().unwrap_or(0) as usize,
        idx_el: s.get(8).copied().unwrap_or(0) as usize,
        min_el: s.get(9).copied().unwrap_or(0) as usize,
        sup_min: s.get(10).copied().unwrap_or(0) as usize,
        page_bits: s.get(11).copied().unwrap_or(0) as usize,
        idx_addr: off_at(b, addr + 12 + 6 * ctx.length_size, ctx),
    };
    let min_elem = if h.client == 1 { ctx.offset_size + 4 } else { ctx.offset_size };
    if h.client > 1 || h.elem_size < min_elem || h.idx_el == 0 || h.idx_addr == 0 || h.idx_addr == usize::MAX {
        return Err(LangError::new("hdf5: invalid extensible-array header parameters", 0));
    }
    Ok(h)
}

fn read_ea_element(b: &[u8], off: usize, h: &EaHeader, ctx: H5Ctx, raw_size: usize) -> Option<EaElement> {
    let bytes = b.get(off..off.checked_add(h.elem_size)?)?;
    if bytes.iter().all(|&v| v == 0xFF) {
        return None;
    }
    let addr = read_uint(bytes, 0, ctx.offset_size)? as usize;
    if addr == 0 || addr == usize::MAX {
        return None;
    }
    if h.client == 0 {
        return Some(EaElement { addr, nbytes: raw_size, mask: 0 });
    }
    let size_len = h.elem_size.checked_sub(ctx.offset_size + 4)?;
    if size_len == 0 {
        return None;
    }
    let nbytes = read_uint(bytes, ctx.offset_size, size_len)? as usize;
    let mask = u32_at(bytes, ctx.offset_size + size_len)?;
    (nbytes > 0).then_some(EaElement { addr, nbytes, mask })
}

fn ea_chunk_offsets(index: usize, rank: usize, dims: &[usize], max_dims: &[usize], chunk_dims: &[usize]) -> Vec<usize> {
    let unlimited = max_dims.iter().position(|&m| m == usize::MAX || m == u64::MAX as usize);
    let Some(u) = unlimited.filter(|&u| u < rank) else {
        return chunk_offsets(index, rank, dims, chunk_dims);
    };
    let mut rem = index;
    let mut offs = vec![0usize; rank];
    for d in (0..rank).rev() {
        if d == u {
            continue;
        }
        let c = chunk_dims.get(d).copied().unwrap_or(1).max(1);
        let n = dims.get(d).copied().unwrap_or(0).div_ceil(c).max(1);
        if let Some(slot) = offs.get_mut(d) {
            *slot = (rem % n).saturating_mul(c);
        }
        rem /= n;
    }
    if let Some(slot) = offs.get_mut(u) {
        *slot = rem.saturating_mul(chunk_dims.get(u).copied().unwrap_or(1));
    }
    offs
}

#[allow(clippy::too_many_arguments)]
fn collect_extensible_array(
    b: &[u8],
    ctx: H5Ctx,
    header_addr: usize,
    grid: usize,
    rank: usize,
    dims: &[usize],
    max_dims: &[usize],
    chunk_dims: &[usize],
    raw_size: usize,
    out: &mut Vec<H5ChunkRecord>,
) -> Result<(), LangError> {
    let h = read_ea_header(b, ctx, header_addr)?;
    let geo = EaGeometry::new(&h)?;
    if h.idx_el.saturating_add(geo.sblk.last().map(|s| s.start_idx.saturating_add(s.ndblks.saturating_mul(s.dblk_nelmts))).unwrap_or(0)) < grid {
        return Err(LangError::new("hdf5: extensible-array does not cover the dataset", 0));
    }
    let idx = h.idx_addr;
    if b.get(idx..idx + 4) != Some(b"EAIB") {
        return Err(LangError::new("hdf5: extensible-array index block is missing", 0));
    }
    let elems_start = idx + 6 + ctx.offset_size;
    for i in 0..h.idx_el.min(grid) {
        let off = elems_start.saturating_add(i.saturating_mul(h.elem_size));
        if let Some(el) = read_ea_element(b, off, &h, ctx, raw_size) {
            out.push(H5ChunkRecord {
                offset: ea_chunk_offsets(i, rank, dims, max_dims, chunk_dims),
                addr: el.addr,
                nbytes: el.nbytes,
                mask: el.mask,
            });
        }
    }
    let dblk_base = elems_start.saturating_add(h.idx_el.saturating_mul(h.elem_size));
    let dblk_addrs: Vec<usize> = (0..geo.ndblk_addrs).map(|i| off_at(b, dblk_base + i * ctx.offset_size, ctx)).collect();
    let sblk_base = dblk_base.saturating_add(geo.ndblk_addrs.saturating_mul(ctx.offset_size));
    let sblk_addrs: Vec<usize> = (0..geo.nsblk_addrs).map(|i| off_at(b, sblk_base + i * ctx.offset_size, ctx)).collect();
    let bo_size = h.max_bits.div_ceil(8).max(1);
    for index in h.idx_el..grid {
        let Some((sblk_idx, local_dblk, offset_in_dblk, dblk_nelmts)) = geo.locate(index) else {
            continue;
        };
        let addr = if sblk_idx < geo.iblock_nsblks {
            let Some(&s) = geo.sblk.get(sblk_idx) else { continue };
            dblk_addrs.get(s.start_dblk + local_dblk).copied().unwrap_or(0)
        } else {
            let s_off = sblk_idx - geo.iblock_nsblks;
            let Some(&sblk_addr) = sblk_addrs.get(s_off) else { continue };
            if sblk_addr == 0 || sblk_addr == usize::MAX {
                continue;
            }
            let s = geo.sblk.get(sblk_idx).copied().unwrap_or(EaSblk { ndblks: 0, dblk_nelmts: 0, start_idx: 0, start_dblk: 0 });
            let paged = dblk_nelmts > geo.page_nelmts;
            let pages = if paged { dblk_nelmts / geo.page_nelmts } else { 0 };
            let bitmap = s.ndblks.saturating_mul(pages.div_ceil(8));
            let addr_off = sblk_addr + 6 + ctx.offset_size + bo_size + if paged { bitmap } else { 0 };
            off_at(b, addr_off + local_dblk * ctx.offset_size, ctx)
        };
        if addr == 0 || addr == usize::MAX {
            continue;
        }
        if b.get(addr..addr + 4) != Some(b"EADB") {
            continue;
        }
        let paged = dblk_nelmts > geo.page_nelmts;
        let elem_off = if paged {
            let pages = dblk_nelmts / geo.page_nelmts;
            let bitmap_len = pages.div_ceil(8);
            let page = offset_in_dblk / geo.page_nelmts;
            let in_page = offset_in_dblk % geo.page_nelmts;
            let bitmap_off = addr + 6 + ctx.offset_size + bo_size;
            if page >= pages || b.get(bitmap_off + page / 8).map(|v| v & (1u8 << (page % 8)) == 0).unwrap_or(true) {
                continue;
            }
            let page_size = geo.page_nelmts.saturating_mul(h.elem_size).saturating_add(4);
            addr + 6 + ctx.offset_size + bo_size + bitmap_len + page.saturating_mul(page_size) + in_page.saturating_mul(h.elem_size)
        } else {
            addr + 6 + ctx.offset_size + bo_size + offset_in_dblk.saturating_mul(h.elem_size)
        };
        if let Some(el) = read_ea_element(b, elem_off, &h, ctx, raw_size) {
            out.push(H5ChunkRecord {
                offset: ea_chunk_offsets(index, rank, dims, max_dims, chunk_dims),
                addr: el.addr,
                nbytes: el.nbytes,
                mask: el.mask,
            });
        }
    }
    Ok(())
}

fn limit_enc_size(limit: usize) -> usize {
    if limit == 0 {
        1
    } else {
        ((usize::BITS - limit.leading_zeros()) as usize).div_ceil(8)
    }
}

#[allow(clippy::too_many_arguments)]
fn collect_btree_v2(
    b: &[u8],
    ctx: H5Ctx,
    addr: usize,
    rank: usize,
    _layout_version: u8,
    chunk_dims: &[usize],
    raw_size: usize,
    elem_size: usize,
    out: &mut Vec<H5ChunkRecord>,
) -> Result<(), LangError> {
    if b.get(addr..addr + 4) != Some(b"BTHD") {
        return Err(LangError::new("hdf5: B-tree v2 header is missing", 0));
    }
    let record_type = b.get(addr + 5).copied().unwrap_or(0);
    let node_size = u32_at(b, addr + 6).unwrap_or(0) as usize;
    let record_size = u16_at(b, addr + 10).unwrap_or(0) as usize;
    let depth = u16_at(b, addr + 12).unwrap_or(0) as usize;
    let root = off_at(b, addr + 16, ctx);
    let root_n = u16_at(b, addr + 16 + ctx.offset_size).unwrap_or(0) as usize;
    if node_size < 10 || record_size == 0 || depth > 64 {
        return Err(LangError::new("hdf5: invalid B-tree v2 geometry", 0));
    }
    let leaf_max = node_size.saturating_sub(10) / record_size;
    let max_nrec_size = limit_enc_size(leaf_max);
    let mut cum_sizes = vec![0usize; depth + 1];
    let mut cum = leaf_max;
    for d in 1..=depth {
        let child_total = if d > 1 { cum_sizes[d - 1] } else { 0 };
        let ptr = ctx.offset_size + max_nrec_size + child_total;
        let max_nrec = node_size.saturating_sub(10 + ptr) / (record_size + ptr).max(1);
        cum = (max_nrec + 1).saturating_mul(cum).saturating_add(max_nrec);
        cum_sizes[d] = limit_enc_size(cum);
    }
    let chunk_size_len = if record_type == 11 { record_size.saturating_sub(ctx.offset_size + 4 + rank * 8).max(1) } else { 0 };
    let mut stack = vec![(root, depth, root_n)];
    let mut visited = 0usize;
    while let Some((node, node_depth, nrec)) = stack.pop() {
        visited += 1;
        if visited > 200_000 {
            return Err(LangError::new("hdf5: B-tree v2 is too large", 0));
        }
        if node_depth == 0 {
            if b.get(node..node + 4) != Some(b"BTLF") {
                return Err(LangError::new("hdf5: B-tree v2 leaf node is missing", 0));
            }
            for i in 0..nrec {
                let p = node + 6 + i * record_size;
                parse_bt2_record(b, ctx, p, record_type, rank, chunk_dims, chunk_size_len, raw_size, elem_size, out);
            }
        } else {
            if b.get(node..node + 4) != Some(b"BTIN") {
                return Err(LangError::new("hdf5: B-tree v2 internal node is missing", 0));
            }
            let rec_start = node + 6;
            for i in 0..nrec {
                parse_bt2_record(b, ctx, rec_start + i * record_size, record_type, rank, chunk_dims, chunk_size_len, raw_size, elem_size, out);
            }
            let mut p = rec_start + nrec * record_size;
            let child_total = if node_depth > 1 { cum_sizes[node_depth - 1] } else { 0 };
            for _ in 0..=nrec {
                let child = off_at(b, p, ctx);
                p += ctx.offset_size;
                let child_n = read_uint(b, p, max_nrec_size).unwrap_or(0) as usize;
                p += max_nrec_size;
                if child_total > 0 {
                    p += child_total;
                }
                if child != 0 && child != usize::MAX {
                    stack.push((child, node_depth - 1, child_n));
                }
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn parse_bt2_record(
    b: &[u8],
    ctx: H5Ctx,
    p: usize,
    record_type: u8,
    rank: usize,
    chunk_dims: &[usize],
    chunk_size_len: usize,
    raw_size: usize,
    _elem_size: usize,
    out: &mut Vec<H5ChunkRecord>,
) {
    if record_type != 10 && record_type != 11 {
        return;
    }
    let addr = off_at(b, p, ctx);
    let (nbytes, mask, offsets_at) = if record_type == 11 {
        let n = read_uint(b, p + ctx.offset_size, chunk_size_len).unwrap_or(0) as usize;
        let m = u32_at(b, p + ctx.offset_size + chunk_size_len).unwrap_or(0);
        (n, m, p + ctx.offset_size + chunk_size_len + 4)
    } else {
        (raw_size, 0, p + ctx.offset_size)
    };
    let mut offset = Vec::with_capacity(rank);
    for d in 0..rank {
        let scaled = read_uint(b, offsets_at + d * 8, 8).unwrap_or(0) as usize;
        offset.push(scaled.saturating_mul(chunk_dims.get(d).copied().unwrap_or(1)));
    }
    if addr != 0 && addr != usize::MAX && nbytes > 0 {
        out.push(H5ChunkRecord { offset, addr, nbytes, mask });
    }
}

fn collect_btree_v1(b: &[u8], ctx: H5Ctx, addr: usize, rank: usize, out: &mut Vec<H5ChunkRecord>) -> Result<(), LangError> {
    let mut stack = vec![addr];
    let mut visited = 0usize;
    while let Some(node) = stack.pop() {
        visited += 1;
        if visited > 100_000 {
            return Err(LangError::new("hdf5: chunk B-tree is too large", 0));
        }
        if b.get(node..node + 4) != Some(b"TREE") {
            return Err(LangError::new("hdf5: chunk B-tree node is missing", 0));
        }
        if b.get(node + 4).copied().unwrap_or(0) != 1 {
            return Err(LangError::new("hdf5: expected a chunk B-tree node", 0));
        }
        let level = b.get(node + 5).copied().unwrap_or(0);
        let n = u16_at(b, node + 6).unwrap_or(0) as usize;
        let mut p = node + 24;
        // Layout-v3 B-tree keys carry rank+1 offsets: the extra one is the
        // element-size dimension added by the chunked layout encoding.
        let key_rank = rank.saturating_add(1);
        for _ in 0..n {
            let nbytes = u32_at(b, p).unwrap_or(0) as usize;
            let mask = u32_at(b, p + 4).unwrap_or(0);
            let mut offset = Vec::with_capacity(rank);
            for d in 0..key_rank {
                let value = read_uint(b, p + 8 + d * 8, 8).unwrap_or(0) as usize;
                if d < rank {
                    offset.push(value);
                }
            }
            let child = off_at(b, p + 8 + key_rank * 8, ctx);
            p += 8 + key_rank * 8 + ctx.offset_size;
            if level == 0 {
                out.push(H5ChunkRecord { offset, addr: child, nbytes, mask });
            } else {
                stack.push(child);
            }
        }
    }
    Ok(())
}

fn decode_chunk(raw: &[u8], mask: u32, filters: &[H5Filter], elem_size: usize) -> Result<Vec<u8>, LangError> {
    let mut data = raw.to_vec();
    for (i, filter) in filters.iter().enumerate().rev() {
        if mask & (1u32 << i.min(31)) != 0 {
            continue;
        }
        match filter.id {
            1 => {
                use std::io::Read;
                let mut out = Vec::new();
                flate2::read::ZlibDecoder::new(data.as_slice())
                    .read_to_end(&mut out)
                    .map_err(|e| LangError::new(format!("hdf5: deflate error: {e}"), 0))?;
                data = out;
            }
            2 => {
                let es = filter.client.first().copied().unwrap_or(elem_size as u32).max(1) as usize;
                data = unshuffle(&data, es);
            }
            3 => {}
            0 => {}
            other => return Err(LangError::new(format!("hdf5: filter {other} is not supported"), 0)),
        }
    }
    Ok(data)
}

fn unshuffle(data: &[u8], elem_size: usize) -> Vec<u8> {
    if elem_size == 0 {
        return data.to_vec();
    }
    let n = data.len() / elem_size;
    let mut out = vec![0u8; n.saturating_mul(elem_size)];
    for i in 0..n {
        for b in 0..elem_size {
            if let (Some(v), Some(slot)) = (data.get(b * n + i), out.get_mut(i * elem_size + b)) {
                *slot = *v;
            }
        }
    }
    out
}

#[derive(Clone)]
enum VdsSelection {
    None,
    All,
    Points(Vec<Vec<u64>>),
    Regular(Vec<(u64, u64, u64, u64)>),
    Blocks(Vec<(Vec<u64>, Vec<u64>)>),
}

struct VdsMapping {
    source_file: String,
    source_dataset: String,
    source_selection: VdsSelection,
    virtual_selection: VdsSelection,
}

const MAX_VDS_ELEMENTS: usize = 50_000_000;

fn vds_take_u32(data: &[u8], pos: &mut usize) -> Option<u32> {
    let v = u32_at(data, *pos)?;
    *pos += 4;
    Some(v)
}

fn vds_take_uint(data: &[u8], pos: &mut usize, size: usize) -> Option<u64> {
    let v = read_uint(data, *pos, size)?;
    *pos += size;
    Some(v)
}

fn vds_take_cstr(data: &[u8], pos: &mut usize) -> Option<String> {
    let start = *pos;
    let end = data.get(start..)?.iter().position(|&b| b == 0).map(|n| start + n)?;
    *pos = end + 1;
    Some(String::from_utf8_lossy(data.get(start..end)?).to_string())
}

/// Parse one serialized dataspace selection. The wire form is a 32-bit
/// selection type, a 32-bit version, then a type-specific body (HDF5's
/// `H5S_select_deserialize`).
fn parse_vds_selection(data: &[u8], pos: &mut usize) -> Result<VdsSelection, LangError> {
    let sel_type = vds_take_u32(data, pos).ok_or_else(|| LangError::new("hdf5: virtual selection is truncated", 0))?;
    let version = vds_take_u32(data, pos).ok_or_else(|| LangError::new("hdf5: virtual selection is truncated", 0))?;
    match sel_type {
        0 | 3 => {
            if version > 1 {
                return Err(LangError::new("hdf5: unsupported virtual selection version", 0));
            }
            *pos = pos.saturating_add(8);
            Ok(if sel_type == 3 { VdsSelection::All } else { VdsSelection::None })
        }
        1 => {
            let enc = if version == 2 {
                let e = *data.get(*pos).ok_or_else(|| LangError::new("hdf5: virtual point selection is truncated", 0))? as usize;
                *pos += 1;
                e
            } else {
                *pos = pos.saturating_add(4 + 4);
                4
            };
            if !matches!(enc, 1 | 2 | 4 | 8) {
                return Err(LangError::new("hdf5: invalid virtual point-selection width", 0));
            }
            let rank = vds_take_u32(data, pos).ok_or_else(|| LangError::new("hdf5: virtual point selection is truncated", 0))? as usize;
            if rank == 0 || rank > 32 {
                return Err(LangError::new("hdf5: invalid virtual point-selection rank", 0));
            }
            let n = vds_take_uint(data, pos, enc).unwrap_or(0) as usize;
            if n > MAX_VDS_ELEMENTS || n.saturating_mul(rank).saturating_mul(enc) > data.len().saturating_sub(*pos) {
                return Err(LangError::new("hdf5: virtual point selection is too large", 0));
            }
            let mut points = Vec::with_capacity(n);
            for _ in 0..n {
                let mut p = Vec::with_capacity(rank);
                for _ in 0..rank {
                    p.push(vds_take_uint(data, pos, enc).ok_or_else(|| LangError::new("hdf5: virtual point selection is truncated", 0))?);
                }
                points.push(p);
            }
            Ok(VdsSelection::Points(points))
        }
        2 => {
            let (flags, enc) = match version {
                1 => {
                    *pos = pos.saturating_add(8);
                    (0u8, 4usize)
                }
                2 => {
                    let f = *data.get(*pos).ok_or_else(|| LangError::new("hdf5: virtual hyperslab is truncated", 0))?;
                    *pos += 1;
                    *pos = pos.saturating_add(4);
                    (f, 8)
                }
                3 => {
                    let f = *data.get(*pos).ok_or_else(|| LangError::new("hdf5: virtual hyperslab is truncated", 0))?;
                    *pos += 1;
                    let e = *data.get(*pos).ok_or_else(|| LangError::new("hdf5: virtual hyperslab is truncated", 0))? as usize;
                    *pos += 1;
                    (f, e)
                }
                _ => return Err(LangError::new("hdf5: unsupported virtual hyperslab version", 0)),
            };
            if !matches!(enc, 1 | 2 | 4 | 8) {
                return Err(LangError::new("hdf5: invalid virtual hyperslab width", 0));
            }
            let rank = vds_take_u32(data, pos).ok_or_else(|| LangError::new("hdf5: virtual hyperslab is truncated", 0))? as usize;
            if rank == 0 || rank > 32 {
                return Err(LangError::new("hdf5: invalid virtual hyperslab rank", 0));
            }
            if version >= 2 && flags & 0x01 != 0 {
                let need = rank.saturating_mul(4).saturating_mul(enc);
                if need > data.len().saturating_sub(*pos) {
                    return Err(LangError::new("hdf5: virtual hyperslab is truncated", 0));
                }
                let mut dims = Vec::with_capacity(rank);
                for _ in 0..rank {
                    let start = vds_take_uint(data, pos, enc).unwrap_or(0);
                    let stride = vds_take_uint(data, pos, enc).unwrap_or(1);
                    let count = vds_take_uint(data, pos, enc).unwrap_or(0);
                    let block = vds_take_uint(data, pos, enc).unwrap_or(0);
                    dims.push((start, stride, count, block));
                }
                Ok(VdsSelection::Regular(dims))
            } else {
                let n = vds_take_uint(data, pos, enc).unwrap_or(0) as usize;
                if n > MAX_VDS_ELEMENTS || n.saturating_mul(rank).saturating_mul(2).saturating_mul(enc) > data.len().saturating_sub(*pos) {
                    return Err(LangError::new("hdf5: virtual hyperslab block list is too large", 0));
                }
                let mut blocks = Vec::with_capacity(n);
                for _ in 0..n {
                    let mut start = Vec::with_capacity(rank);
                    let mut end = Vec::with_capacity(rank);
                    for _ in 0..rank {
                        start.push(vds_take_uint(data, pos, enc).unwrap_or(0));
                    }
                    for _ in 0..rank {
                        end.push(vds_take_uint(data, pos, enc).unwrap_or(0));
                    }
                    blocks.push((start, end));
                }
                Ok(VdsSelection::Blocks(blocks))
            }
        }
        _ => Err(LangError::new("hdf5: unknown virtual selection type", 0)),
    }
}

fn vds_linear(coord: &[u64], shape: &[usize]) -> Option<usize> {
    if coord.len() != shape.len() {
        return None;
    }
    let mut stride = 1usize;
    let mut linear = 0usize;
    for d in (0..shape.len()).rev() {
        let c = *coord.get(d)? as usize;
        if c >= shape[d] {
            return None;
        }
        linear = linear.checked_add(c.checked_mul(stride)?)?;
        stride = stride.checked_mul(shape[d])?;
    }
    Some(linear)
}

fn vds_cartesian_offsets(per_dim: &[Vec<u64>], shape: &[usize], out: &mut Vec<usize>) -> Result<(), LangError> {
    if per_dim.len() != shape.len() {
        return Err(LangError::new("hdf5: virtual hyperslab rank does not match the dataspace", 0));
    }
    let total = per_dim.iter().fold(1usize, |a, v| a.saturating_mul(v.len()));
    if total > MAX_VDS_ELEMENTS {
        return Err(LangError::new("hdf5: virtual hyperslab is too large", 0));
    }
    if per_dim.iter().any(|v| v.is_empty()) || per_dim.is_empty() {
        return Ok(());
    }
    let mut idx = vec![0usize; per_dim.len()];
    loop {
        let coord: Vec<u64> = per_dim.iter().enumerate().map(|(d, v)| v[idx[d]]).collect();
        if let Some(linear) = vds_linear(&coord, shape) {
            out.push(linear);
        }
        let mut d = per_dim.len();
        loop {
            if d == 0 {
                return Ok(());
            }
            d -= 1;
            idx[d] += 1;
            if idx[d] < per_dim[d].len() {
                break;
            }
            idx[d] = 0;
        }
    }
}

fn vds_selection_offsets(sel: &VdsSelection, shape: &[usize]) -> Result<Vec<usize>, LangError> {
    let total = shape.iter().fold(1usize, |a, &d| a.saturating_mul(d));
    if total > MAX_VDS_ELEMENTS {
        return Err(LangError::new("hdf5: virtual dataspace is too large", 0));
    }
    match sel {
        VdsSelection::None => Ok(Vec::new()),
        VdsSelection::All => Ok((0..total).collect()),
        VdsSelection::Points(points) => {
            let mut out = Vec::with_capacity(points.len());
            for p in points {
                out.push(vds_linear(p, shape).ok_or_else(|| LangError::new("hdf5: virtual point is out of bounds", 0))?);
            }
            Ok(out)
        }
        VdsSelection::Regular(dims) => {
            if dims.len() != shape.len() {
                return Err(LangError::new("hdf5: virtual hyperslab rank does not match the dataspace", 0));
            }
            let mut per_dim: Vec<Vec<u64>> = Vec::with_capacity(dims.len());
            for (d, &(start, stride, count, block)) in dims.iter().enumerate() {
                let dim = shape[d];
                let stride = stride.max(1);
                let count = if count == u64::MAX { (dim as u64).saturating_sub(start).div_ceil(stride).max(1) } else { count };
                let block = if block == u64::MAX { dim as u64 } else { block };
                let mut coords = Vec::new();
                for k in 0..count {
                    let base = start.saturating_add(k.saturating_mul(stride));
                    if base >= dim as u64 {
                        break;
                    }
                    let len = block.min((dim as u64).saturating_sub(base));
                    for j in 0..len {
                        coords.push(base + j);
                        if coords.len() > MAX_VDS_ELEMENTS {
                            return Err(LangError::new("hdf5: virtual hyperslab is too large", 0));
                        }
                    }
                }
                per_dim.push(coords);
            }
            let count = per_dim.iter().fold(1usize, |a, v| a.saturating_mul(v.len()));
            let mut out = Vec::with_capacity(count.min(MAX_VDS_ELEMENTS));
            vds_cartesian_offsets(&per_dim, shape, &mut out)?;
            Ok(out)
        }
        VdsSelection::Blocks(blocks) => {
            let mut out = Vec::new();
            for (start, end) in blocks {
                if start.len() != shape.len() || end.len() != shape.len() {
                    return Err(LangError::new("hdf5: virtual hyperslab rank does not match the dataspace", 0));
                }
                let mut per_dim: Vec<Vec<u64>> = Vec::with_capacity(shape.len());
                for d in 0..shape.len() {
                    let lo = start[d].min(shape[d] as u64);
                    let hi = end[d].min(shape[d].saturating_sub(1) as u64);
                    per_dim.push(if hi >= lo { (lo..=hi).collect() } else { Vec::new() });
                }
                vds_cartesian_offsets(&per_dim, shape, &mut out)?;
            }
            out.sort_unstable();
            out.dedup();
            Ok(out)
        }
    }
}

fn vds_heap_object(b: &[u8], ctx: H5Ctx, addr: usize, index: usize) -> Result<&[u8], LangError> {
    if b.get(addr..addr + 4) != Some(b"GCOL") {
        return Err(LangError::new("hdf5: virtual mapping heap collection is missing", 0));
    }
    let size = len_at(b, addr + 8, ctx);
    let end = addr.saturating_add(size).min(b.len());
    let mut p = addr.saturating_add(16);
    while p + 8 + ctx.length_size <= end {
        let obj_index = u16_at(b, p).unwrap_or(0) as usize;
        let obj_size = len_at(b, p + 8, ctx);
        let data_start = p + 8 + ctx.length_size;
        if obj_index == index {
            return b
                .get(data_start..data_start.saturating_add(obj_size))
                .ok_or_else(|| LangError::new("hdf5: virtual mapping heap object is truncated", 0));
        }
        let step = 8usize.saturating_add(ctx.length_size).saturating_add(pad8(obj_size));
        if step == 0 {
            break;
        }
        p = p.saturating_add(step);
    }
    Err(LangError::new("hdf5: virtual mapping heap object was not found", 0))
}

fn parse_vds_mappings(data: &[u8], ctx: H5Ctx) -> Result<Vec<VdsMapping>, LangError> {
    if data.len() < 1 + ctx.length_size {
        return Err(LangError::new("hdf5: virtual mapping block is truncated", 0));
    }
    let version = data[0];
    if version > 1 {
        return Err(LangError::new("hdf5: unsupported virtual mapping block version", 0));
    }
    let num = len_at(data, 1, ctx);
    if num > 100_000 {
        return Err(LangError::new("hdf5: virtual mapping block has too many entries", 0));
    }
    let mut pos = 1 + ctx.length_size;
    let mut mappings: Vec<VdsMapping> = Vec::with_capacity(num);
    for _ in 0..num {
        let flags = if version >= 1 {
            let f = *data.get(pos).ok_or_else(|| LangError::new("hdf5: virtual mapping is truncated", 0))?;
            pos += 1;
            if f & !0x07 != 0 {
                return Err(LangError::new("hdf5: unknown virtual mapping flags", 0));
            }
            f
        } else {
            0
        };
        let source_file = if flags & 0x04 != 0 {
            ".".to_string()
        } else if flags & 0x01 != 0 {
            let origin = len_at(data, pos, ctx);
            pos += ctx.length_size;
            mappings.get(origin).map(|m| m.source_file.clone()).ok_or_else(|| LangError::new("hdf5: invalid shared virtual source file", 0))?
        } else {
            vds_take_cstr(data, &mut pos).ok_or_else(|| LangError::new("hdf5: virtual source file name is truncated", 0))?
        };
        let source_dataset = if flags & 0x02 != 0 {
            let origin = len_at(data, pos, ctx);
            pos += ctx.length_size;
            mappings.get(origin).map(|m| m.source_dataset.clone()).ok_or_else(|| LangError::new("hdf5: invalid shared virtual source dataset", 0))?
        } else {
            vds_take_cstr(data, &mut pos).ok_or_else(|| LangError::new("hdf5: virtual source dataset name is truncated", 0))?
        };
        let source_selection = parse_vds_selection(data, &mut pos)?;
        let virtual_selection = parse_vds_selection(data, &mut pos)?;
        if pos > data.len() {
            return Err(LangError::new("hdf5: virtual mapping block is truncated", 0));
        }
        mappings.push(VdsMapping { source_file, source_dataset, source_selection, virtual_selection });
    }
    Ok(mappings)
}

fn read_virtual(b: &[u8], ctx: H5Ctx, rctx: &H5ReadCtx, heap_addr: usize, heap_index: usize, dims: &[usize]) -> Result<Vec<f64>, LangError> {
    let count = dims.iter().fold(1usize, |a, &d| a.saturating_mul(d));
    if count > 50_000_000 {
        return Err(LangError::new("hdf5: virtual dataset is too large", 0));
    }
    let mut out = vec![0.0f64; count];
    if rctx.depth > 3 {
        return Err(LangError::new("hdf5: virtual dataset nesting is too deep", 0));
    }
    let obj = vds_heap_object(b, ctx, heap_addr, heap_index)?;
    let mappings = parse_vds_mappings(obj, ctx)?;
    for m in mappings {
        let source_file = if m.source_file.is_empty() || m.source_file == "." {
            read_bytes_opts(b, rctx.base.clone(), rctx.depth + 1, false)?
        } else {
            let path = std::path::Path::new(&m.source_file);
            let resolved = if path.is_absolute() {
                path.to_path_buf()
            } else if let Some(base) = &rctx.base {
                base.join(path)
            } else {
                path.to_path_buf()
            };
            let Ok(bytes) = std::fs::read(&resolved) else {
                continue;
            };
            let base = resolved.parent().map(|p| p.to_path_buf());
            read_bytes_opts(&bytes, base, rctx.depth + 1, true)?
        };
        let want = m.source_dataset.trim_start_matches('/');
        let Some(source) = source_file.datasets.iter().find(|d| d.name == want || d.name == m.source_dataset) else {
            continue;
        };
        let src_offsets = vds_selection_offsets(&m.source_selection, &source.dims)?;
        let dst_offsets = vds_selection_offsets(&m.virtual_selection, dims)?;
        for (&src, &dst) in src_offsets.iter().zip(dst_offsets.iter()) {
            if let (Some(&v), Some(slot)) = (source.data.get(src), out.get_mut(dst)) {
                *slot = v;
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup3_reference_values() {
        assert_eq!(lookup3(b""), 0x31b8_a510);
        assert_eq!(lookup3(b"123456789"), 0x845d_9a96);
        assert_eq!(lookup3(b"hello"), 0x34cb_bc6e);
        assert_eq!(lookup3(b"MatCraft HDF5"), 0x75ce_5b69);
    }

    #[test]
    fn roundtrip_v2_superblock_writer() {
        let data: Vec<f64> = (0..10_000).map(|i| i as f64).collect();
        let mut f = H5File::default();
        f.datasets.push(H5Dataset { name: "grid".into(), dims: vec![100, 100], data: data.clone() });
        let bytes = write_bytes(&f).unwrap();
        assert_eq!(bytes.get(8), Some(&2));
        assert_eq!(u32_at(&bytes, 44), Some(lookup3(&bytes[..44])));
        let back = read_bytes(&bytes).unwrap();
        assert_eq!(back.superblock_version, 2);
        assert_eq!(back.datasets.len(), 1);
        assert_eq!(back.datasets[0].dims, vec![100, 100]);
        assert_eq!(back.datasets[0].data, data);
    }

    #[test]
    fn roundtrip_v3_superblock_writer() {
        let data: Vec<f64> = (0..10_000).map(|i| i as f64).collect();
        let mut f = H5File { superblock_version: 3, ..H5File::default() };
        f.datasets.push(H5Dataset { name: "grid".into(), dims: vec![100, 100], data: data.clone() });
        let bytes = write_bytes(&f).unwrap();
        assert_eq!(bytes.get(8), Some(&3));
        assert_eq!(u32_at(&bytes, 44), Some(lookup3(&bytes[..44])));
        let back = read_bytes(&bytes).unwrap();
        assert_eq!(back.superblock_version, 3);
        assert_eq!(back.datasets[0].data, data);
    }

    #[test]
    fn roundtrip_large_chunk_grid() {
        // 2049x2049 with 64x64 chunks needs 1089 fixed-array entries, more
        // than the old 1024-entry single-page limit.
        let n = 2049usize;
        let data: Vec<f64> = (0..n * n).map(|i| (i % 997) as f64).collect();
        let mut f = H5File::default();
        f.datasets.push(H5Dataset { name: "large".into(), dims: vec![n, n], data: data.clone() });
        let bytes = write_bytes(&f).unwrap();
        let back = read_bytes(&bytes).unwrap();
        assert_eq!(back.datasets.len(), 1);
        assert_eq!(back.datasets[0].dims, vec![n, n]);
        assert_eq!(back.datasets[0].data.len(), data.len());
        assert_eq!(back.datasets[0].data.first(), data.first());
        assert_eq!(back.datasets[0].data.last(), data.last());
    }

    #[test]
    fn roundtrip_two_datasets() {
        let mut f = H5File::default();
        f.datasets.push(H5Dataset { name: "a".into(), dims: vec![2, 3], data: vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0] });
        f.datasets.push(H5Dataset { name: "b".into(), dims: vec![4], data: vec![9.0, 8.0, 7.0, 6.0] });
        let bytes = write_bytes(&f).unwrap();
        let back = read_bytes(&bytes).unwrap();
        assert_eq!(back.datasets.len(), 2);
        assert_eq!(back.datasets[0].dims, vec![2, 3]);
        assert_eq!(back.datasets[0].data, vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        assert_eq!(back.datasets[1].name, "b");
    }

    fn check_fixture(bytes: &[u8], name: &str, dims: &[usize], count: usize) {
        let f = read_bytes(bytes).unwrap();
        let ds = f.datasets.iter().find(|d| d.name == name).unwrap();
        assert_eq!(ds.dims, dims);
        let expected: Vec<f64> = (0..count).map(|i| i as f64).collect();
        assert_eq!(ds.data, expected);
    }

    #[test]
    fn read_superblock_v2_contiguous() {
        check_fixture(include_bytes!("../tests/fixtures/h5_v2_contiguous.h5"), "a", &[6], 6);
    }

    #[test]
    fn read_writer_v2_fixture() {
        let f = read_bytes(include_bytes!("../tests/fixtures/h5_writer_v2.h5")).unwrap();
        assert_eq!(f.superblock_version, 2);
        let grid = f.datasets.iter().find(|d| d.name == "grid").unwrap();
        assert_eq!(grid.dims, vec![2, 3]);
        assert_eq!(grid.data, vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        let line = f.datasets.iter().find(|d| d.name == "line").unwrap();
        assert_eq!(line.dims, vec![5]);
        assert_eq!(line.data, vec![9.0, 8.0, 7.0, 6.0, 5.0]);
    }

    #[test]
    fn read_superblock_v3_gzip_shuffle_fixed_array() {
        check_fixture(include_bytes!("../tests/fixtures/h5_v3_gzip.h5"), "a", &[4, 6], 24);
    }

    #[test]
    fn read_superblock_v0_gzip_btree_v1() {
        check_fixture(include_bytes!("../tests/fixtures/h5_v0_gzip.h5"), "a", &[4, 6], 24);
    }

    #[test]
    fn read_superblock_v3_gzip_shuffle_btree_v2() {
        check_fixture(include_bytes!("../tests/fixtures/h5_v3_bt2.h5"), "a", &[3, 4], 12);
    }

    #[test]
    fn read_extensible_array_plain() {
        check_fixture(include_bytes!("../tests/fixtures/h5_ea_plain.h5"), "a", &[100, 100], 10_000);
    }

    #[test]
    fn read_extensible_array_gzip() {
        check_fixture(include_bytes!("../tests/fixtures/h5_ea_gzip.h5"), "a", &[100, 100], 10_000);
    }

    #[test]
    fn read_extensible_array_second_dimension_unlimited() {
        check_fixture(include_bytes!("../tests/fixtures/h5_ea_col.h5"), "a", &[100, 100], 10_000);
    }

    #[test]
    fn read_multi_unlimited_btree_v2() {
        check_fixture(include_bytes!("../tests/fixtures/h5_bt2_multi.h5"), "a", &[20, 20], 400);
    }

    #[test]
    fn read_paged_fixed_array() {
        check_fixture(include_bytes!("../tests/fixtures/h5_fa_paged.h5"), "a", &[100, 100], 10_000);
    }

    #[test]
    fn read_virtual_dataset() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/h5_vds.h5");
        let f = read(&path).unwrap();
        let ds = f.datasets.iter().find(|d| d.name == "v").unwrap();
        assert_eq!(ds.dims, vec![20, 10]);
        let row: Vec<f64> = (0..100).map(|i| i as f64).collect();
        let expected: Vec<f64> = row.iter().chain(row.iter()).copied().collect();
        assert_eq!(ds.data, expected);
    }

    #[test]
    fn read_virtual_dataset_offset_mapping() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/h5_vds_sel.h5");
        let f = read(&path).unwrap();
        let ds = f.datasets.iter().find(|d| d.name == "v").unwrap();
        assert_eq!(ds.dims, vec![20, 20]);
        let mut expected = vec![0.0f64; 400];
        for r in 0..10 {
            for c in 0..10 {
                expected[(r + 5) * 20 + c + 5] = (r * 10 + c) as f64;
            }
        }
        assert_eq!(ds.data, expected);
    }
}
