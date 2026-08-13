//! Read GPG's own record of an installed app from store.db.
//!
//! %LOCALAPPDATA%\Google\Play Games\store.db — SQLite, table UserSettingsState, BLOB = protobuf.
//! Per app under .3.1: field 1 = package, .4.5.1 = app version, and .5.1 / .5.2 = { 1: width, 2: height }
//! for the maximum option and the current render resolution.
//!
//! The BLOB is stored contiguously and GPG's own handle allows a shared read, so the raw bytes can be
//! read and walked while GPG is running.
//!
//! This is the resolution the game content renders at, so it tracks the monitor's aspect ratio and is the
//! source of truth for the window ratio check.

use std::path::PathBuf;

type Wh = (u32, u32);

/// Fields past the package are optional so an app GPG has merely heard of still counts as present.
pub struct AppRecord {
    pub package: String,
    version: Option<String>,
    cur: Option<Wh>,
    max: Option<Wh>,
}

impl AppRecord {
    pub fn describe(&self) -> String {
        let wh = |v: Option<Wh>| v.map_or_else(|| "?".to_string(), |(w, h)| format!("{}x{}", w, h));
        format!("{} v{} render {} (max {})", self.package, self.version.as_deref().unwrap_or("?"), wh(self.cur), wh(self.max))
    }
}

/// None means never launched through GPG, so the launch URI would only raise GPG's own window.
pub fn app_record(package: &str) -> Option<AppRecord> {
    let bytes = std::fs::read(store_db_path()?).ok()?;
    let packages = find_packages(&bytes);

    // The package name also sits in store URLs and in pages SQLite has freed, so the fullest hit wins.
    let mut fallback: Option<AppRecord> = None;
    for (index, (offset, name)) in packages.iter().enumerate() {
        if !name.starts_with(package) {
            continue;
        }
        let end = packages.get(index + 1).map_or(bytes.len(), |(o, _)| *o);
        let record = parse_entry(&bytes[..end], *offset, name.clone());
        if record.version.is_some() && record.cur.is_some() {
            return Some(record);
        }
        fallback.get_or_insert(record);
    }
    fallback
}

pub fn render_resolution(package: &str) -> Option<Wh> {
    app_record(package)?.cur
}

fn store_db_path() -> Option<PathBuf> {
    let local = std::env::var_os("LOCALAPPDATA")?;
    Some(PathBuf::from(local).join("Google").join("Play Games").join("store.db"))
}

// A record opens with its own package string, so the fields that follow it are that app's.
fn parse_entry(buf: &[u8], package_offset: usize, package: String) -> AppRecord {
    let mut record = AppRecord { package, version: None, cur: None, max: None };
    let mut pos = package_offset + record.package.len();

    while pos < buf.len() && (record.version.is_none() || record.cur.is_none()) {
        let Some((field, wire)) = read_tag(buf, &mut pos) else {
            break;
        };
        if wire != 2 {
            if skip_wire(buf, &mut pos, wire).is_none() {
                break;
            }
            continue;
        }
        let Some(len) = read_varint(buf, &mut pos).map(|l| l as usize).filter(|l| pos + l <= buf.len()) else {
            break;
        };
        let sub = &buf[pos..pos + len];
        pos += len;

        match field {
            4 => record.version = find_version(sub),
            5 => {
                if let Some((max, cur)) = parse_res_group(sub) {
                    record.max = max.filter(|&(w, h)| plausible(w, h));
                    record.cur = cur.filter(|&(w, h)| plausible(w, h));
                }
            }
            _ => {}
        }
    }

    record
}

// field 5 sub-message: field 1 = the largest option offered, field 2 = the one in effect.
fn parse_res_group(sub: &[u8]) -> Option<(Option<Wh>, Option<Wh>)> {
    let mut pos = 0;
    let (mut max, mut cur) = (None, None);
    while pos < sub.len() {
        let (field, wire) = read_tag(sub, &mut pos)?;
        if wire == 2 {
            let len = read_varint(sub, &mut pos)? as usize;
            if pos + len > sub.len() {
                return None;
            }
            let inner = &sub[pos..pos + len];
            pos += len;
            match field {
                1 => max = parse_wh(inner),
                2 => cur = parse_wh(inner),
                _ => {}
            }
        } else {
            skip_wire(sub, &mut pos, wire)?;
        }
    }
    Some((max, cur))
}

// field 4 sub-message: the app version sits at .5.1.
fn find_version(sub: &[u8]) -> Option<String> {
    let bytes = nested_field(nested_field(sub, 5)?, 1)?;
    std::str::from_utf8(bytes).ok().map(str::to_owned)
}

fn nested_field(sub: &[u8], field: u8) -> Option<&[u8]> {
    let mut pos = 0;
    while pos < sub.len() {
        let (f, wire) = read_tag(sub, &mut pos)?;
        if wire == 2 {
            let len = read_varint(sub, &mut pos)? as usize;
            if pos + len > sub.len() {
                return None;
            }
            if f == field {
                return Some(&sub[pos..pos + len]);
            }
            pos += len;
        } else {
            skip_wire(sub, &mut pos, wire)?;
        }
    }
    None
}

// resolution sub-message: field 1 = width, field 2 = height.
fn parse_wh(sub: &[u8]) -> Option<Wh> {
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

// Length-delimited bytes are bounds-checked so a truncated BLOB fails the parse.
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
