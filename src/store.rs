//! Read the currently-selected GPG render resolution from store.db.
//!
//! GPG stores the "screen resolution" setting in
//!   %LOCALAPPDATA%\Google\Play Games\store.db   (SQLite, table UserSettingsState, BLOB = protobuf).
//! Per-package the resolution lives at protobuf path .3.1.5 : field 1 = max option, field 2 = current selection,
//! each a sub-message { 1: width, 2: height }.
//!
//! No SQLite/protobuf crate is needed: the BLOB is stored contiguously, so we read the raw bytes (shared read works
//! while GPG holds the file) and walk the wire format for the resolution groups. The value is the resolution the game
//! content is rendered at, which tracks the monitor's aspect ratio — a non-16:9 monitor yields non-16:9 values here,
//! so it is the source of truth for the window aspect-ratio check.

use std::path::PathBuf;

/// Currently-selected render resolution (width, height) for `package`, or None if store.db is unreadable / the entry
/// is absent (schema changed, package never launched).
pub fn render_resolution(package: &str) -> Option<(u32, u32)> {
    let bytes = std::fs::read(store_db_path()?).ok()?;
    scan(&bytes).into_iter().find(|e| e.package.starts_with(package)).map(|e| e.cur)
}

fn store_db_path() -> Option<PathBuf> {
    let local = std::env::var_os("LOCALAPPDATA")?;
    Some(PathBuf::from(local).join("Google").join("Play Games").join("store.db"))
}

struct Entry {
    package: String,
    cur: (u32, u32),
}

// Scan the raw file for resolution groups (protobuf field 5, tag 0x2A) and pair each with the nearest preceding
// package string. The structured parse plus the plausibility bound make false positives effectively impossible.
fn scan(buf: &[u8]) -> Vec<Entry> {
    let pkgs = find_packages(buf);
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < buf.len() {
        if buf[i] == 0x2A {
            // field 5, wire type 2 (length-delimited)
            let mut pos = i + 1;
            if let Some(len) = read_varint(buf, &mut pos) {
                let len = len as usize;
                if pos + len <= buf.len() {
                    if let Some(curr) = parse_res_group(&buf[pos..pos + len]) {
                        if let Some((w, h)) = curr.filter(|&(w, h)| plausible(w, h)) {
                            out.push(Entry { package: nearest_package(&pkgs, i), cur: (w, h) });
                            i = pos + len;
                            continue;
                        }
                    }
                }
            }
        }
        i += 1;
    }
    out
}

// field 5 sub-message: field 2 = current resolution (field 1 = max option, which we don't need).
fn parse_res_group(sub: &[u8]) -> Option<Option<(u32, u32)>> {
    let mut pos = 0;
    let mut curr = None;
    while pos < sub.len() {
        let (field, wire) = read_tag(sub, &mut pos)?;
        if wire == 2 {
            let len = read_varint(sub, &mut pos)? as usize;
            if pos + len > sub.len() {
                return None;
            }
            let inner = &sub[pos..pos + len];
            pos += len;
            if field == 2 {
                curr = parse_wh(inner);
            }
        } else {
            skip_wire(sub, &mut pos, wire)?;
        }
    }
    Some(curr)
}

// resolution sub-message: field 1 = width, field 2 = height.
fn parse_wh(sub: &[u8]) -> Option<(u32, u32)> {
    let mut pos = 0;
    let (mut w, mut h) = (None, None);
    while pos < sub.len() {
        let (field, wire) = read_tag(sub, &mut pos)?;
        if wire == 0 {
            let v = read_varint(sub, &mut pos)?;
            match field {
                1 => w = Some(v as u32),
                2 => h = Some(v as u32),
                _ => {}
            }
        } else {
            skip_wire(sub, &mut pos, wire)?;
        }
    }
    Some((w?, h?))
}

// Advance past one non-target field; length-delimited bytes are bounds-checked so a truncated BLOB fails the parse.
fn skip_wire(buf: &[u8], pos: &mut usize, wire: u8) -> Option<()> {
    match wire {
        0 => {
            read_varint(buf, pos)?;
        }
        2 => {
            let len = read_varint(buf, pos)? as usize;
            if *pos + len > buf.len() {
                return None;
            }
            *pos += len;
        }
        5 => *pos += 4,
        1 => *pos += 8,
        _ => return None,
    }
    Some(())
}

fn read_tag(buf: &[u8], pos: &mut usize) -> Option<(u8, u8)> {
    let tag = read_varint(buf, pos)?;
    Some(((tag >> 3) as u8, (tag & 7) as u8))
}

fn read_varint(buf: &[u8], pos: &mut usize) -> Option<u64> {
    let mut result = 0u64;
    let mut shift = 0u32;
    while *pos < buf.len() {
        let b = buf[*pos];
        *pos += 1;
        result |= ((b & 0x7f) as u64) << shift;
        if b & 0x80 == 0 {
            return Some(result);
        }
        shift += 7;
        if shift >= 64 {
            return None;
        }
    }
    None
}

fn plausible(w: u32, h: u32) -> bool {
    (200..=10000).contains(&w) && (200..=10000).contains(&h)
}

// Collect (offset, package) for every "com.…"-style id embedded in the BLOB.
fn find_packages(buf: &[u8]) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i + 4 <= buf.len() {
        if &buf[i..i + 4] == b"com." {
            let start = i;
            let mut j = i;
            while j < buf.len() && is_pkg_byte(buf[j]) {
                j += 1;
            }
            if j - start >= 8 {
                out.push((start, String::from_utf8_lossy(&buf[start..j]).into_owned()));
            }
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

fn is_pkg_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'.' || b == b'_'
}

fn nearest_package(pkgs: &[(usize, String)], offset: usize) -> String {
    pkgs.iter()
        .filter(|(o, _)| *o < offset)
        .max_by_key(|(o, _)| *o)
        .map(|(_, name)| name.clone())
        .unwrap_or_else(|| "<unknown>".to_string())
}
