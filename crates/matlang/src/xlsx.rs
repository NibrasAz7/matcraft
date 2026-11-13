//! Minimal `.xlsx` (OpenXML) reader/writer.
//!
//! XLSX is a ZIP container of XML parts. This module writes stored (not
//! deflated) ZIP entries and reads both stored and deflated entries, so it
//! needs only `flate2` and no extra ZIP/XML crates. The XML reader covers
//! the parts MATLAB/Excel produce for ordinary worksheets: shared strings,
//! inline strings, numbers, booleans and cached formula values.

use std::io::Read;

use flate2::read::DeflateDecoder;

use crate::LangError;

const MAX_ENTRIES: usize = 10_000;
const MAX_ENTRY_BYTES: usize = 128 * 1024 * 1024;
const MAX_CELLS: usize = 10_000_000;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Cell {
    Empty,
    Number(f64),
    Text(String),
    Bool(bool),
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Grid {
    pub rows: usize,
    pub cols: usize,
    cells: Vec<Cell>,
}

impl Grid {
    pub fn get(&self, r: usize, c: usize) -> Cell {
        if r < self.rows && c < self.cols {
            self.cells.get(c.saturating_mul(self.rows).saturating_add(r)).cloned().unwrap_or(Cell::Empty)
        } else {
            Cell::Empty
        }
    }

    pub fn set(&mut self, r: usize, c: usize, cell: Cell) -> Result<(), LangError> {
        if r >= self.rows || c >= self.cols {
            self.grow_to(r.saturating_add(1), c.saturating_add(1))?;
        }
        if let Some(slot) = self.cells.get_mut(c.saturating_mul(self.rows).saturating_add(r)) {
            *slot = cell;
        }
        Ok(())
    }

    pub fn grow_to(&mut self, rows: usize, cols: usize) -> Result<(), LangError> {
        let rows = rows.max(self.rows);
        let cols = cols.max(self.cols);
        if rows == self.rows && cols == self.cols {
            return Ok(());
        }
        let n = rows.saturating_mul(cols);
        if n > MAX_CELLS {
            return Err(LangError::new("xlsx: sheet is too large", 0));
        }
        let mut next = vec![Cell::Empty; n];
        for c in 0..self.cols {
            for r in 0..self.rows {
                if let Some(v) = self.cells.get(c.saturating_mul(self.rows).saturating_add(r)) {
                    if let Some(slot) = next.get_mut(c.saturating_mul(rows).saturating_add(r)) {
                        *slot = v.clone();
                    }
                }
            }
        }
        self.rows = rows;
        self.cols = cols;
        self.cells = next;
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Sheet {
    pub name: String,
    pub grid: Grid,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Workbook {
    pub sheets: Vec<Sheet>,
}

pub(crate) fn read_workbook(path: &std::path::Path) -> Result<Workbook, LangError> {
    let bytes = std::fs::read(path).map_err(|e| LangError::new(format!("xlsx: cannot read {}: {e}", path.display()), 0))?;
    read_workbook_bytes(&bytes)
}

pub(crate) fn write_workbook(path: &std::path::Path, wb: &Workbook) -> Result<(), LangError> {
    let bytes = write_workbook_bytes(wb)?;
    crate::write_file(path, &bytes).map_err(|e| LangError::new(format!("xlsx: cannot write {}: {e}", path.display()), 0))
}

fn read_workbook_bytes(bytes: &[u8]) -> Result<Workbook, LangError> {
    let entries = zip_read(bytes)?;
    let find = |name: &str| entries.iter().find(|(n, _)| n == name).map(|(_, d)| d.as_slice());
    let workbook = find("xl/workbook.xml").map(|d| String::from_utf8_lossy(d).to_string()).unwrap_or_default();
    let rels = find("xl/_rels/workbook.xml.rels").map(|d| String::from_utf8_lossy(d).to_string()).unwrap_or_default();
    let shared = find("xl/sharedStrings.xml").map(|d| parse_shared_strings(&String::from_utf8_lossy(d))).unwrap_or_default();
    let sheets = parse_workbook(&workbook);
    let rel_map = parse_rels(&rels);
    let mut out = Vec::new();
    if sheets.is_empty() {
        if let Some(data) = find("xl/worksheets/sheet1.xml") {
            out.push(Sheet { name: "Sheet1".into(), grid: parse_sheet(&String::from_utf8_lossy(data), &shared)? });
        }
    } else {
        for (idx, (name, rid)) in sheets.into_iter().enumerate() {
            let target = rel_map
                .iter()
                .find(|(id, _)| *id == rid)
                .map(|(_, t)| normalize_target(t))
                .unwrap_or_else(|| format!("xl/worksheets/sheet{}.xml", idx.saturating_add(1)));
            let grid = match find(&target) {
                Some(data) => parse_sheet(&String::from_utf8_lossy(data), &shared)?,
                None => Grid::default(),
            };
            out.push(Sheet { name, grid });
        }
    }
    if out.is_empty() {
        out.push(Sheet { name: "Sheet1".into(), grid: Grid::default() });
    }
    Ok(Workbook { sheets: out })
}

fn write_workbook_bytes(wb: &Workbook) -> Result<Vec<u8>, LangError> {
    let sheets = if wb.sheets.is_empty() { vec![Sheet { name: "Sheet1".into(), grid: Grid::default() }] } else { wb.sheets.clone() };
    let mut entries: Vec<(String, Vec<u8>)> = vec![("[Content_Types].xml".into(), content_types(&sheets).into_bytes())];
    entries.push((
        "_rels/.rels".into(),
        br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#.to_vec(),
    ));
    entries.push(("xl/workbook.xml".into(), workbook_xml(&sheets).into_bytes()));
    entries.push(("xl/_rels/workbook.xml.rels".into(), workbook_rels(&sheets).into_bytes()));
    for (i, sheet) in sheets.iter().enumerate() {
        entries.push((format!("xl/worksheets/sheet{}.xml", i.saturating_add(1)), sheet_xml(&sheet.grid).into_bytes()));
    }
    zip_store(&entries)
}

fn content_types(sheets: &[Sheet]) -> String {
    let mut s = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>"#,
    );
    for i in 0..sheets.len() {
        s.push_str(&format!(
            r#"<Override PartName="/xl/worksheets/sheet{}.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>"#,
            i.saturating_add(1)
        ));
    }
    s.push_str("</Types>");
    s
}

fn workbook_xml(sheets: &[Sheet]) -> String {
    let mut s = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets>"#,
    );
    for (i, sheet) in sheets.iter().enumerate() {
        s.push_str(&format!(r#"<sheet name="{}" sheetId="{}" r:id="rId{}"/>"#, xml_escape(&sheet.name), i.saturating_add(1), i.saturating_add(1)));
    }
    s.push_str("</sheets></workbook>");
    s
}

fn workbook_rels(sheets: &[Sheet]) -> String {
    let mut s = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
    );
    for i in 0..sheets.len() {
        s.push_str(&format!(
            r#"<Relationship Id="rId{}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet{}.xml"/>"#,
            i.saturating_add(1),
            i.saturating_add(1)
        ));
    }
    s.push_str("</Relationships>");
    s
}

fn sheet_xml(grid: &Grid) -> String {
    let mut s = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>"#,
    );
    for r in 0..grid.rows {
        let mut row_started = false;
        for c in 0..grid.cols {
            let cell = grid.get(r, c);
            if matches!(cell, Cell::Empty) {
                continue;
            }
            if !row_started {
                s.push_str(&format!(r#"<row r="{}">"#, r.saturating_add(1)));
                row_started = true;
            }
            let reference = format!("{}{}", col_name(c), r.saturating_add(1));
            match cell {
                Cell::Number(v) if v.is_finite() => {
                    s.push_str(&format!(r#"<c r="{reference}"><v>{v}</v></c>"#));
                }
                Cell::Number(v) => {
                    let text = if v.is_nan() {
                        "NaN"
                    } else if v > 0.0 {
                        "Inf"
                    } else {
                        "-Inf"
                    };
                    s.push_str(&format!(r#"<c r="{reference}" t="inlineStr"><is><t>{text}</t></is></c>"#));
                }
                Cell::Text(text) => {
                    s.push_str(&format!(r#"<c r="{reference}" t="inlineStr"><is><t xml:space="preserve">{}</t></is></c>"#, xml_escape(&text)));
                }
                Cell::Bool(b) => {
                    s.push_str(&format!(r#"<c r="{reference}" t="b"><v>{}</v></c>"#, if b { 1 } else { 0 }));
                }
                Cell::Empty => {}
            }
        }
        if row_started {
            s.push_str("</row>");
        }
    }
    s.push_str("</sheetData></worksheet>");
    s
}

fn normalize_target(target: &str) -> String {
    let t = target.trim_start_matches('/');
    if t.starts_with("xl/") {
        t.to_string()
    } else {
        format!("xl/{t}")
    }
}

fn parse_workbook(xml: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    while let Some(rel) = xml.get(pos..).and_then(|s| s.find("<sheet")) {
        let start = pos.saturating_add(rel);
        let Some(end) = xml.get(start..).and_then(|s| s.find('>')).map(|e| start.saturating_add(e)) else { break };
        let tag = xml.get(start..=end).unwrap_or("");
        if !tag.starts_with("<sheet ") && !tag.starts_with("<sheet>") {
            pos = end.saturating_add(1);
            continue;
        }
        let name = attr(tag, "name").unwrap_or_else(|| format!("Sheet{}", out.len().saturating_add(1)));
        let rid = attr(tag, "r:id").or_else(|| attr(tag, "id")).unwrap_or_default();
        out.push((name, rid));
        pos = end.saturating_add(1);
    }
    out
}

fn parse_rels(xml: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    while let Some(rel) = xml.get(pos..).and_then(|s| s.find("<Relationship")) {
        let start = pos.saturating_add(rel);
        let Some(end) = xml.get(start..).and_then(|s| s.find('>')).map(|e| start.saturating_add(e)) else { break };
        let tag = xml.get(start..=end).unwrap_or("");
        if let (Some(id), Some(target)) = (attr(tag, "Id"), attr(tag, "Target")) {
            out.push((id, target));
        }
        pos = end.saturating_add(1);
    }
    out
}

fn parse_shared_strings(xml: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    while let Some(rel) = xml.get(pos..).and_then(|s| s.find("<si")) {
        let start = pos.saturating_add(rel);
        if let Some(end) = xml.get(start..).and_then(|s| s.find("</si>")).map(|e| start.saturating_add(e)) {
            let fragment = xml.get(start..end).unwrap_or("");
            out.push(extract_t_text(fragment));
            pos = end.saturating_add(5);
        } else {
            break;
        }
    }
    out
}

fn extract_t_text(fragment: &str) -> String {
    let mut out = String::new();
    let mut pos = 0usize;
    while let Some(rel) = fragment.get(pos..).and_then(|s| s.find("<t")) {
        let start = pos.saturating_add(rel);
        let Some(open_end) = fragment.get(start..).and_then(|s| s.find('>')).map(|e| start.saturating_add(e)) else { break };
        let after = open_end.saturating_add(1);
        let Some(close_rel) = fragment.get(after..).and_then(|s| s.find("</t>")) else { break };
        let close = after.saturating_add(close_rel);
        out.push_str(&xml_unescape(fragment.get(after..close).unwrap_or("")));
        pos = close.saturating_add(4);
    }
    out
}

fn parse_sheet(xml: &str, shared: &[String]) -> Result<Grid, LangError> {
    let mut grid = Grid::default();
    let mut pos = 0usize;
    while let Some(rel) = xml.get(pos..).and_then(|s| s.find("<c")) {
        let start = pos.saturating_add(rel);
        let rest = xml.get(start.saturating_add(2)..).unwrap_or("");
        let Some(first) = rest.chars().next() else { break };
        if !first.is_ascii_whitespace() && first != '>' {
            pos = start.saturating_add(2);
            continue;
        }
        let Some(open_end) = xml.get(start..).and_then(|s| s.find('>')).map(|e| start.saturating_add(e)) else { break };
        let tag = xml.get(start..=open_end).unwrap_or("");
        let self_closing = tag.trim_end().ends_with("/>");
        let content_end = if self_closing {
            open_end
        } else {
            match xml.get(open_end.saturating_add(1)..).and_then(|s| s.find("</c>")) {
                Some(e) => open_end.saturating_add(1).saturating_add(e),
                None => break,
            }
        };
        let next_pos = if self_closing { open_end.saturating_add(1) } else { content_end.saturating_add(4) };
        let content = xml.get(open_end.saturating_add(1)..content_end).unwrap_or("");
        if let Some(reference) = attr(tag, "r") {
            if let Some((row, col)) = parse_ref(&reference) {
                let kind = attr(tag, "t").unwrap_or_default();
                let cell = if self_closing {
                    Cell::Empty
                } else if kind == "inlineStr" {
                    let text = extract_t_text(content);
                    if text.is_empty() {
                        Cell::Empty
                    } else {
                        Cell::Text(text)
                    }
                } else if let Some(v) = value_between(content, "<v", "</v>") {
                    if kind == "s" {
                        let idx = v.parse::<usize>().unwrap_or(usize::MAX);
                        shared.get(idx).map(|s| Cell::Text(s.clone())).unwrap_or(Cell::Empty)
                    } else if kind == "b" {
                        Cell::Bool(v.trim() == "1")
                    } else if kind == "str" || kind == "e" {
                        Cell::Text(xml_unescape(v))
                    } else if let Ok(n) = v.trim().parse::<f64>() {
                        Cell::Number(n)
                    } else {
                        Cell::Text(xml_unescape(v))
                    }
                } else {
                    Cell::Empty
                };
                grid.set(row.saturating_sub(1), col.saturating_sub(1), cell)?;
            }
        }
        pos = next_pos;
    }
    Ok(grid)
}

fn value_between<'a>(fragment: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let start = fragment.find(open)?;
    let after = fragment.get(start..)?.find('>')? + start + 1;
    let end = fragment.get(after..)?.find(close)? + after;
    fragment.get(after..end)
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let needle = format!("{name}=");
    let mut from = 0usize;
    while let Some(rel) = tag.get(from..)?.find(&needle) {
        let pos = from.saturating_add(rel);
        let before_ok =
            pos == 0 || tag.get(pos.saturating_sub(1)..pos).and_then(|s| s.chars().next()).map(|c| c.is_whitespace() || c == '<').unwrap_or(false);
        if before_ok {
            let rest = tag.get(pos.saturating_add(needle.len())..)?;
            let quote = rest.chars().next()?;
            if quote != '"' && quote != '\'' {
                return None;
            }
            let end = rest.get(1..)?.find(quote)? + 1;
            return Some(xml_unescape(rest.get(1..end)?));
        }
        from = pos.saturating_add(needle.len());
    }
    None
}

fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

fn xml_unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '&' {
            out.push(c);
            continue;
        }
        let mut ent = String::new();
        while let Some(&n) = chars.peek() {
            if n == ';' || ent.len() > 12 {
                break;
            }
            ent.push(n);
            chars.next();
        }
        if chars.peek() == Some(&';') {
            chars.next();
        }
        match ent.as_str() {
            "amp" => out.push('&'),
            "lt" => out.push('<'),
            "gt" => out.push('>'),
            "quot" => out.push('"'),
            "apos" => out.push('\''),
            _ => {
                if let Some(hex) = ent.strip_prefix("#x").or_else(|| ent.strip_prefix("#X")) {
                    if let Ok(v) = u32::from_str_radix(hex, 16) {
                        if let Some(ch) = char::from_u32(v) {
                            out.push(ch);
                        }
                    }
                } else if let Some(dec) = ent.strip_prefix('#') {
                    if let Ok(v) = dec.parse::<u32>() {
                        if let Some(ch) = char::from_u32(v) {
                            out.push(ch);
                        }
                    }
                } else {
                    out.push('&');
                    out.push_str(&ent);
                    out.push(';');
                }
            }
        }
    }
    out
}

pub(crate) fn parse_ref(s: &str) -> Option<(usize, usize)> {
    let s = s.trim().trim_start_matches('$');
    let mut col = 0usize;
    let mut i = 0usize;
    for (idx, c) in s.char_indices() {
        if c.is_ascii_alphabetic() {
            col = col.saturating_mul(26).saturating_add((c.to_ascii_uppercase() as u8).saturating_sub(b'A') as usize + 1);
            i = idx + c.len_utf8();
        } else {
            break;
        }
    }
    if col == 0 || i >= s.len() {
        return None;
    }
    let row = s.get(i..)?.trim_start_matches('$').parse::<usize>().ok()?;
    if row == 0 {
        return None;
    }
    Some((row, col))
}

pub(crate) fn col_name(col: usize) -> String {
    let mut n = col.saturating_add(1);
    let mut out = String::new();
    while n > 0 {
        let rem = (n - 1) % 26;
        out.insert(0, (b'A' + rem as u8) as char);
        n = (n - 1) / 26;
    }
    out
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

fn push_u16(out: &mut Vec<u8>, v: usize) -> Result<(), LangError> {
    let v = u16::try_from(v).map_err(|_| LangError::new("xlsx: value too large for ZIP", 0))?;
    out.extend_from_slice(&v.to_le_bytes());
    Ok(())
}

fn push_u32(out: &mut Vec<u8>, v: usize) -> Result<(), LangError> {
    let v = u32::try_from(v).map_err(|_| LangError::new("xlsx: value too large for ZIP", 0))?;
    out.extend_from_slice(&v.to_le_bytes());
    Ok(())
}

fn zip_store(entries: &[(String, Vec<u8>)]) -> Result<Vec<u8>, LangError> {
    if entries.len() > MAX_ENTRIES {
        return Err(LangError::new("xlsx: too many ZIP entries", 0));
    }
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in entries {
        if data.len() > MAX_ENTRY_BYTES {
            return Err(LangError::new("xlsx: ZIP entry is too large", 0));
        }
        let offset = out.len();
        let crc = crc32(data);
        out.extend_from_slice(&0x0403_4B50u32.to_le_bytes());
        push_u16(&mut out, 20)?;
        push_u16(&mut out, 0)?;
        push_u16(&mut out, 0)?;
        push_u16(&mut out, 0)?;
        push_u16(&mut out, 0)?;
        push_u32(&mut out, crc as usize)?;
        push_u32(&mut out, data.len())?;
        push_u32(&mut out, data.len())?;
        push_u16(&mut out, name.len())?;
        push_u16(&mut out, 0)?;
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);
        central.extend_from_slice(&0x0201_4B50u32.to_le_bytes());
        push_u16(&mut central, 20)?;
        push_u16(&mut central, 20)?;
        push_u16(&mut central, 0)?;
        push_u16(&mut central, 0)?;
        push_u16(&mut central, 0)?;
        push_u16(&mut central, 0)?;
        push_u32(&mut central, crc as usize)?;
        push_u32(&mut central, data.len())?;
        push_u32(&mut central, data.len())?;
        push_u16(&mut central, name.len())?;
        push_u16(&mut central, 0)?;
        push_u16(&mut central, 0)?;
        push_u16(&mut central, 0)?;
        push_u16(&mut central, 0)?;
        push_u32(&mut central, 0)?;
        push_u32(&mut central, offset)?;
        central.extend_from_slice(name.as_bytes());
    }
    let cd_offset = out.len();
    let cd_size = central.len();
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x0605_4B50u32.to_le_bytes());
    push_u16(&mut out, 0)?;
    push_u16(&mut out, 0)?;
    push_u16(&mut out, entries.len())?;
    push_u16(&mut out, entries.len())?;
    push_u32(&mut out, cd_size)?;
    push_u32(&mut out, cd_offset)?;
    push_u16(&mut out, 0)?;
    Ok(out)
}

fn le_u16(b: &[u8], pos: usize) -> Option<usize> {
    let s = b.get(pos..pos.saturating_add(2))?;
    Some(u16::from_le_bytes([*s.first()?, *s.get(1)?]) as usize)
}

fn le_u32(b: &[u8], pos: usize) -> Option<usize> {
    let s = b.get(pos..pos.saturating_add(4))?;
    Some(u32::from_le_bytes([*s.first()?, *s.get(1)?, *s.get(2)?, *s.get(3)?]) as usize)
}

fn zip_read(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>, LangError> {
    if bytes.len() < 22 {
        return Err(LangError::new("xlsx: not a ZIP archive", 0));
    }
    let mut eocd = None;
    let start = bytes.len().saturating_sub(22);
    let lower = bytes.len().saturating_sub(22usize.saturating_add(65_535));
    for pos in (lower..=start).rev() {
        if le_u32(bytes, pos) == Some(0x0605_4B50) {
            eocd = Some(pos);
            break;
        }
    }
    let eocd = eocd.ok_or_else(|| LangError::new("xlsx: ZIP end-of-central-directory not found", 0))?;
    let count = le_u16(bytes, eocd.saturating_add(10)).unwrap_or(0);
    if count > MAX_ENTRIES {
        return Err(LangError::new("xlsx: too many ZIP entries", 0));
    }
    let mut pos = le_u32(bytes, eocd.saturating_add(16)).ok_or_else(|| LangError::new("xlsx: bad ZIP central directory", 0))?;
    let mut out = Vec::new();
    for _ in 0..count {
        if le_u32(bytes, pos) != Some(0x0201_4B50) {
            return Err(LangError::new("xlsx: bad ZIP central-directory entry", 0));
        }
        let method = le_u16(bytes, pos.saturating_add(10)).unwrap_or(0);
        let csize = le_u32(bytes, pos.saturating_add(20)).ok_or_else(|| LangError::new("xlsx: bad ZIP entry size", 0))?;
        let usize_ = le_u32(bytes, pos.saturating_add(24)).ok_or_else(|| LangError::new("xlsx: bad ZIP entry size", 0))?;
        let nlen = le_u16(bytes, pos.saturating_add(28)).unwrap_or(0);
        let elen = le_u16(bytes, pos.saturating_add(30)).unwrap_or(0);
        let clen = le_u16(bytes, pos.saturating_add(32)).unwrap_or(0);
        let local = le_u32(bytes, pos.saturating_add(42)).ok_or_else(|| LangError::new("xlsx: bad ZIP local offset", 0))?;
        let name = String::from_utf8_lossy(bytes.get(pos.saturating_add(46)..pos.saturating_add(46).saturating_add(nlen)).unwrap_or(&[])).to_string();
        if usize_ > MAX_ENTRY_BYTES || csize > MAX_ENTRY_BYTES {
            return Err(LangError::new("xlsx: ZIP entry is too large", 0));
        }
        if le_u32(bytes, local) != Some(0x0403_4B50) {
            return Err(LangError::new("xlsx: bad ZIP local header", 0));
        }
        let lnlen = le_u16(bytes, local.saturating_add(26)).unwrap_or(0);
        let lelen = le_u16(bytes, local.saturating_add(28)).unwrap_or(0);
        let data_start = local.saturating_add(30).saturating_add(lnlen).saturating_add(lelen);
        let raw = bytes.get(data_start..data_start.saturating_add(csize)).ok_or_else(|| LangError::new("xlsx: truncated ZIP entry", 0))?;
        let data = match method {
            0 => raw.to_vec(),
            8 => {
                let mut d = Vec::new();
                DeflateDecoder::new(raw).read_to_end(&mut d).map_err(|e| LangError::new(format!("xlsx: deflate error: {e}"), 0))?;
                d
            }
            _ => return Err(LangError::new(format!("xlsx: unsupported ZIP compression method {method}"), 0)),
        };
        if data.len() > MAX_ENTRY_BYTES {
            return Err(LangError::new("xlsx: ZIP entry is too large", 0));
        }
        out.push((name, data));
        pos = pos.saturating_add(46).saturating_add(nlen).saturating_add(elen).saturating_add(clen);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_refs_and_columns() {
        assert_eq!(parse_ref("A1"), Some((1, 1)));
        assert_eq!(parse_ref("B3"), Some((3, 2)));
        assert_eq!(parse_ref("AA10"), Some((10, 27)));
        assert_eq!(col_name(0), "A");
        assert_eq!(col_name(25), "Z");
        assert_eq!(col_name(26), "AA");
    }

    #[test]
    fn zip_roundtrip() {
        let entries = vec![("a.txt".to_string(), b"hello".to_vec()), ("b.bin".to_string(), vec![1u8, 2, 3])];
        let z = zip_store(&entries).unwrap();
        let got = zip_read(&z).unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].0, "a.txt");
        assert_eq!(got[0].1, b"hello");
        assert_eq!(got[1].1, vec![1, 2, 3]);
    }

    #[test]
    fn parse_shared_and_cells() {
        let shared = vec!["hello".to_string(), "world".to_string()];
        let xml = r#"<worksheet><sheetData><row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1"><v>3.5</v></c><c r="C1" t="b"><v>1</v></c><c r="D1" t="inlineStr"><is><t>inline</t></is></c></row></sheetData></worksheet>"#;
        let grid = parse_sheet(xml, &shared).unwrap();
        assert_eq!(grid.rows, 1);
        assert_eq!(grid.cols, 4);
        assert_eq!(grid.get(0, 0), Cell::Text("hello".into()));
        assert_eq!(grid.get(0, 1), Cell::Number(3.5));
        assert_eq!(grid.get(0, 2), Cell::Bool(true));
        assert_eq!(grid.get(0, 3), Cell::Text("inline".into()));
    }

    fn zip_one_deflated(name: &str, data: &[u8]) -> Vec<u8> {
        use flate2::write::DeflateEncoder;
        use flate2::Compression;
        use std::io::Write;
        let mut comp = Vec::new();
        let mut enc = DeflateEncoder::new(&mut comp, Compression::default());
        enc.write_all(data).unwrap();
        enc.finish().unwrap();
        let crc = crc32(data);
        let mut out = Vec::new();
        let mut central = Vec::new();
        out.extend_from_slice(&0x0403_4B50u32.to_le_bytes());
        push_u16(&mut out, 20).unwrap();
        push_u16(&mut out, 0).unwrap();
        push_u16(&mut out, 8).unwrap();
        push_u16(&mut out, 0).unwrap();
        push_u16(&mut out, 0).unwrap();
        push_u32(&mut out, crc as usize).unwrap();
        push_u32(&mut out, comp.len()).unwrap();
        push_u32(&mut out, data.len()).unwrap();
        push_u16(&mut out, name.len()).unwrap();
        push_u16(&mut out, 0).unwrap();
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&comp);
        central.extend_from_slice(&0x0201_4B50u32.to_le_bytes());
        push_u16(&mut central, 20).unwrap();
        push_u16(&mut central, 20).unwrap();
        push_u16(&mut central, 0).unwrap();
        push_u16(&mut central, 8).unwrap();
        push_u16(&mut central, 0).unwrap();
        push_u16(&mut central, 0).unwrap();
        push_u32(&mut central, crc as usize).unwrap();
        push_u32(&mut central, comp.len()).unwrap();
        push_u32(&mut central, data.len()).unwrap();
        push_u16(&mut central, name.len()).unwrap();
        push_u16(&mut central, 0).unwrap();
        push_u16(&mut central, 0).unwrap();
        push_u16(&mut central, 0).unwrap();
        push_u16(&mut central, 0).unwrap();
        push_u32(&mut central, 0).unwrap();
        push_u32(&mut central, 0).unwrap();
        central.extend_from_slice(name.as_bytes());
        let cd_offset = out.len();
        out.extend_from_slice(&central);
        out.extend_from_slice(&0x0605_4B50u32.to_le_bytes());
        push_u16(&mut out, 0).unwrap();
        push_u16(&mut out, 0).unwrap();
        push_u16(&mut out, 1).unwrap();
        push_u16(&mut out, 1).unwrap();
        push_u32(&mut out, central.len()).unwrap();
        push_u32(&mut out, cd_offset).unwrap();
        push_u16(&mut out, 0).unwrap();
        out
    }

    #[test]
    fn zip_read_deflated() {
        let z = zip_one_deflated("a.txt", b"hello deflate");
        let got = zip_read(&z).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, "a.txt");
        assert_eq!(got[0].1, b"hello deflate");
    }

    #[test]
    fn shared_strings_workbook() {
        let entries = vec![
            ("xl/workbook.xml".to_string(), br#"<workbook><sheets><sheet name="Data" sheetId="1" r:id="rId1"/></sheets></workbook>"#.to_vec()),
            (
                "xl/_rels/workbook.xml.rels".to_string(),
                br#"<Relationships><Relationship Id="rId1" Type="worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#.to_vec(),
            ),
            ("xl/sharedStrings.xml".to_string(), br#"<sst><si><t>Hello</t></si><si><r><t>Wor</t></r><r><t>ld</t></r></si></sst>"#.to_vec()),
            (
                "xl/worksheets/sheet1.xml".to_string(),
                br#"<worksheet><sheetData><row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" t="s"><v>1</v></c></row></sheetData></worksheet>"#
                    .to_vec(),
            ),
        ];
        let z = zip_store(&entries).unwrap();
        let wb = read_workbook_bytes(&z).unwrap();
        assert_eq!(wb.sheets.len(), 1);
        assert_eq!(wb.sheets[0].name, "Data");
        assert_eq!(wb.sheets[0].grid.get(0, 0), Cell::Text("Hello".into()));
        assert_eq!(wb.sheets[0].grid.get(0, 1), Cell::Text("World".into()));
    }

    #[test]
    fn workbook_roundtrip_bytes() {
        let mut wb = Workbook::default();
        let mut grid = Grid::default();
        grid.set(0, 0, Cell::Number(42.0)).unwrap();
        grid.set(0, 1, Cell::Text("x & y".into())).unwrap();
        wb.sheets.push(Sheet { name: "Data".into(), grid });
        let bytes = write_workbook_bytes(&wb).unwrap();
        let back = read_workbook_bytes(&bytes).unwrap();
        assert_eq!(back.sheets.len(), 1);
        assert_eq!(back.sheets[0].name, "Data");
        assert_eq!(back.sheets[0].grid.get(0, 0), Cell::Number(42.0));
        assert_eq!(back.sheets[0].grid.get(0, 1), Cell::Text("x & y".into()));
    }
}
