//! Ask macOS for the two permissions recording needs, then print the result.
//! `cargo run -p mirona-capture --example permissions`

fn main() {
    println!("microphone:   {:?}", mirona_capture::mic_permission());
    println!(
        "system audio: {:?}",
        mirona_capture::system_audio_permission()
    );
    println!(
        "requesting microphone…   granted: {}",
        mirona_capture::request_mic_permission()
    );
    println!(
        "requesting system audio… granted: {}",
        mirona_capture::request_system_audio_permission()
    );
    println!("microphone:   {:?}", mirona_capture::mic_permission());
    println!(
        "system audio: {:?}",
        mirona_capture::system_audio_permission()
    );
}
