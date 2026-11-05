//! macOS devices and permissions (NGR-137).
//!
//! - Mic: the default input device through cpal (Core Audio).
//! - System audio: a Core Audio process tap on the default output device (all processes, the
//!   audio still plays), which is what cpal does when you record from an output device
//!   (macOS 14.6+). Chosen over `ScreenCaptureKit`: one dependency for both sources, no screen
//!   recording permission or purple screen-capture indicator, only the narrower "System Audio
//!   Recording Only" permission. Meetily (MIT) also records the system mix with a process tap.
//! - Device changes: input streams don't follow the default device, and the tap is bound to
//!   the output device it was made for, so once a second we compare the defaults with what
//!   we're recording and rebuild the stream when they differ or the stream died.

use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{DeviceId, ErrorKind, SampleFormat};

use crate::live::{Msg, Pipeline, RecorderConfig};
use crate::{Event, Permission, Source, downmix};

const MIC_SETTINGS: &str = "System Settings › Privacy & Security › Microphone";
const SYSTEM_SETTINGS: &str = "System Settings › Privacy & Security › Screen & System Audio Recording › System Audio Recording Only";

/// One device stream and what it was built from.
struct Input {
    source: Source,
    stream: Option<cpal::Stream>,
    device: Option<DeviceId>,
    /// Sample rate the stream was built with. Bluetooth headsets keep their name but switch
    /// rate (and break existing taps) when their mic opens, so a change means rebuild.
    rate: Option<u32>,
    last_error: Option<String>,
}

impl Input {
    /// (Re)build the stream if the default device changed or the stream died.
    fn check(&mut self, host: &cpal::Host, tx: &Sender<Msg>, events: &Sender<Event>) {
        let dev = match self.source {
            Source::Mic => host.default_input_device(),
            _ => host.default_output_device(),
        };
        let id = dev.as_ref().and_then(|d| d.id().ok());
        let rate = dev.as_ref().and_then(|d| current_rate(d, self.source));
        if self.stream.is_some() && id == self.device && rate == self.rate {
            return;
        }
        self.stream = None; // drop first: destroys the old tap before making a new one
        let res = match &dev {
            Some(d) => build(d, self.source, tx.clone()),
            None => Err(match self.source {
                Source::Mic => "no microphone found".to_owned(),
                _ => "no audio output device found".to_owned(),
            }),
        };
        match res {
            Ok(stream) => {
                self.stream = Some(stream);
                self.device = id;
                self.rate = rate;
                self.last_error = None;
                let name = dev.map(|d| d.to_string()).unwrap_or_default();
                let _ = events.send(Event::Device {
                    source: self.source,
                    name,
                });
            }
            // Report each distinct failure once, then keep retrying quietly.
            Err(message) if self.last_error.as_ref() != Some(&message) => {
                self.last_error = Some(message.clone());
                let _ = events.send(Event::Error {
                    source: Some(self.source),
                    message,
                });
            }
            Err(_) => {}
        }
    }
}

fn current_rate(dev: &cpal::Device, source: Source) -> Option<u32> {
    match source {
        Source::Mic => dev.default_input_config(),
        _ => dev.default_output_config(),
    }
    .ok()
    .map(|c| c.sample_rate())
}

fn build(dev: &cpal::Device, source: Source, tx: Sender<Msg>) -> Result<cpal::Stream, String> {
    let what = if source == Source::Mic {
        "microphone"
    } else {
        "system audio"
    };
    let explain = |e: cpal::Error| match e.kind() {
        ErrorKind::PermissionDenied if source == Source::Mic => {
            format!("microphone access denied: allow it in {MIC_SETTINGS}")
        }
        ErrorKind::PermissionDenied => {
            format!("system audio access denied: allow it in {SYSTEM_SETTINGS}")
        }
        _ => format!("{what} ({dev}): {e}"),
    };
    let cfg = match source {
        Source::Mic => dev.default_input_config(),
        _ => dev.default_output_config(),
    }
    .map_err(explain)?;
    if cfg.sample_format() != SampleFormat::F32 {
        return Err(format!(
            "{what} ({dev}): unsupported format {}",
            cfg.sample_format()
        ));
    }
    let (rate, channels) = (cfg.sample_rate(), usize::from(cfg.channels()));
    let lost = tx.clone();
    let stream = dev
        .build_input_stream::<f32, _, _>(
            cfg.into(),
            // ponytail: allocates a Vec per callback (~10 ms of audio); a lock-free ring
            // buffer if the audio thread ever glitches.
            move |data, _| {
                let _ = tx.send(Msg::Audio {
                    source,
                    rate,
                    mono: downmix(data, channels),
                });
            },
            move |e| {
                if !matches!(
                    e.kind(),
                    ErrorKind::Xrun | ErrorKind::RealtimeDenied | ErrorKind::DeviceChanged
                ) {
                    let _ = lost.send(Msg::Lost {
                        source,
                        message: e.to_string(),
                    });
                }
            },
            None,
        )
        .map_err(explain)?;
    stream.play().map_err(explain)?;
    Ok(stream)
}

/// The audio thread: owns the device streams (cpal streams stay on the thread that made them).
pub(crate) fn run(
    cfg: &RecorderConfig,
    mut pipeline: Pipeline,
    rx: &Receiver<Msg>,
    tx: &Sender<Msg>,
    events: &Sender<Event>,
) -> Result<(), String> {
    let host = cpal::default_host();
    let mut inputs: Vec<Input> = [(cfg.mic, Source::Mic), (cfg.system, Source::System)]
        .into_iter()
        .filter(|(on, _)| *on)
        .map(|(_, source)| Input {
            source,
            stream: None,
            device: None,
            rate: None,
            last_error: None,
        })
        .collect();
    let start = Instant::now();
    let (mut last_tick, mut last_check) = (start, start);
    for i in &mut inputs {
        i.check(&host, tx, events);
    }
    loop {
        match rx.recv_timeout(Duration::from_millis(20)) {
            Ok(Msg::Audio { source, rate, mono }) => pipeline.push(source, rate, &mono),
            Ok(Msg::Lost { source, message }) => {
                if let Some(i) = inputs.iter_mut().find(|i| i.source == source) {
                    i.stream = None; // rebuilt at the next check
                }
                let _ = events.send(Event::Error {
                    source: Some(source),
                    message,
                });
            }
            Ok(Msg::Stop) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {}
        }
        if last_tick.elapsed() >= Duration::from_millis(100) {
            last_tick = Instant::now();
            pipeline.catch_up(start.elapsed());
            let (mic, system) = pipeline.levels();
            let _ = events.send(Event::Levels { mic, system });
        }
        if last_check.elapsed() >= Duration::from_secs(1) {
            last_check = Instant::now();
            for i in &mut inputs {
                i.check(&host, tx, events);
            }
        }
    }
    drop(inputs); // stop the devices
    while let Ok(Msg::Audio { source, rate, mono }) = rx.try_recv() {
        pipeline.push(source, rate, &mono); // audio still queued at stop
    }
    pipeline.finish()
}

/// Refuse to start without permission: an unanswered prompt blocks the device open (and so
/// `stop`) until the user answers, and a denied source records silence without an error.
pub(crate) fn check_permissions(cfg: &RecorderConfig) -> Result<(), String> {
    let mic = if cfg.mic {
        mic_permission()
    } else {
        Permission::Granted
    };
    // An unreadable TCC answer (private API gone) must not block recording.
    let system = match (cfg.system, tcc::preflight()) {
        (true, Some(0) | None) | (false, _) => Permission::Granted,
        (true, Some(1)) => Permission::Denied,
        (true, Some(_)) => Permission::NotDetermined,
    };
    match (mic, system) {
        (Permission::Denied, _) => Err(format!(
            "microphone access denied: allow it in {MIC_SETTINGS}"
        )),
        (_, Permission::Denied) => Err(format!(
            "system audio access denied: allow it in {SYSTEM_SETTINGS}"
        )),
        (Permission::NotDetermined, _) => {
            Err("microphone permission not asked yet: call request_mic_permission() first".into())
        }
        (_, Permission::NotDetermined) => Err(
            "system audio permission not asked yet: call request_system_audio_permission() first"
                .into(),
        ),
        _ => Ok(()),
    }
}

pub(crate) fn mic_permission() -> Permission {
    use objc2_av_foundation::{AVAuthorizationStatus, AVCaptureDevice, AVMediaTypeAudio};
    let Some(audio) = (unsafe { AVMediaTypeAudio }) else {
        return Permission::NotDetermined;
    };
    match unsafe { AVCaptureDevice::authorizationStatusForMediaType(audio) } {
        AVAuthorizationStatus::Authorized => Permission::Granted,
        AVAuthorizationStatus::NotDetermined => Permission::NotDetermined,
        _ => Permission::Denied, // denied, or restricted by MDM/parental controls
    }
}

pub(crate) fn request_mic_permission() -> bool {
    use objc2_av_foundation::{AVCaptureDevice, AVMediaTypeAudio};
    let Some(audio) = (unsafe { AVMediaTypeAudio }) else {
        return false;
    };
    let (tx, rx) = std::sync::mpsc::channel();
    let block = block2::RcBlock::new(move |granted: objc2::runtime::Bool| {
        let _ = tx.send(granted.as_bool());
    });
    unsafe { AVCaptureDevice::requestAccessForMediaType_completionHandler(audio, &block) };
    rx.recv().unwrap_or(false)
}

/// macOS has no public API to check system-audio (process tap) access: the tap just records
/// silence when denied. Like `AudioCap` (github.com/insidegui/AudioCap), we ask TCC directly
/// through its private framework, loaded at run time so a missing symbol degrades to
/// `NotDetermined` instead of failing to launch.
mod tcc {
    use std::ffi::c_void;
    use std::sync::OnceLock;

    use objc2_core_foundation::CFString;

    type Preflight = unsafe extern "C" fn(*const CFString, *const c_void) -> i32;
    type Request = unsafe extern "C" fn(*const CFString, *const c_void, *const c_void);

    const SERVICE: &str = "kTCCServiceAudioCapture";

    fn symbol(name: &std::ffi::CStr) -> Option<*mut c_void> {
        static LIB: OnceLock<usize> = OnceLock::new();
        let lib = *LIB.get_or_init(|| unsafe {
            libc::dlopen(
                c"/System/Library/PrivateFrameworks/TCC.framework/Versions/A/TCC".as_ptr(),
                libc::RTLD_NOW,
            ) as usize
        });
        if lib == 0 {
            return None;
        }
        let f = unsafe { libc::dlsym(lib as *mut c_void, name.as_ptr()) };
        (!f.is_null()).then_some(f)
    }

    /// 0 granted, 1 denied, anything else not asked yet.
    pub fn preflight() -> Option<i32> {
        let f: Preflight = unsafe { std::mem::transmute(symbol(c"TCCAccessPreflight")?) };
        let service = CFString::from_str(SERVICE);
        Some(unsafe { f(&raw const *service, std::ptr::null()) })
    }

    pub fn request() -> Option<bool> {
        let f: Request = unsafe { std::mem::transmute(symbol(c"TCCAccessRequest")?) };
        let service = CFString::from_str(SERVICE);
        let (tx, rx) = std::sync::mpsc::channel();
        let block = block2::RcBlock::new(move |granted: u8| {
            let _ = tx.send(granted != 0);
        });
        unsafe {
            f(
                &raw const *service,
                std::ptr::null(),
                block2::RcBlock::as_ptr(&block).cast(),
            );
        }
        rx.recv().ok()
    }
}

pub(crate) fn system_audio_permission() -> Permission {
    match tcc::preflight() {
        Some(0) => Permission::Granted,
        Some(1) => Permission::Denied,
        _ => Permission::NotDetermined,
    }
}

pub(crate) fn request_system_audio_permission() -> bool {
    tcc::request().unwrap_or(false)
}
