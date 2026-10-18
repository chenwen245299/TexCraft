//! Local times for display (the UI crate has no date library): the UTC offset and locale are
//! asked of the system once.

use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

/// The local UTC offset in seconds (0 when unknown).
fn utc_offset() -> i64 {
    static OFFSET: OnceLock<i64> = OnceLock::new();
    *OFFSET.get_or_init(|| {
        #[cfg(unix)]
        {
            let out = std::process::Command::new("date").arg("+%z").output().ok();
            let s = out
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .unwrap_or_default();
            if s.len() == 5 {
                let sign = if s.starts_with('-') { -1 } else { 1 };
                let h: i64 = s.get(1..3).and_then(|x| x.parse().ok()).unwrap_or(0);
                let m: i64 = s.get(3..5).and_then(|x| x.parse().ok()).unwrap_or(0);
                return sign * (h * 3600 + m * 60);
            }
        }
        0
    })
}

/// Whether the system formats dates the East Asian way (年月日).
fn cjk_locale() -> bool {
    static CJK: OnceLock<bool> = OnceLock::new();
    *CJK.get_or_init(|| {
        #[cfg(target_os = "macos")]
        {
            let out = std::process::Command::new("defaults")
                .args(["read", "-g", "AppleLocale"])
                .output()
                .ok();
            let s = out
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .unwrap_or_default();
            s.starts_with("zh") || s.starts_with("ja")
        }
        #[cfg(not(target_os = "macos"))]
        {
            std::env::var("LANG").is_ok_and(|l| l.starts_with("zh") || l.starts_with("ja"))
        }
    })
}

fn local_secs(t: SystemTime) -> i64 {
    t.duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
        + utc_offset()
}

/// `HH:MM:SS` in local time.
pub(crate) fn clock(t: SystemTime) -> String {
    let day = local_secs(t).rem_euclid(86_400);
    format!("{:02}:{:02}:{:02}", day / 3600, (day % 3600) / 60, day % 60)
}

/// (year, month, day) of a day count since 1970-01-01 (proleptic Gregorian).
fn civil(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// Date and time in local time, the way the system's locale writes it (roughly); always the
/// 年月日 way when the interface is in Chinese.
pub(crate) fn date_time(t: SystemTime) -> String {
    let chinese = texcraft_i18n::language() == texcraft_i18n::Lang::ZhHans;
    format_date_time(local_secs(t), chinese || cjk_locale())
}

fn format_date_time(secs: i64, cjk: bool) -> String {
    let (y, m, d) = civil(secs.div_euclid(86_400));
    let day = secs.rem_euclid(86_400);
    let time = format!("{:02}:{:02}:{:02}", day / 3600, (day % 3600) / 60, day % 60);
    if cjk {
        format!("{y}年{m}月{d}日 {time}")
    } else {
        const MONTHS: [&str; 12] = [
            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
        ];
        let name = usize::try_from(m - 1)
            .ok()
            .and_then(|i| MONTHS.get(i))
            .unwrap_or(&"");
        format!("{d} {name} {y} at {time}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_dates() {
        // 2026-09-24 16:21:54 UTC.
        let secs = 1_790_266_914;
        assert_eq!(format_date_time(secs, true), "2026年9月24日 16:21:54");
        assert_eq!(format_date_time(secs, false), "24 Sep 2026 at 16:21:54");
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(-1), (1969, 12, 31));
        assert_eq!(civil(19_782), (2024, 2, 29));
    }

    #[test]
    fn chinese_interface_writes_dates_the_chinese_way() {
        let _zh = texcraft_i18n::scoped(texcraft_i18n::Lang::ZhHans);
        let t = UNIX_EPOCH + std::time::Duration::from_secs(1_790_266_914);
        let s = date_time(t);
        assert!(
            s.contains('年') && s.contains('月') && s.contains('日'),
            "{s}"
        );
        assert!(!s.contains(" at "), "{s}");
    }
}
