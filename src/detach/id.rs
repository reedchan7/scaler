//! Run id: `<YYYYMMDD-HHMMSS>-<4 lowercase hex>`, local time when possible.

use std::time::{SystemTime, UNIX_EPOCH};

use time::{OffsetDateTime, UtcOffset, macros::format_description};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RunId(String);

impl RunId {
    pub fn generate() -> Self {
        let now = OffsetDateTime::now_local()
            .unwrap_or_else(|_| OffsetDateTime::now_utc().to_offset(UtcOffset::UTC));
        let fmt = format_description!("[year][month][day]-[hour][minute][second]");
        let stamp = now.format(&fmt).expect("format is total");
        let hex = random_hex4();
        Self(format!("{stamp}-{hex}"))
    }

    pub fn parse(s: &str) -> Option<Self> {
        if s.len() != 20 {
            return None;
        }
        let bytes = s.as_bytes();
        if bytes[8] != b'-' || bytes[15] != b'-' {
            return None;
        }
        if !bytes[..8].iter().all(u8::is_ascii_digit) {
            return None;
        }
        if !bytes[9..15].iter().all(u8::is_ascii_digit) {
            return None;
        }
        if !bytes[16..20]
            .iter()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
        {
            return None;
        }
        Some(Self(s.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn find_by_prefix<'a>(needle: &str, all: &'a [RunId]) -> Option<&'a RunId> {
        if let Some(hit) = all.iter().find(|id| id.as_str() == needle) {
            return Some(hit);
        }
        let mut iter = all.iter().filter(|id| id.as_str().starts_with(needle));
        let first = iter.next()?;
        if iter.next().is_some() {
            return None;
        }
        Some(first)
    }
}

/// 16 bits of entropy from a combination of process id, high-resolution
/// nanoseconds since the epoch, and a per-call counter. Not cryptographic,
/// but good enough to avoid id collisions within a wall-clock second.
fn random_hex4() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let pid = std::process::id();
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut x = nanos.wrapping_mul(0x9E37_79B9) ^ pid ^ seq.wrapping_mul(0x85EB_CA77);
    x ^= x >> 16;
    x = x.wrapping_mul(0x7FEB_352D);
    x ^= x >> 15;
    format!("{:04x}", x & 0xFFFF)
}
