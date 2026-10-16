//! The interface's font for Chinese, Japanese and Korean text, and how to line it up with the
//! system font.
//!
//! egui places a fallback font's glyphs by centring the difference between its line height and
//! the primary font's. CJK fonts often carry a large line gap (Hiragino Sans GB: half an em),
//! which lifts their glyphs a quarter of an em above the Latin baseline. [`baseline_offset`]
//! computes the shift that puts both on the same baseline, from the fonts' own metrics.

use std::path::{Path, PathBuf};

/// Vertical metrics in em: ascent (positive), descent (negative), line gap.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct VMetrics {
    pub ascent: f32,
    pub descent: f32,
    pub gap: f32,
}

impl VMetrics {
    fn height(self) -> f32 {
        self.ascent - self.descent + self.gap
    }
}

fn u16_at(d: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*d.get(o)?, *d.get(o.checked_add(1)?)?]))
}

fn i16_at(d: &[u8], o: usize) -> Option<i16> {
    u16_at(d, o).map(|v| v as i16)
}

fn u32_at(d: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_be_bytes([
        *d.get(o)?,
        *d.get(o.checked_add(1)?)?,
        *d.get(o.checked_add(2)?)?,
        *d.get(o.checked_add(3)?)?,
    ]))
}

/// Offset of face `index`'s table directory (a collection or a single font).
fn face_offset(d: &[u8], index: u32) -> Option<usize> {
    if d.get(0..4)? == b"ttcf" {
        let count = u32_at(d, 8)?;
        if index >= count {
            return None;
        }
        usize::try_from(u32_at(d, 12usize.checked_add(4 * index as usize)?)?).ok()
    } else {
        (index == 0).then_some(0)
    }
}

/// Number of faces in the file.
fn face_count(d: &[u8]) -> u32 {
    if d.get(0..4) == Some(b"ttcf") {
        u32_at(d, 8).unwrap_or(0)
    } else {
        1
    }
}

/// Offset of table `tag` of the face at `face`.
fn table(d: &[u8], face: usize, tag: &[u8; 4]) -> Option<usize> {
    let n = u16_at(d, face.checked_add(4)?)?;
    (0..usize::from(n)).find_map(|i| {
        let rec = face.checked_add(12)?.checked_add(16 * i)?;
        (d.get(rec..rec.checked_add(4)?)? == tag)
            .then(|| u32_at(d, rec + 8).and_then(|o| usize::try_from(o).ok()))
            .flatten()
    })
}

/// The face's full name (name ID 4, Windows English).
fn full_name(d: &[u8], face: usize) -> Option<String> {
    let name = table(d, face, b"name")?;
    let count = u16_at(d, name + 2)?;
    let strings = name.checked_add(usize::from(u16_at(d, name + 4)?))?;
    (0..usize::from(count)).find_map(|i| {
        let r = name.checked_add(6)?.checked_add(12 * i)?;
        let (platform, lang, id) = (u16_at(d, r)?, u16_at(d, r + 4)?, u16_at(d, r + 6)?);
        if platform != 3 || lang != 0x409 || id != 4 {
            return None;
        }
        let (len, off) = (
            usize::from(u16_at(d, r + 8)?),
            usize::from(u16_at(d, r + 10)?),
        );
        let start = strings.checked_add(off)?;
        let bytes = d.get(start..start.checked_add(len)?)?;
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect();
        String::from_utf16(&units).ok()
    })
}

/// The index of the face named `name` in a font collection.
pub(crate) fn face_index(d: &[u8], name: &str) -> Option<u32> {
    (0..face_count(d)).find(|&i| {
        face_offset(d, i)
            .and_then(|f| full_name(d, f))
            .is_some_and(|n| n == name)
    })
}

/// Vertical metrics as font renderers read them: the OS/2 typographic values when the font asks
/// for them (`USE_TYPO_METRICS`), else `hhea`.
pub(crate) fn vmetrics(d: &[u8], index: u32) -> Option<VMetrics> {
    let face = face_offset(d, index)?;
    let head = table(d, face, b"head")?;
    let upm = f32::from(u16_at(d, head + 18)?.max(1));
    let os2 = table(d, face, b"OS/2");
    let typo = os2.and_then(|o| {
        let use_typo = u16_at(d, o + 62)? & (1 << 7) != 0;
        use_typo.then(|| Some((i16_at(d, o + 68)?, i16_at(d, o + 70)?, i16_at(d, o + 72)?)))?
    });
    let (a, de, g) = match typo {
        Some(t) => t,
        None => {
            let hhea = table(d, face, b"hhea")?;
            (
                i16_at(d, hhea + 4)?,
                i16_at(d, hhea + 6)?,
                i16_at(d, hhea + 8)?,
            )
        }
    };
    Some(VMetrics {
        ascent: f32::from(a) / upm,
        descent: f32::from(de) / upm,
        gap: f32::from(g) / upm,
    })
}

/// How far (in em) to move the fallback font's glyphs down so its baseline meets the primary
/// font's, given how egui centres fallback glyphs.
pub(crate) fn baseline_offset(primary: VMetrics, fallback: VMetrics) -> f32 {
    let placed = fallback.ascent + 0.5 * (primary.height() - fallback.height());
    primary.ascent - placed
}

/// PingFang (the system's Chinese font) is a downloadable asset on recent macOS: its folder
/// name changes, so look for it.
fn pingfang() -> Option<PathBuf> {
    let assets = Path::new("/System/Library/AssetsV2");
    for dir in std::fs::read_dir(assets).ok()?.flatten() {
        if !dir
            .file_name()
            .to_string_lossy()
            .starts_with("com_apple_MobileAsset_Font")
        {
            continue;
        }
        for asset in std::fs::read_dir(dir.path()).ok()?.flatten() {
            let p = asset.path().join("AssetData").join("PingFang.ttc");
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// The CJK interface font: (file contents, face index).
pub(crate) fn find() -> Option<(Vec<u8>, u32)> {
    if let Some(bytes) = pingfang().and_then(|p| std::fs::read(p).ok())
        && let Some(i) = face_index(&bytes, "PingFang SC Regular")
    {
        return Some((bytes, i));
    }
    [
        "/System/Library/Fonts/Hiragino Sans GB.ttc",
        "/System/Library/Fonts/STHeiti Light.ttc",
        "C:\\Windows\\Fonts\\msyh.ttc",
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
    ]
    .iter()
    .find_map(|p| std::fs::read(p).ok())
    .map(|b| (b, 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offset_lines_up_baselines() {
        // SF: ascent .967, descent -.211, no gap. Hiragino Sans GB: .88, -.12, gap .5.
        let sf = VMetrics {
            ascent: 0.967,
            descent: -0.211,
            gap: 0.0,
        };
        let hiragino = VMetrics {
            ascent: 0.88,
            descent: -0.12,
            gap: 0.5,
        };
        let off = baseline_offset(sf, hiragino);
        assert!((off - 0.248).abs() < 0.001, "{off}");
        // Same metrics: no shift.
        assert!(baseline_offset(sf, sf).abs() < 1e-6);
    }

    #[test]
    fn reads_system_font_metrics_when_present() {
        let Ok(sf) = std::fs::read("/System/Library/Fonts/SFNSMono.ttf") else {
            return;
        };
        let m = vmetrics(&sf, 0).unwrap();
        assert!(m.ascent > 0.8 && m.descent < 0.0, "{m:?}");
        if let Ok(h) = std::fs::read("/System/Library/Fonts/Hiragino Sans GB.ttc") {
            assert_eq!(face_index(&h, "Hiragino Sans GB W3"), Some(0));
            let hm = vmetrics(&h, 0).unwrap();
            assert!((hm.gap - 0.5).abs() < 0.01, "{hm:?}");
        }
        // Garbage never panics.
        assert!(vmetrics(b"ttcf\0\0\0\0\xff\xff\xff\xff", 3).is_none());
        assert!(face_index(&[0u8; 3], "x").is_none());
    }
}
