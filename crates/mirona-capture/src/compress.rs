//! Compressed recordings: 16 kHz mono Opus (24 kbit/s, ~11 MB/hour) in a CAF file.
//!
//! The recorder still writes a WAV while capturing (its header is rewritten every 100 ms, so a
//! crash keeps everything up to then). [`compress`] turns that WAV into `audio.caf` afterwards
//! and checks the result decodes to the same number of samples; the caller deletes the WAV.
//!
//! Why Opus in CAF: `WebKit` (the macOS webview) plays and seeks it through `AVFoundation` (CAF's
//! packet table is the seek index), while Opus in Ogg doesn't play there. Symphonia has no Opus
//! decoder, so reading goes through libopus here too ([`decode_caf`]), on every OS.

use std::path::{Path, PathBuf};

use opus::{Application, Bitrate, Channels, Decoder, Encoder};

use crate::{Resampler, SAMPLE_RATE, downmix};

/// 20 ms packets at 16 kHz.
const FRAME: usize = 320;
/// Speech stays clear (Parakeet's WER matches the WAV) at a tenth of the WAV's size.
pub const BITRATE: i32 = 24_000;

/// Encode `input` (anything [`crate::decode`] reads, normally the recording's WAV) to
/// `<input>.caf` next to it, verify it decodes back to the same length, and return its path.
/// `input` is left alone; a failed run leaves nothing behind.
pub fn compress(input: &Path) -> Result<PathBuf, String> {
    let out = input.with_extension("caf");
    let part = input.with_extension("caf.part");
    let run = || {
        let n = encode_caf(input, &part)?;
        let mut m = 0;
        decode_caf(&part, &mut |_, pcm, _| {
            m += pcm.len() as u64;
            Ok(())
        })?;
        if m != n {
            return Err(format!(
                "{}: decodes to {m} samples, not {n}",
                part.display()
            ));
        }
        std::fs::rename(&part, &out).map_err(|e| format!("{}: {e}", out.display()))
    };
    run().inspect_err(|_| {
        std::fs::remove_file(&part).ok();
    })?;
    Ok(out)
}

/// Write `input` as Opus in CAF at `out`; returns the number of 16 kHz samples encoded.
fn encode_caf(input: &Path, out: &Path) -> Result<u64, String> {
    let err = |e: opus::Error| format!("opus: {e}");
    let mut enc = Encoder::new(SAMPLE_RATE, Channels::Mono, Application::Voip).map_err(err)?;
    enc.set_bitrate(Bitrate::Bits(BITRATE)).map_err(err)?;
    let pre = usize::try_from(enc.get_lookahead().map_err(err)?).unwrap_or(0);

    let (mut sizes, mut data, mut pending) = (Vec::<usize>::new(), Vec::new(), Vec::new());
    let mut packet = [0u8; 1500];
    let mut encode = |pending: &mut Vec<f32>| -> Result<(), String> {
        for frame in pending.as_chunks::<FRAME>().0 {
            let len = enc.encode_float(frame, &mut packet).map_err(err)?;
            data.extend_from_slice(&packet[..len]);
            sizes.push(len);
        }
        pending.drain(..pending.len() / FRAME * FRAME);
        Ok(())
    };
    let mut rs = Resampler::default();
    let mut n = 0;
    crate::decode(input, &mut |rate, mono| {
        let pcm = rs.to_16k(rate, mono);
        n += pcm.len();
        pending.extend(pcm);
        encode(&mut pending)
    })?;
    // The decoder starts `pre` samples late: pad the end so the last real sample gets coded.
    let packets = (n + pre).div_ceil(FRAME);
    pending.resize(packets * FRAME - (n - pending.len()), 0.0);
    encode(&mut pending)?;
    drop(encode);

    let be32 = |v: usize| u32::try_from(v).unwrap_or(u32::MAX).to_be_bytes();
    let mut f = b"caff\0\x01\0\0".to_vec();
    let mut chunk = |kind: &[u8; 4], body: &[u8]| {
        f.extend_from_slice(kind);
        f.extend_from_slice(&(body.len() as u64).to_be_bytes());
        f.extend_from_slice(body);
    };
    let mut desc = f64::from(SAMPLE_RATE).to_be_bytes().to_vec();
    for v in [0x6f70_7573, 0, 0, FRAME, 1, 0] {
        desc.extend_from_slice(&be32(v)); // 'opus', flags, bytes/packet, frames/packet, ch, bits
    }
    chunk(b"desc", &desc);
    let mut pakt = Vec::new();
    pakt.extend_from_slice(&(packets as u64).to_be_bytes());
    pakt.extend_from_slice(&(n as u64).to_be_bytes());
    pakt.extend_from_slice(&be32(pre));
    pakt.extend_from_slice(&be32(packets * FRAME - n - pre));
    for s in sizes {
        // Big-endian base-128, high bit = more bytes follow.
        let mut v = vec![u8::try_from(s & 0x7f).unwrap_or(0)];
        let mut s = s >> 7;
        while s > 0 {
            v.insert(0, 0x80 | u8::try_from(s & 0x7f).unwrap_or(0));
            s >>= 7;
        }
        pakt.extend(v);
    }
    chunk(b"pakt", &pakt);
    chunk(b"data", &[&[0u8; 4][..], &data].concat()); // edit count 0, then the packets

    let io = |e: std::io::Error| format!("{}: {e}", out.display());
    let mut file = std::fs::File::create(out).map_err(io)?;
    std::io::Write::write_all(&mut file, &f).map_err(io)?;
    file.sync_all().map_err(io)?;
    Ok(n as u64)
}

/// Decoded audio: sample rate, a block of mono samples, the total length in frames.
pub(crate) type OnAudioLen<'a> = dyn FnMut(u32, &[f32], Option<u64>) -> Result<(), String> + 'a;

/// True when `path` starts like a CAF file.
pub(crate) fn is_caf(path: &Path) -> bool {
    let mut magic = [0u8; 4];
    std::fs::File::open(path)
        .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut magic))
        .is_ok_and(|()| &magic == b"caff")
}

/// Decode Opus in CAF (ours, or Apple's `afconvert -d opus`): mono blocks, their rate and the
/// total length in frames, priming and padding trimmed.
pub(crate) fn decode_caf(path: &Path, on_audio: &mut OnAudioLen<'_>) -> Result<(), String> {
    let bad = |what: &str| format!("{}: {what}", path.display());
    let d = std::fs::read(path).map_err(|e| bad(&e.to_string()))?;
    let (mut desc, mut pakt, mut data) = (None, None, None);
    let mut pos = 8;
    while pos + 12 <= d.len() {
        let size = i64::from_be_bytes(d[pos + 4..pos + 12].try_into().unwrap_or_default());
        let start = pos + 12;
        // -1: the data chunk runs to the end of the file.
        let end = usize::try_from(size).map_or(d.len(), |s| start.saturating_add(s));
        let body = d
            .get(start..end)
            .ok_or_else(|| bad("truncated CAF chunk"))?;
        match &d[pos..pos + 4] {
            b"desc" => desc = Some(body),
            b"pakt" => pakt = Some(body),
            b"data" => data = body.get(4..), // after the edit count
            _ => {}
        }
        pos = end;
    }
    let (Some(desc), Some(pakt), Some(data)) = (desc, pakt, data) else {
        return Err(bad("not a CAF file with desc, pakt and data"));
    };
    let u32_at = |b: &[u8], i: usize| {
        b.get(i..i + 4)
            .map_or(0, |s| u32::from_be_bytes(s.try_into().unwrap_or_default()))
    };
    let u64_at = |b: &[u8], i: usize| {
        b.get(i..i + 8)
            .map_or(0, |s| u64::from_be_bytes(s.try_into().unwrap_or_default()))
    };
    if desc.get(8..12) != Some(b"opus") || pakt.len() < 24 {
        return Err(bad("only Opus CAF files are supported"));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let rate = f64::from_bits(u64_at(desc, 0)) as u32;
    let channels = match u32_at(desc, 24) {
        1 => Channels::Mono,
        2 => Channels::Stereo,
        n => return Err(bad(&format!("{n} channels"))),
    };
    let ch = channels as usize;
    let (packets, valid) = (u64_at(pakt, 0), u64_at(pakt, 8));
    let mut skip = u64::from(u32_at(pakt, 16));
    let mut left = valid;

    let mut decoder = Decoder::new(rate, channels).map_err(|e| bad(&format!("opus: {e}")))?;
    let mut pcm = vec![0f32; 5760 * ch]; // 120 ms at 48 kHz: the longest Opus packet
    let mut block = Vec::new();
    let (mut sizes, mut at) = (&pakt[24..], 0usize);
    for _ in 0..packets {
        let mut size = 0usize;
        loop {
            let (&b, rest) = sizes
                .split_first()
                .ok_or_else(|| bad("truncated packet table"))?;
            sizes = rest;
            size = size << 7 | usize::from(b & 0x7f);
            if b & 0x80 == 0 {
                break;
            }
        }
        let packet = data
            .get(at..at + size)
            .ok_or_else(|| bad("truncated audio data"))?;
        at += size;
        let frames = decoder
            .decode_float(packet, &mut pcm, false)
            .map_err(|e| bad(&format!("opus: {e}")))?;
        let mono = downmix(&pcm[..frames * ch], ch);
        let from = usize::try_from(skip).unwrap_or(usize::MAX).min(mono.len());
        skip -= from as u64;
        let take = usize::try_from(left)
            .unwrap_or(usize::MAX)
            .min(mono.len() - from);
        left -= take as u64;
        block.extend_from_slice(&mono[from..from + take]);
        if block.len() >= rate as usize {
            on_audio(rate, &block, Some(valid))?;
            block.clear();
        }
    }
    if !block.is_empty() {
        on_audio(rate, &block, Some(valid))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{REFERENCE, fixture, test_models_dir, wer};
    use crate::{Stt, VadConfig, transcribe_file};

    fn temp(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("mirona-compress-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    fn samples(path: &Path) -> Vec<f32> {
        let mut out = Vec::new();
        crate::decode(path, &mut |rate, m| {
            assert_eq!(rate, 16_000);
            out.extend_from_slice(m);
            Ok(())
        })
        .unwrap();
        out
    }

    /// WAV → CAF keeps every sample, stays under 30 MB/hour, and still sounds like the WAV.
    #[test]
    fn round_trip_keeps_length_and_shrinks() {
        let wav = temp("rt.wav");
        std::fs::copy(fixture("librispeech-2utt.wav"), &wav).unwrap();
        let caf = compress(&wav).unwrap();
        assert_eq!(caf, wav.with_extension("caf"));
        assert!(!wav.with_extension("caf.part").exists());

        let (a, b) = (samples(&wav), samples(&caf));
        assert_eq!(a.len(), b.len(), "length must survive exactly");
        // Opus is lossy, but the waveform should track the original closely (signal-to-error).
        let (sig, noise) = a.iter().zip(&b).fold((0f64, 0f64), |(s, n), (x, y)| {
            (s + f64::from(*x).powi(2), n + f64::from(x - y).powi(2))
        });
        let snr = 10.0 * (sig / noise).log10();
        assert!(snr > 3.0, "SNR {snr:.1} dB");

        #[allow(clippy::cast_precision_loss)]
        let mb_per_hour =
            std::fs::metadata(&caf).unwrap().len() as f64 / (a.len() as f64 / 16e3) * 3600.0 / 1e6;
        println!("{mb_per_hour:.1} MB/hour, SNR {snr:.1} dB");
        assert!(mb_per_hour < 30.0, "{mb_per_hour:.1} MB/hour");
        std::fs::remove_dir_all(wav.parent().unwrap()).ok();
    }

    #[test]
    fn rejects_files_that_are_not_opus_caf() {
        let f = temp("bad.caf");
        std::fs::write(&f, b"caff\0\x01\0\0garbage").unwrap();
        assert!(crate::decode(&f, &mut |_, _| Ok(())).is_err());
        assert!(compress(Path::new("/nonexistent.wav")).is_err());
        std::fs::remove_file(f).ok();
    }

    /// Apple's tools read our CAF (same length, priming and padding), and we read theirs: what
    /// `WebKit` plays goes through the same `AVFoundation` code.
    #[cfg(target_os = "macos")]
    #[test]
    fn apple_reads_ours_and_we_read_apples() {
        let wav = temp("apple.wav");
        std::fs::copy(fixture("librispeech-2utt.wav"), &wav).unwrap();
        let ours = compress(&wav).unwrap();
        let out = std::process::Command::new("afinfo")
            .arg(&ours)
            .output()
            .unwrap();
        let info = String::from_utf8_lossy(&out.stdout);
        assert!(info.contains("152320 valid frames"), "{info}");

        let theirs = temp("theirs.caf");
        let ok = std::process::Command::new("afconvert")
            .args(["-f", "caff", "-d", "opus", "-b", "24000"])
            .arg(&wav)
            .arg(&theirs)
            .status()
            .unwrap();
        assert!(ok.success());
        assert_eq!(samples(&theirs).len(), 152_320);
        std::fs::remove_dir_all(wav.parent().unwrap()).ok();
    }

    /// Parakeet on the compressed fixture does as well as on the WAV.
    #[test]
    #[ignore = "needs models (MIRONA_MODELS_DIR)"]
    fn compressed_speech_transcribes_like_the_wav() {
        let dir = test_models_dir();
        let stt = Stt::load("parakeet", &dir).unwrap();
        let wav = temp("stt.wav");
        std::fs::copy(fixture("librispeech-2utt.wav"), &wav).unwrap();
        let caf = compress(&wav).unwrap();
        let run = |p: &Path| {
            let segs = transcribe_file(p, &dir, VadConfig::default(), &stt, &mut |_| {}).unwrap();
            let text: Vec<_> = segs.iter().map(|s| s.text.as_str()).collect();
            wer(REFERENCE, &text.join(" "))
        };
        let (w, c) = (run(&wav), run(&caf));
        println!("WER wav {:.1}%, caf {:.1}%", 100.0 * w, 100.0 * c);
        assert!(c <= w + 0.05, "caf WER {c} vs wav {w}");
        std::fs::remove_dir_all(wav.parent().unwrap()).ok();
    }
}
