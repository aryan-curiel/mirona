//! Calls starting on this Mac (NGR-170): which call apps are using the microphone, and the
//! title of the calendar event happening now.
//!
//! - Mic use: Core Audio's process objects (macOS 14+) say which processes run audio input
//!   right now (`kAudioProcessPropertyIsRunningInput`), by bundle id. No permission needed.
//! - Calendar: `EventKit`, behind the calendar permission; only read when the user opts in.
//!
//! Other OSes: no detection (empty list, no event).

/// A known call app that is using the microphone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallApp {
    pub name: String,
    pub bundle_id: String,
}

/// Bundle id (or id prefix: helpers are `<app>.helper…`) → what we call the app.
// ponytail: a browser can't be told apart from Meet without reading its tabs, so any browser
// using the mic counts as a call; per-tab detection would need the Accessibility API (NGR-170).
const APPS: [(&str, &str); 26] = [
    ("us.zoom.xos", "Zoom"),
    ("com.microsoft.teams2", "Microsoft Teams"),
    ("com.microsoft.teams", "Microsoft Teams"),
    ("com.cisco.webexmeetingsapp", "Webex"),
    ("com.webex.meetingmanager", "Webex"),
    ("Cisco-Systems.Spark", "Webex"),
    ("com.tinyspeck.slackmacgap", "Slack"),
    ("com.apple.FaceTime", "FaceTime"),
    ("com.apple.avconferenced", "FaceTime"),
    ("com.hnc.Discord", "Discord"),
    ("com.google.Chrome", "Chrome (Google Meet?)"),
    ("com.apple.Safari", "Safari (Google Meet?)"),
    // Safari's (and other WebKit apps') capture runs in WebKit's GPU process.
    ("com.apple.WebKit", "Safari (Google Meet?)"),
    ("company.thebrowser.Browser", "Arc (Google Meet?)"),
    ("company.thebrowser.dia", "Dia (Google Meet?)"),
    ("com.microsoft.edgemac", "Edge (Google Meet?)"),
    ("org.mozilla.firefox", "Firefox (Google Meet?)"),
    (
        "org.mozilla.firefoxdeveloperedition",
        "Firefox (Google Meet?)",
    ),
    ("org.mozilla.nightly", "Firefox (Google Meet?)"),
    ("app.zen-browser.zen", "Zen (Google Meet?)"),
    ("org.chromium.Chromium", "Chromium (Google Meet?)"),
    ("com.operasoftware.OperaGX", "Opera GX (Google Meet?)"),
    ("ru.yandex.desktop.yandex-browser", "Yandex (Google Meet?)"),
    ("com.brave.Browser", "Brave (Google Meet?)"),
    ("com.operasoftware.Opera", "Opera (Google Meet?)"),
    ("com.vivaldi.Vivaldi", "Vivaldi (Google Meet?)"),
];

/// The call app a bundle id belongs to, if it's one we know.
pub fn app_name(bundle_id: &str) -> Option<&'static str> {
    APPS.iter()
        .find(|(id, _)| {
            bundle_id
                .strip_prefix(id)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
        })
        .map(|(_, name)| *name)
}

/// The tag whose name (`project:acme` → "acme") appears in a calendar title, any case; the
/// longest name wins so "Acme Labs" beats "Acme".
pub fn tag_in_title<'a>(title: &str, tags: impl IntoIterator<Item = &'a String>) -> Option<String> {
    let title = title.to_lowercase();
    let name = |t: &str| {
        t.rsplit(':')
            .next()
            .unwrap_or_default()
            .trim()
            .to_lowercase()
    };
    tags.into_iter()
        .filter(|t| !name(t).is_empty() && title.contains(&name(t)))
        .max_by_key(|t| name(t).len())
        .cloned()
}

/// Known call apps using the microphone right now, one entry per app, Mirona excluded.
pub fn calls_using_mic() -> Vec<CallApp> {
    #[cfg(target_os = "macos")]
    let procs = mac::processes_using_mic();
    #[cfg(not(target_os = "macos"))]
    let procs: Vec<(i32, String)> = vec![];
    let mut out: Vec<CallApp> = vec![];
    for (pid, bundle_id) in procs {
        if pid == std::process::id().cast_signed() {
            continue;
        }
        if let Some(name) = app_name(&bundle_id)
            && !out.iter().any(|c| c.name == name)
        {
            out.push(CallApp {
                name: name.into(),
                bundle_id,
            });
        }
    }
    out
}

/// How far ahead an event still counts: people join a few minutes early.
pub const EARLY_JOIN_SECS: f64 = 5.0 * 60.0;

/// The title of the (non all-day) calendar event a call starting now belongs to: one running
/// now or starting within [`EARLY_JOIN_SECS`]. `None` without calendar access or off macOS.
pub fn current_event_title() -> Option<String> {
    #[cfg(target_os = "macos")]
    return mac::current_event_title();
    #[cfg(not(target_os = "macos"))]
    None
}

/// Of `(start, title)` events (Unix seconds) overlapping now..now + [`EARLY_JOIN_SECS`], the one
/// whose start is closest to `now`: joining at 9:58 picks the 10:00 call over the 9:00 one still
/// running. Blank titles don't count.
pub fn pick_event(events: impl IntoIterator<Item = (f64, String)>, now: f64) -> Option<String> {
    events
        .into_iter()
        .filter(|(_, t)| !t.trim().is_empty())
        .min_by(|a, b| (a.0 - now).abs().total_cmp(&(b.0 - now).abs()))
        .map(|(_, t)| t)
}

/// Ask for calendar access (blocks until answered). `true` once granted.
pub fn request_calendar_permission() -> bool {
    #[cfg(target_os = "macos")]
    return mac::request_calendar();
    #[cfg(not(target_os = "macos"))]
    false
}

#[cfg(target_os = "macos")]
mod mac {
    use std::ffi::c_void;
    use std::ptr::NonNull;

    use objc2_core_audio::{
        AudioObjectGetPropertyData, AudioObjectGetPropertyDataSize, AudioObjectID,
        AudioObjectPropertyAddress, AudioObjectPropertySelector,
        kAudioHardwarePropertyProcessObjectList, kAudioObjectPropertyElementMain,
        kAudioObjectPropertyScopeGlobal, kAudioObjectSystemObject, kAudioProcessPropertyBundleID,
        kAudioProcessPropertyIsRunningInput, kAudioProcessPropertyPID,
    };
    use objc2_core_foundation::{CFRetained, CFString};
    use objc2_event_kit::{EKAuthorizationStatus, EKEntityType, EKEventStore};
    use objc2_foundation::NSDate;

    fn address(selector: AudioObjectPropertySelector) -> AudioObjectPropertyAddress {
        AudioObjectPropertyAddress {
            mSelector: selector,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain,
        }
    }

    /// Read a fixed-size property into `T`.
    fn get<T: Copy>(
        obj: AudioObjectID,
        selector: AudioObjectPropertySelector,
        zero: T,
    ) -> Option<T> {
        let mut out = zero;
        let mut size = u32::try_from(size_of::<T>()).ok()?;
        let mut addr = address(selector);
        let status = unsafe {
            AudioObjectGetPropertyData(
                obj,
                NonNull::from(&mut addr),
                0,
                std::ptr::null(),
                NonNull::from(&mut size),
                NonNull::from(&mut out).cast::<c_void>(),
            )
        };
        (status == 0).then_some(out)
    }

    /// (pid, bundle id) of every process with audio input running. Empty before macOS 14.
    pub fn processes_using_mic() -> Vec<(i32, String)> {
        #[allow(clippy::cast_sign_loss)] // kAudioObjectSystemObject is 1
        let system = kAudioObjectSystemObject as AudioObjectID;
        let mut addr = address(kAudioHardwarePropertyProcessObjectList);
        let mut size = 0u32;
        let status = unsafe {
            AudioObjectGetPropertyDataSize(
                system,
                NonNull::from(&mut addr),
                0,
                std::ptr::null(),
                NonNull::from(&mut size),
            )
        };
        if status != 0 || size == 0 {
            return vec![];
        }
        let mut ids = vec![0 as AudioObjectID; size as usize / size_of::<AudioObjectID>()];
        let status = unsafe {
            AudioObjectGetPropertyData(
                system,
                NonNull::from(&mut addr),
                0,
                std::ptr::null(),
                NonNull::from(&mut size),
                NonNull::new(ids.as_mut_ptr().cast::<c_void>()).unwrap(),
            )
        };
        if status != 0 {
            return vec![];
        }
        ids.truncate(size as usize / size_of::<AudioObjectID>());
        ids.into_iter()
            .filter(|&p| get(p, kAudioProcessPropertyIsRunningInput, 0u32).is_some_and(|r| r != 0))
            .filter_map(|p| {
                let pid = get(p, kAudioProcessPropertyPID, 0i32)?;
                let raw = get(
                    p,
                    kAudioProcessPropertyBundleID,
                    std::ptr::null::<CFString>(),
                )?;
                // The caller owns the returned CFString.
                let id = unsafe { CFRetained::from_raw(NonNull::new(raw.cast_mut())?) };
                Some((pid, id.to_string()))
            })
            .collect()
    }

    fn calendar_granted() -> bool {
        let status = unsafe { EKEventStore::authorizationStatusForEntityType(EKEntityType::Event) };
        status == EKAuthorizationStatus::FullAccess
    }

    pub fn request_calendar() -> bool {
        if calendar_granted() {
            return true;
        }
        let store = unsafe { EKEventStore::new() };
        let (tx, rx) = std::sync::mpsc::channel();
        let block = block2::RcBlock::new(move |granted: objc2::runtime::Bool, _err| {
            let _ = tx.send(granted.as_bool());
        });
        unsafe { store.requestFullAccessToEventsWithCompletion(block2::RcBlock::as_ptr(&block)) };
        rx.recv().unwrap_or(false)
    }

    pub fn current_event_title() -> Option<String> {
        if !calendar_granted() {
            return None;
        }
        let store = unsafe { EKEventStore::new() };
        let start = NSDate::now();
        let end = NSDate::dateWithTimeIntervalSinceNow(super::EARLY_JOIN_SECS);
        let events = unsafe {
            let p = store.predicateForEventsWithStartDate_endDate_calendars(&start, &end, None);
            store.eventsMatchingPredicate(&p)
        };
        let events = events
            .iter()
            .filter(|e| unsafe { !e.isAllDay() })
            .map(|e| unsafe { (e.startDate().timeIntervalSince1970(), e.title().to_string()) });
        super::pick_event(events, start.timeIntervalSince1970())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_ids_map_to_call_apps() {
        assert_eq!(app_name("us.zoom.xos"), Some("Zoom"));
        assert_eq!(app_name("com.microsoft.teams2"), Some("Microsoft Teams"));
        assert_eq!(
            app_name("com.google.Chrome.helper"),
            Some("Chrome (Google Meet?)")
        );
        assert_eq!(
            app_name("com.apple.WebKit.GPU"),
            Some("Safari (Google Meet?)")
        );
        // A prefix only counts at a dot: com.microsoft.teamsX is not Teams.
        assert_eq!(
            app_name("company.thebrowser.dia.helper"),
            Some("Dia (Google Meet?)")
        );
        assert_eq!(
            app_name("org.mozilla.nightly"),
            Some("Firefox (Google Meet?)")
        );
        assert_eq!(app_name("com.microsoft.teamsX"), None);
        assert_eq!(app_name("com.nagare.mirona"), None);
    }

    #[test]
    fn calendar_titles_match_tags() {
        let tags = [
            "project:Acme".to_string(),
            "project:Acme Labs".into(),
            "Northwind".into(),
        ];
        assert_eq!(
            tag_in_title("Weekly sync / ACME LABS", &tags),
            Some("project:Acme Labs".into())
        );
        assert_eq!(
            tag_in_title("northwind kickoff", &tags),
            Some("Northwind".into())
        );
        assert_eq!(tag_in_title("1:1 with Sam", &tags), None);
    }

    #[test]
    fn early_joins_pick_the_nearest_event() {
        let ev = |start: f64, t: &str| (start, t.to_string());
        let now = 10_000.0;
        // 9:58: the 10:00 call beats the 9:00 block still running.
        assert_eq!(
            pick_event(
                [ev(now - 3480.0, "Focus"), ev(now + 120.0, "Setr sync")],
                now
            ),
            Some("Setr sync".into())
        );
        // 10:03, running late: the 10:00 call beats the 10:30 one.
        assert_eq!(
            pick_event(
                [ev(now - 180.0, "Setr sync"), ev(now + 1620.0, "Later")],
                now
            ),
            Some("Setr sync".into())
        );
        assert_eq!(pick_event([ev(now, " ")], now), None);
        assert_eq!(pick_event([], now), None);
    }

    /// Prints what's using the mic and the calendar event now. Run with a call open:
    /// `cargo test -p mirona-capture calls_on_this_mac -- --ignored --nocapture`.
    #[test]
    #[ignore = "real hardware"]
    fn calls_on_this_mac() {
        #[cfg(target_os = "macos")]
        println!("processes using the mic: {:?}", mac::processes_using_mic());
        println!("call apps: {:?}", calls_using_mic());
        println!("event now: {:?}", current_event_title());
    }
}
