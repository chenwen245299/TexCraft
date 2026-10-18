//! macOS services that winit does not expose: window-button placement and the system spell
//! checker.
//!
//! TexCraft draws its own toolbar under a transparent title bar. AppKit keeps the window buttons
//! (close, minimise, zoom) at the top of a 28 pt title bar, but the target app's toolbar is taller
//! with the buttons vertically centred, so [`place_traffic_lights`] moves them, the way Electron's
//! `trafficLightPosition` does. This is the only crate with `unsafe` code (Objective-C calls);
//! every block carries a `SAFETY:` comment.

use raw_window_handle::HasWindowHandle;

/// Centres the window buttons vertically in a bar `height` points tall at the top of the window,
/// the close button's left edge `left` points from the window edge. Cheap to call every frame:
/// it only moves views that are not already in place (AppKit re-lays the title bar out after
/// resizes). Returns whether the buttons are in place.
pub fn place_traffic_lights(window: &impl HasWindowHandle, height: f64, left: f64) -> bool {
    #[cfg(target_os = "macos")]
    {
        imp::place(window, height, left)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (window, height, left);
        false
    }
}

/// Sets the window buttons' opacity (0 hidden … 1 shown), e.g. to show them again once they are
/// back in place after full screen. Cheap to call every frame: it only touches buttons whose
/// opacity differs. Returns whether the window's buttons were found.
pub fn set_traffic_lights_alpha(window: &impl HasWindowHandle, alpha: f64) -> bool {
    #[cfg(target_os = "macos")]
    {
        imp::set_alpha(window, alpha)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (window, alpha);
        false
    }
}

/// Makes the window buttons transparent the moment the window starts leaving full screen (AppKit's
/// "will exit full screen" notification), so they do not show at AppKit's default place during
/// the exit animation, when the app draws no frames; the app shows them again afterwards with
/// [`set_traffic_lights_alpha`]. Installs the observer once; cheap to call every frame. Returns
/// whether it is installed.
pub fn hide_traffic_lights_on_full_screen_exit(window: &impl HasWindowHandle) -> bool {
    #[cfg(target_os = "macos")]
    {
        imp::hide_on_exit(window)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = window;
        false
    }
}

/// Whether AppKit has the window in full screen (its style mask), or `None` off macOS. Unlike
/// winit's state, which follows AppKit's "did enter / did exit" notifications, this changes
/// when the transition begins.
pub fn is_full_screen(window: &impl HasWindowHandle) -> Option<bool> {
    #[cfg(target_os = "macos")]
    {
        imp::is_full_screen(window)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = window;
        None
    }
}

/// Enters or leaves full screen the way the green window button does (`toggleFullScreen:`).
/// Returns whether the window was found.
pub fn toggle_full_screen(window: &impl HasWindowHandle) -> bool {
    #[cfg(target_os = "macos")]
    {
        imp::toggle_full_screen(window)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = window;
        false
    }
}

/// Like [`place_traffic_lights`] for a window found by its title (egui's secondary viewports do
/// not hand out window handles). With `close_only`, the minimise and zoom buttons are hidden, as
/// in a preferences window. Returns whether the window was found and its buttons are in place.
pub fn place_traffic_lights_titled(title: &str, height: f64, left: f64, close_only: bool) -> bool {
    #[cfg(target_os = "macos")]
    {
        imp::place_titled(title, height, left, close_only)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (title, height, left, close_only);
        false
    }
}

/// Puts the system's translucent window material (vibrancy) behind the content of the window
/// titled `title`; the content must draw on a transparent background. Cheap to call every frame
/// (it installs the material once). Returns whether the material is in place.
pub fn set_vibrancy_titled(title: &str) -> bool {
    #[cfg(target_os = "macos")]
    {
        imp::vibrancy_titled(title)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = title;
        false
    }
}

/// Whether the system appearance (System Settings → Appearance) is dark, asked of AppKit. The
/// app's windows can't tell while they carry an appearance of their own (a Light or Dark choice
/// in TexCraft): they hear of no system change then. `None` off macOS or off the main thread.
pub fn system_dark() -> Option<bool> {
    #[cfg(target_os = "macos")]
    {
        imp::system_dark()
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

/// The app's windows as (title, visible, [x, y, width, height] in screen points), for checks
/// and the control channel. Empty off macOS.
pub fn windows() -> Vec<(String, bool, [f64; 4])> {
    #[cfg(target_os = "macos")]
    {
        imp::windows()
    }
    #[cfg(not(target_os = "macos"))]
    {
        Vec::new()
    }
}

/// The window buttons' frames as `[x, y_from_top, width, height]` in points (for checks and
/// the control channel). Empty off macOS.
pub fn traffic_light_frames(window: &impl HasWindowHandle) -> Vec<[f64; 4]> {
    #[cfg(target_os = "macos")]
    {
        imp::frames(window)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = window;
        Vec::new()
    }
}

/// Asks the system spell checker whether `word` is spelled correctly (in the user's languages,
/// honouring learned words). `None` off macOS or off the main thread.
pub fn spelling_ok(word: &str) -> Option<bool> {
    #[cfg(target_os = "macos")]
    {
        spell::ok(word)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = word;
        None
    }
}

/// Spelling suggestions for `word` from the system spell checker (empty off macOS).
pub fn spelling_guesses(word: &str) -> Vec<String> {
    #[cfg(target_os = "macos")]
    {
        spell::guesses(word)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = word;
        Vec::new()
    }
}

/// Adds `word` to the user's spelling dictionary (macOS "Learn Spelling").
pub fn learn_word(word: &str) {
    #[cfg(target_os = "macos")]
    spell::learn(word);
    #[cfg(not(target_os = "macos"))]
    let _ = word;
}

/// The user's preferred languages (System Settings → General → Language & Region), most
/// preferred first, as BCP 47 tags (`zh-Hans-CN`, `en-GB`). Empty off macOS.
pub fn preferred_languages() -> Vec<String> {
    #[cfg(target_os = "macos")]
    {
        locale::preferred_languages()
    }
    #[cfg(not(target_os = "macos"))]
    {
        Vec::new()
    }
}

/// Moves `path` (a file or folder) to the Trash, where Finder can put it back.
pub fn trash(path: &std::path::Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        files::trash(path)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        Err("the Trash is a macOS service".into())
    }
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod locale {
    use objc2_foundation::NSLocale;

    pub(crate) fn preferred_languages() -> Vec<String> {
        // SAFETY: a Foundation class method without arguments that returns a retained array of
        // strings; NSLocale may be used from any thread.
        let list = unsafe { NSLocale::preferredLanguages() };
        list.iter().map(|s| s.to_string()).collect()
    }
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod files {
    use objc2_foundation::{NSFileManager, NSString, NSURL};

    pub(crate) fn trash(path: &std::path::Path) -> Result<(), String> {
        let path = NSString::from_str(&path.to_string_lossy());
        // SAFETY: Foundation calls on valid, retained objects; `fileURLWithPath:` accepts any
        // string, and NSFileManager's shared instance may be used from any thread.
        let result = unsafe {
            let url = NSURL::fileURLWithPath(&path);
            NSFileManager::defaultManager().trashItemAtURL_resultingItemURL_error(&url, None)
        };
        result.map_err(|e| e.localizedDescription().to_string())
    }
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod spell {
    use objc2_app_kit::NSSpellChecker;
    use objc2_foundation::{MainThreadMarker, NSRange, NSString};

    fn checker() -> Option<objc2::rc::Retained<NSSpellChecker>> {
        MainThreadMarker::new()?;
        // SAFETY: the shared checker is created and used on the main thread (checked above).
        Some(unsafe { NSSpellChecker::sharedSpellChecker() })
    }

    pub fn ok(word: &str) -> Option<bool> {
        let checker = checker()?;
        let s = NSString::from_str(word);
        // English explicitly: on a lone word, automatic language detection can pick a language
        // in which a misspelling happens to be valid.
        let en = NSString::from_str("en");
        // SAFETY: valid NSStrings, main thread, null word-count pointer (allowed); returns the
        // range of the first misspelling.
        let range: NSRange = unsafe {
            checker.checkSpellingOfString_startingAt_language_wrap_inSpellDocumentWithTag_wordCount(
                &s,
                0,
                Some(&en),
                false,
                0,
                std::ptr::null_mut(),
            )
        };
        Some(range.length == 0)
    }

    pub fn guesses(word: &str) -> Vec<String> {
        let Some(checker) = checker() else {
            return Vec::new();
        };
        let s = NSString::from_str(word);
        let range = NSRange::new(0, s.length());
        let en = NSString::from_str("en");
        // SAFETY: valid strings and a range within the word, main thread.
        let list = unsafe {
            checker.guessesForWordRange_inString_language_inSpellDocumentWithTag(
                range,
                &s,
                Some(&en),
                0,
            )
        };
        list.map(|a| a.iter().take(6).map(|g| g.to_string()).collect())
            .unwrap_or_default()
    }

    pub fn learn(word: &str) {
        if let Some(checker) = checker() {
            let s = NSString::from_str(word);
            // SAFETY: valid string, main thread.
            unsafe { checker.learnWord(&s) };
        }
    }
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod imp {
    use objc2_app_kit::{NSApplication, NSView, NSWindow, NSWindowButton};
    use objc2_foundation::{MainThreadMarker, NSPoint, NSRect};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    pub fn system_dark() -> Option<bool> {
        use objc2_app_kit::{NSAppearanceNameAqua, NSAppearanceNameDarkAqua};
        use objc2_foundation::NSCopying;
        let mtm = MainThreadMarker::new()?;
        // The application's own appearance is never set, so its effective one is the system's.
        let appearance = NSApplication::sharedApplication(mtm).effectiveAppearance();
        // SAFETY: AppKit's constant appearance names, read on the main thread.
        let (aqua, dark) = unsafe { (NSAppearanceNameAqua, NSAppearanceNameDarkAqua) };
        let names = objc2_foundation::NSArray::from_vec(vec![aqua.copy(), dark.copy()]);
        let best = appearance.bestMatchFromAppearancesWithNames(&names)?;
        Some(&*best == dark)
    }

    pub fn frames(window: &impl HasWindowHandle) -> Vec<[f64; 4]> {
        if MainThreadMarker::new().is_none() {
            return Vec::new();
        }
        let Ok(handle) = window.window_handle() else {
            return Vec::new();
        };
        let RawWindowHandle::AppKit(h) = handle.as_raw() else {
            return Vec::new();
        };
        // SAFETY: as in `place`: the live window's content view, on the main thread.
        let view: &NSView = unsafe { h.ns_view.cast::<NSView>().as_ref() };
        let Some(win) = view.window() else {
            return Vec::new();
        };
        let height = win.frame().size.height;
        [
            NSWindowButton::NSWindowCloseButton,
            NSWindowButton::NSWindowMiniaturizeButton,
            NSWindowButton::NSWindowZoomButton,
        ]
        .iter()
        .filter_map(|&k| win.standardWindowButton(k))
        .map(|b| {
            // Convert to window coordinates, then measure from the top.
            let r = b.convertRect_toView(b.bounds(), None);
            [
                r.origin.x,
                height - r.origin.y - r.size.height,
                r.size.width,
                r.size.height,
            ]
        })
        .collect()
    }

    fn close_enough(a: f64, b: f64) -> bool {
        (a - b).abs() < 0.5
    }

    pub fn place(window: &impl HasWindowHandle, height: f64, left: f64) -> bool {
        if MainThreadMarker::new().is_none() || !(height.is_finite() && height > 0.0) {
            return false;
        }
        let Ok(handle) = window.window_handle() else {
            return false;
        };
        let RawWindowHandle::AppKit(h) = handle.as_raw() else {
            return false;
        };
        // SAFETY: eframe hands out the AppKit handle of its live window; `ns_view` is that
        // window's content view, valid while `window` is borrowed, and we are on the main thread
        // (checked above) as AppKit requires.
        let view: &NSView = unsafe { h.ns_view.cast::<NSView>().as_ref() };
        let Some(win) = view.window() else {
            return false;
        };
        place_on(&win, height, left, false)
    }

    fn ns_window(window: &impl HasWindowHandle) -> Option<objc2::rc::Retained<NSWindow>> {
        MainThreadMarker::new()?;
        let handle = window.window_handle().ok()?;
        let RawWindowHandle::AppKit(h) = handle.as_raw() else {
            return None;
        };
        // SAFETY: as in `place`: the live window's content view, on the main thread.
        let view: &NSView = unsafe { h.ns_view.cast::<NSView>().as_ref() };
        view.window()
    }

    pub fn is_full_screen(window: &impl HasWindowHandle) -> Option<bool> {
        let win = ns_window(window)?;
        Some(
            win.styleMask()
                .contains(objc2_app_kit::NSWindowStyleMask::FullScreen),
        )
    }

    pub fn toggle_full_screen(window: &impl HasWindowHandle) -> bool {
        let Some(win) = ns_window(window) else {
            return false;
        };
        win.toggleFullScreen(None);
        true
    }

    pub fn set_alpha(window: &impl HasWindowHandle, alpha: f64) -> bool {
        if !alpha.is_finite() {
            return false;
        }
        let Some(win) = ns_window(window) else {
            return false;
        };
        buttons_alpha(&win, alpha)
    }

    /// Whether the exit observer is installed (the app has one main window).
    static EXIT_OBSERVER: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

    pub fn hide_on_exit(window: &impl HasWindowHandle) -> bool {
        use std::sync::atomic::Ordering;
        if EXIT_OBSERVER.load(Ordering::Relaxed) {
            return true;
        }
        let Some(win) = ns_window(window) else {
            return false;
        };
        let weak = objc2::rc::Weak::from_retained(&win);
        let block = block2::RcBlock::new(
            move |_: std::ptr::NonNull<objc2_foundation::NSNotification>| {
                if MainThreadMarker::new().is_some()
                    && let Some(win) = weak.load()
                {
                    buttons_alpha(&win, 0.0);
                }
            },
        );
        let object: &objc2::runtime::AnyObject = win.as_ref();
        // SAFETY: the process's default notification centre; AppKit's notification-name
        // constant, a valid static NSString. The observer block only captures a weak reference
        // and runs on the posting (main) thread, which it checks; the window is the sender.
        let token = unsafe {
            let center = objc2_foundation::NSNotificationCenter::defaultCenter();
            center.addObserverForName_object_queue_usingBlock(
                Some(objc2_app_kit::NSWindowWillExitFullScreenNotification),
                Some(object),
                None,
                &block,
            )
        };
        // Observe for the rest of the app's life (the main window lives as long).
        std::mem::forget(token);
        EXIT_OBSERVER.store(true, Ordering::Relaxed);
        true
    }

    fn buttons_alpha(win: &NSWindow, alpha: f64) -> bool {
        let alpha = alpha.clamp(0.0, 1.0);
        let mut found = false;
        for k in [
            NSWindowButton::NSWindowCloseButton,
            NSWindowButton::NSWindowMiniaturizeButton,
            NSWindowButton::NSWindowZoomButton,
        ] {
            let Some(b) = win.standardWindowButton(k) else {
                continue;
            };
            found = true;
            // SAFETY: reading and setting the opacity of a standard button of our own window,
            // on the main thread (every caller checks it: `ns_window`, the exit observer).
            unsafe {
                if (b.alphaValue() - alpha).abs() > 0.004 {
                    b.setAlphaValue(alpha);
                }
            }
        }
        found
    }

    pub fn windows() -> Vec<(String, bool, [f64; 4])> {
        let Some(mtm) = MainThreadMarker::new() else {
            return Vec::new();
        };
        NSApplication::sharedApplication(mtm)
            .windows()
            .iter()
            .map(|w| {
                let f = w.frame();
                (
                    w.title().to_string(),
                    w.isVisible(),
                    [f.origin.x, f.origin.y, f.size.width, f.size.height],
                )
            })
            .collect()
    }

    pub fn vibrancy_titled(title: &str) -> bool {
        use objc2::ClassType;
        use objc2_app_kit::{
            NSAutoresizingMaskOptions, NSColor, NSVisualEffectBlendingMode, NSVisualEffectMaterial,
            NSVisualEffectState, NSVisualEffectView, NSWindowOrderingMode,
        };
        use objc2_foundation::NSObjectProtocol;
        let Some(mtm) = MainThreadMarker::new() else {
            return false;
        };
        let app = NSApplication::sharedApplication(mtm);
        let windows = app.windows();
        let Some(win) = windows.iter().find(|w| w.title().to_string() == title) else {
            return false;
        };
        let Some(content) = win.contentView() else {
            return false;
        };
        // SAFETY: view-hierarchy getters on the main thread (checked above).
        let Some(frame_view) = (unsafe { content.superview() }) else {
            return false;
        };
        // SAFETY: as above.
        let subviews = unsafe { frame_view.subviews() };
        if subviews
            .iter()
            .any(|v| v.isKindOfClass(NSVisualEffectView::class()))
        {
            return true;
        }
        // SAFETY: a new view created and configured on the main thread, then added to our own
        // window's frame view below the content view, which keeps drawing on top.
        unsafe {
            let effect = NSVisualEffectView::initWithFrame(mtm.alloc(), content.frame());
            effect.setMaterial(NSVisualEffectMaterial::UnderWindowBackground);
            effect.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
            effect.setState(NSVisualEffectState::FollowsWindowActiveState);
            effect.setAutoresizingMask(
                NSAutoresizingMaskOptions::NSViewWidthSizable
                    | NSAutoresizingMaskOptions::NSViewHeightSizable,
            );
            frame_view.addSubview_positioned_relativeTo(
                &effect,
                NSWindowOrderingMode::NSWindowBelow,
                Some(&content),
            );
        }
        win.setOpaque(false);
        // SAFETY: a class method returning a shared colour, on the main thread.
        let clear = unsafe { NSColor::clearColor() };
        win.setBackgroundColor(Some(&clear));
        true
    }

    pub fn place_titled(title: &str, height: f64, left: f64, close_only: bool) -> bool {
        let Some(mtm) = MainThreadMarker::new() else {
            return false;
        };
        if !(height.is_finite() && height > 0.0) {
            return false;
        }
        let app = NSApplication::sharedApplication(mtm);
        let windows = app.windows();
        let Some(win) = windows.iter().find(|w| w.title().to_string() == title) else {
            return false;
        };
        place_on(win, height, left, close_only)
    }

    fn place_on(win: &NSWindow, height: f64, left: f64, close_only: bool) -> bool {
        let kinds = [
            NSWindowButton::NSWindowCloseButton,
            NSWindowButton::NSWindowMiniaturizeButton,
            NSWindowButton::NSWindowZoomButton,
        ];
        let buttons: Vec<_> = kinds.iter().map(|&k| win.standardWindowButton(k)).collect();
        let Some(Some(close)) = buttons.first() else {
            return false;
        };
        // The buttons sit in the title-bar view inside the title-bar container: grow the
        // container to the toolbar height so the buttons can be centred in it.
        // SAFETY: plain view-hierarchy getters on the main thread.
        let Some(titlebar) = (unsafe { close.superview() }) else {
            return false;
        };
        // SAFETY: as above.
        let Some(container) = (unsafe { titlebar.superview() }) else {
            return false;
        };
        let win_frame = win.frame();
        let mut cf: NSRect = container.frame();
        if !close_enough(cf.size.height, height) {
            cf.size.height = height;
            cf.origin.y = win_frame.size.height - height;
            // SAFETY: resizing a view of our own window on the main thread.
            unsafe { container.setFrame(cf) };
        }
        let mut tf: NSRect = titlebar.frame();
        if !close_enough(tf.size.height, height) || !close_enough(tf.origin.y, 0.0) {
            tf.size.height = height;
            tf.origin.y = 0.0;
            // SAFETY: as above.
            unsafe { titlebar.setFrame(tf) };
        }
        let first_x = close.frame().origin.x;
        let step = buttons
            .get(1)
            .and_then(|b| b.as_ref())
            .map_or(20.0, |m| (m.frame().origin.x - first_x).max(16.0));
        for (i, b) in buttons.iter().enumerate() {
            let Some(b) = b else { continue };
            if close_only && i > 0 {
                b.setHidden(true);
                continue;
            }
            let f = b.frame();
            let target = NSPoint::new(
                left + step * i as f64,
                ((height - f.size.height) / 2.0).round(),
            );
            if !close_enough(f.origin.x, target.x) || !close_enough(f.origin.y, target.y) {
                // SAFETY: moving a standard button of our own window on the main thread.
                unsafe { b.setFrameOrigin(target) };
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    /// AppKit is asked on the main thread only: elsewhere no answer, and no crash.
    #[test]
    fn system_dark_off_the_main_thread_is_none() {
        let answer = std::thread::spawn(super::system_dark).join();
        assert!(matches!(answer, Ok(None)));
    }
}
