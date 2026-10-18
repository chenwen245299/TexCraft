//! Timing of the SyncTeX highlight, the same in both directions: forward search flashes boxes in
//! the PDF, inverse search the source in the editor. Full strength for [`HOLD`] seconds, then a
//! fade over [`FADE`] seconds; frames are asked for only while it runs.

use std::time::Duration;

/// Seconds at full strength.
pub const HOLD: f32 = 1.0;
/// Seconds of the fade-out after that.
pub const FADE: f32 = 0.7;

/// Opacity `age` seconds after the jump: 1 while held, easing out to 0, then 0.
pub fn alpha(age: f32) -> f32 {
    if !age.is_finite() {
        return 0.0;
    }
    if age <= HOLD {
        return 1.0;
    }
    let t = (age - HOLD) / FADE;
    if t >= 1.0 {
        0.0
    } else {
        1.0 - t * t * (3.0 - 2.0 * t)
    }
}

/// When the next frame is needed: when the hold ends, every frame while fading, never after.
pub fn next_frame(age: f32) -> Option<Duration> {
    if !age.is_finite() || age >= HOLD + FADE {
        None
    } else if age < HOLD {
        Some(Duration::from_secs_f32(HOLD - age))
    } else {
        Some(Duration::from_millis(16))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn holds_then_fades_out() {
        assert_eq!(alpha(0.0), 1.0);
        assert_eq!(alpha(HOLD), 1.0);
        assert_eq!(alpha(-1.0), 1.0, "a clock step back keeps it lit");
        let mid = alpha(HOLD + FADE / 2.0);
        assert!((mid - 0.5).abs() < 1e-6, "{mid}");
        // Strictly decreasing through the fade.
        let mut last = 1.0;
        for k in 1..=20 {
            let a = alpha(HOLD + FADE * k as f32 / 20.0);
            assert!(a < last, "{k}: {a} after {last}");
            last = a;
        }
        assert_eq!(alpha(HOLD + FADE), 0.0);
        assert_eq!(alpha(10.0), 0.0);
        assert_eq!(alpha(f32::NAN), 0.0);
        // About 1–2 s in all.
        assert!((1.0..=2.0).contains(&(HOLD + FADE)));
    }

    #[test]
    fn frames_only_while_it_runs() {
        assert_eq!(next_frame(0.25), Some(Duration::from_secs_f32(HOLD - 0.25)));
        assert_eq!(next_frame(HOLD + 0.1), Some(Duration::from_millis(16)));
        assert_eq!(next_frame(HOLD + FADE), None);
        assert_eq!(next_frame(5.0), None);
        assert_eq!(next_frame(f32::INFINITY), None);
    }
}
