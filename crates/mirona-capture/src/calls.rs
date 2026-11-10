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

/// The title of the (non all-day) calendar event happening now; the latest to start if several.
/// `None` without calendar access or off macOS.
pub fn current_event_title() -> Option<String> {
    #[cfg(target_os = "macos")]
    return mac::current_event_title();
    #[cfg(not(target_os = "macos"))]
    None
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
        let (start, end) = (NSDate::now(), NSDate::dateWithTimeIntervalSinceNow(1.0));
        let events = unsafe {
            let p = store.predicateForEventsWithStartDate_endDate_calendars(&start, &end, None);
            store.eventsMatchingPredicate(&p)
        };
        events
            .iter()
            .filter(|e| unsafe { !e.isAllDay() })
            .max_by(|a, b| unsafe {
                let (a, b) = (a.startDate(), b.startDate());
                a.timeIntervalSince1970()
                    .total_cmp(&b.timeIntervalSince1970())
            })
            .map(|e| unsafe { e.title() }.to_string())
            .filter(|t| !t.trim().is_empty())
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
