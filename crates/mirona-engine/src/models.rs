//! Model downloader (NGR-174): fetches the speech and speaker models the configured providers
//! need, from pinned URLs, with resume and a SHA-256 check per file. The app's onboarding and
//! Settings run it; `scripts/fetch-models.sh` stays for source checkouts.
//!
//! Requests go through [`net::allow_model`]: only these pinned hosts, only when the user asks,
//! and no meeting data in them, so privacy mode lets them through.

use crate::diarize::{COREML_MIN_RAM, ram_bytes};
use mirona_capture::WHISPER_MODELS;
use mirona_providers::{config::Config, net};
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

pub struct File {
    /// Relative to the models folder.
    pub path: &'static str,
    pub url: &'static str,
    pub size: u64,
    pub sha256: &'static str,
}

/// The files the configured providers need on this machine (the hardware profile: the `CoreML`
/// speaker set only on a Mac with enough memory to run it), and the ones not there yet.
/// A file at its full size counts as there; the downloader checked it when it wrote it.
pub fn needed(cfg: &Config, models: &Path) -> Vec<&'static File> {
    let mut packs = vec![VAD];
    if crate::stage(cfg, "stt", "parakeet") == "whisper" {
        if !WHISPER_MODELS.iter().any(|m| models.join(m).is_file()) {
            packs.push(WHISPER_SMALL);
        }
    } else {
        packs.push(PARAKEET);
    }
    let diarization = crate::stage(cfg, "diarization", "speakrs");
    if diarization != "off" {
        packs.push(SPEAKRS_CPU);
        if cfg!(target_os = "macos")
            && diarization != "speakrs-cpu"
            && ram_bytes().is_some_and(|b| b >= COREML_MIN_RAM)
        {
            packs.push(SPEAKRS_COREML);
        }
    }
    packs
        .into_iter()
        .flatten()
        .filter(|f| fs::metadata(models.join(f.path)).map_or(true, |m| m.len() != f.size))
        .collect()
}

/// Downloads every [`needed`] file into `models`. `progress(done, total)` in bytes, counting
/// what earlier attempts left behind. Setting `cancel` stops it between reads; the partial files
/// stay and the next run resumes them.
pub fn download(
    cfg: &Config,
    models: &Path,
    cancel: &AtomicBool,
    mut progress: impl FnMut(u64, u64),
) -> Result<(), String> {
    let files = needed(cfg, models);
    let total = files.iter().map(|f| f.size).sum();
    let mut done = 0;
    progress(done, total);
    for f in files {
        fetch(f, models, cancel, &mut |n| {
            done += n;
            progress(done, total);
        })?;
    }
    Ok(())
}

/// One file: resume `<path>.part`, check its size and SHA-256, then move it into place, so a
/// file under its real name is always whole.
fn fetch(
    f: &File,
    models: &Path,
    cancel: &AtomicBool,
    tick: &mut impl FnMut(u64),
) -> Result<(), String> {
    net::allow_model(f.url).map_err(|e| e.to_string())?;
    let dest = models.join(f.path);
    let part = PathBuf::from(format!("{}.part", dest.display()));
    let err = |e: io::Error| format!("{}: {e}", f.path);
    fs::create_dir_all(dest.parent().unwrap_or(models)).map_err(err)?;
    let mut hash = Sha256::new();
    let mut have = fs::metadata(&part).map_or(0, |m| m.len());
    if have > f.size {
        have = 0;
    }
    let resp = if have < f.size {
        let mut req = ureq::get(f.url);
        if have > 0 {
            req = req.header("Range", format!("bytes={have}-"));
        }
        let resp = req
            .call()
            .map_err(|e| format!("Couldn't download {}: {e}", f.path))?;
        if resp.status() != 206 {
            have = 0; // the server sends the whole file: start over
        }
        Some(resp)
    } else {
        None
    };
    if have > 0 {
        io::copy(&mut fs::File::open(&part).map_err(err)?, &mut hash).map_err(err)?;
        tick(have);
    }
    if let Some(resp) = resp {
        let mut out = OpenOptions::new()
            .create(true)
            .write(true)
            .append(have > 0)
            .truncate(have == 0)
            .open(&part)
            .map_err(err)?;
        let mut body = resp.into_body().into_reader().take(f.size - have);
        let mut buf = vec![0; 1 << 16];
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err("Download cancelled.".into());
            }
            let n = body.read(&mut buf).map_err(err)?;
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n]).map_err(err)?;
            hash.update(&buf[..n]);
            have += n as u64;
            tick(n as u64);
        }
    }
    if have != f.size || format!("{:x}", hash.finalize()) != f.sha256 {
        fs::remove_file(&part).ok();
        return Err(format!(
            "{} didn't download correctly (size or checksum); try again.",
            f.path
        ));
    }
    fs::rename(&part, &dest).map_err(err)
}

// Pinned manifest. Within a pack, the file `missing_models` checks for comes last, so a
// half-downloaded pack never looks installed. Regenerate with the Hugging Face tree API
// (`lfs.oid` is the SHA-256) when a revision moves.
pub const PARAKEET: &[File] = &[
    File {
        path: "sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8/encoder.int8.onnx",
        url: "https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8/resolve/1ab9323565ddb038682214b292f588070a538ce2/encoder.int8.onnx",
        size: 652_184_296,
        sha256: "a32b12d17bbbc309d0686fbbcc2987b5e9b8333a7da83fa6b089f0a2acd651ab",
    },
    File {
        path: "sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8/decoder.int8.onnx",
        url: "https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8/resolve/1ab9323565ddb038682214b292f588070a538ce2/decoder.int8.onnx",
        size: 7_257_753,
        sha256: "b6bb64963457237b900e496ee9994b59294526439fbcc1fecf705b31a15c6b4e",
    },
    File {
        path: "sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8/joiner.int8.onnx",
        url: "https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8/resolve/1ab9323565ddb038682214b292f588070a538ce2/joiner.int8.onnx",
        size: 1_739_080,
        sha256: "7946164367946e7f9f29a122407c3252b680dbae9a51343eb2488d057c3c43d2",
    },
    File {
        path: "sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8/tokens.txt",
        url: "https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8/resolve/1ab9323565ddb038682214b292f588070a538ce2/tokens.txt",
        size: 9_384,
        sha256: "ec182b70dd42113aff6c5372c75cac58c952443eb22322f57bbd7f53977d497d",
    },
];
pub const SPEAKRS_CPU: &[File] = &[
    File {
        path: "speakrs/plda_lda.npy",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/plda_lda.npy",
        size: 131_200,
        sha256: "e20c9b012bebd1aabda5a38a127e63a43cf35debdc502715fc143e2fb6bc3c4b",
    },
    File {
        path: "speakrs/plda_tr.npy",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/plda_tr.npy",
        size: 131_200,
        sha256: "e700b68cb319de3fafb5fa093eb9222c23c447084741f8d3a533640d425510ee",
    },
    File {
        path: "speakrs/plda_mu.npy",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/plda_mu.npy",
        size: 1_152,
        sha256: "d286d48acf99bbc1ed1502fed0a3e361ae5626ce1870c8be9f7397c5e47886c6",
    },
    File {
        path: "speakrs/plda_psi.npy",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/plda_psi.npy",
        size: 1_152,
        sha256: "d7128c9ed2f28a9781971805131129f077c04f948e2df12e52dcdb99f2b4e5f5",
    },
    File {
        path: "speakrs/plda_mean1.npy",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/plda_mean1.npy",
        size: 2_176,
        sha256: "e424c0c352182aa8e0f555dec1f3b30e29a20b9ed6b25d339f112af92e51e36f",
    },
    File {
        path: "speakrs/plda_mean2.npy",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/plda_mean2.npy",
        size: 640,
        sha256: "6f6fb708a2037197b5b84ffeaa8f140cb878088fbecd6ab042ad26a7691bd2cf",
    },
    File {
        path: "speakrs/wespeaker-voxceleb-resnet34.min_num_samples.txt",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/wespeaker-voxceleb-resnet34.min_num_samples.txt",
        size: 4,
        sha256: "e4df891c484d7abb985dadf539fa1883a646dab6337af5cae4159c587b7050cc",
    },
    File {
        path: "speakrs/segmentation-3.0.safetensors",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/segmentation-3.0.safetensors",
        size: 5_897_576,
        sha256: "88e7883cf56f679170e162419efddf970f929915281ebce80e351c42b5a7115d",
    },
    File {
        path: "speakrs/wespeaker-multimask-tail.safetensors",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/wespeaker-multimask-tail.safetensors",
        size: 26_527_656,
        sha256: "b97b090ceebcf10924732ad4552a2d867d76d38dcc1409a13687a6ba71ff1ef3",
    },
];
pub const SPEAKRS_COREML: &[File] = &[
    File {
        path: "speakrs/segmentation-3.0.mlmodelc/model.mil",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/segmentation-3.0.mlmodelc/model.mil",
        size: 37_089,
        sha256: "cb2ee400e54e7cd1ba1dfac3d60dc36c3d01f0152e5d4a559f7873eb7715d96f",
    },
    File {
        path: "speakrs/segmentation-3.0.mlmodelc/coremldata.bin",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/segmentation-3.0.mlmodelc/coremldata.bin",
        size: 439,
        sha256: "ac710fb9bcd0310d0c40fd82f2350ca5287b18596da33470bf5185be148aad81",
    },
    File {
        path: "speakrs/segmentation-3.0.mlmodelc/analytics/coremldata.bin",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/segmentation-3.0.mlmodelc/analytics/coremldata.bin",
        size: 243,
        sha256: "129d4f2316a01d29c2636cb72fea64880086685250918bd6a89ea2b770286e68",
    },
    File {
        path: "speakrs/segmentation-3.0.mlmodelc/weights/weight.bin",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/segmentation-3.0.mlmodelc/weights/weight.bin",
        size: 5_959_360,
        sha256: "c3189a64946c75bc24fcb98afe89ad78c52bdbadfdf65e857fb1b81e2cc9fbb2",
    },
    File {
        path: "speakrs/segmentation-3.0-b32.mlmodelc/model.mil",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/segmentation-3.0-b32.mlmodelc/model.mil",
        size: 37_089,
        sha256: "cb2ee400e54e7cd1ba1dfac3d60dc36c3d01f0152e5d4a559f7873eb7715d96f",
    },
    File {
        path: "speakrs/segmentation-3.0-b32.mlmodelc/coremldata.bin",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/segmentation-3.0-b32.mlmodelc/coremldata.bin",
        size: 439,
        sha256: "ac710fb9bcd0310d0c40fd82f2350ca5287b18596da33470bf5185be148aad81",
    },
    File {
        path: "speakrs/segmentation-3.0-b32.mlmodelc/analytics/coremldata.bin",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/segmentation-3.0-b32.mlmodelc/analytics/coremldata.bin",
        size: 243,
        sha256: "129d4f2316a01d29c2636cb72fea64880086685250918bd6a89ea2b770286e68",
    },
    File {
        path: "speakrs/segmentation-3.0-b32.mlmodelc/weights/weight.bin",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/segmentation-3.0-b32.mlmodelc/weights/weight.bin",
        size: 5_959_360,
        sha256: "c3189a64946c75bc24fcb98afe89ad78c52bdbadfdf65e857fb1b81e2cc9fbb2",
    },
    File {
        path: "speakrs/segmentation-3.0-b64.mlmodelc/model.mil",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/segmentation-3.0-b64.mlmodelc/model.mil",
        size: 26_777,
        sha256: "d2d35b55977eb6631cfb456eb67e8a771e834ac972ae051196ec68e9cacd9bac",
    },
    File {
        path: "speakrs/segmentation-3.0-b64.mlmodelc/coremldata.bin",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/segmentation-3.0-b64.mlmodelc/coremldata.bin",
        size: 150,
        sha256: "0460ffe22d22a0ae0dc86c0cc22d94c074f29bbd6be1434707a66b5e154796c8",
    },
    File {
        path: "speakrs/segmentation-3.0-b64.mlmodelc/analytics/coremldata.bin",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/segmentation-3.0-b64.mlmodelc/analytics/coremldata.bin",
        size: 243,
        sha256: "7497fe39c383a49061eae9ebb3052a59360752ee96deeee46ae701f9afbd6e4d",
    },
    File {
        path: "speakrs/segmentation-3.0-b64.mlmodelc/weights/weight.bin",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/segmentation-3.0-b64.mlmodelc/weights/weight.bin",
        size: 6_024_960,
        sha256: "0b8ef91e4a97b435f0f3a4bb66ca647ff53820edc4cb747c242413045a4aaa56",
    },
    File {
        path: "speakrs/wespeaker-fbank.mlmodelc/model.mil",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/wespeaker-fbank.mlmodelc/model.mil",
        size: 7_997,
        sha256: "acee98c155b533afa6a7eb3d5e2158cb4005f85999be06f12276f94c7fab9d34",
    },
    File {
        path: "speakrs/wespeaker-fbank.mlmodelc/coremldata.bin",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/wespeaker-fbank.mlmodelc/coremldata.bin",
        size: 168,
        sha256: "4241bd2543b6face59368f98e9cb7a049c46db8cc5ebd70a12814d3382324ede",
    },
    File {
        path: "speakrs/wespeaker-fbank.mlmodelc/analytics/coremldata.bin",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/wespeaker-fbank.mlmodelc/analytics/coremldata.bin",
        size: 243,
        sha256: "b4a6a692403029bf4d417d17ba0e43ab89482e9d862d4ed7d897b309d7455910",
    },
    File {
        path: "speakrs/wespeaker-fbank.mlmodelc/weights/weight.bin",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/wespeaker-fbank.mlmodelc/weights/weight.bin",
        size: 1_778_432,
        sha256: "27396cb0afdd09164a9d6b2dbd10688bed15230948b3e0a6692ec490c03ae4d7",
    },
    File {
        path: "speakrs/wespeaker-fbank-b32.mlmodelc/model.mil",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/wespeaker-fbank-b32.mlmodelc/model.mil",
        size: 7_997,
        sha256: "acee98c155b533afa6a7eb3d5e2158cb4005f85999be06f12276f94c7fab9d34",
    },
    File {
        path: "speakrs/wespeaker-fbank-b32.mlmodelc/coremldata.bin",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/wespeaker-fbank-b32.mlmodelc/coremldata.bin",
        size: 168,
        sha256: "4241bd2543b6face59368f98e9cb7a049c46db8cc5ebd70a12814d3382324ede",
    },
    File {
        path: "speakrs/wespeaker-fbank-b32.mlmodelc/analytics/coremldata.bin",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/wespeaker-fbank-b32.mlmodelc/analytics/coremldata.bin",
        size: 243,
        sha256: "b4a6a692403029bf4d417d17ba0e43ab89482e9d862d4ed7d897b309d7455910",
    },
    File {
        path: "speakrs/wespeaker-fbank-b32.mlmodelc/weights/weight.bin",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/wespeaker-fbank-b32.mlmodelc/weights/weight.bin",
        size: 1_778_432,
        sha256: "27396cb0afdd09164a9d6b2dbd10688bed15230948b3e0a6692ec490c03ae4d7",
    },
    File {
        path: "speakrs/wespeaker-fbank-30s.mlmodelc/model.mil",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/wespeaker-fbank-30s.mlmodelc/model.mil",
        size: 7_131,
        sha256: "34a76696a6318312ce0557b93e0d4defabfdbc540a682ae5dfbdc80d1f52f52d",
    },
    File {
        path: "speakrs/wespeaker-fbank-30s.mlmodelc/coremldata.bin",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/wespeaker-fbank-30s.mlmodelc/coremldata.bin",
        size: 153,
        sha256: "d52fb49c6521e14c9366e0194e1e91f4550d8ba39d31e03e72bfe014b1e826fe",
    },
    File {
        path: "speakrs/wespeaker-fbank-30s.mlmodelc/analytics/coremldata.bin",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/wespeaker-fbank-30s.mlmodelc/analytics/coremldata.bin",
        size: 243,
        sha256: "413a9d484295f712f6a496624abd9a663e72149d2fe43f8ad101dc1dc09981d0",
    },
    File {
        path: "speakrs/wespeaker-fbank-30s.mlmodelc/weights/weight.bin",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/wespeaker-fbank-30s.mlmodelc/weights/weight.bin",
        size: 1_778_432,
        sha256: "27396cb0afdd09164a9d6b2dbd10688bed15230948b3e0a6692ec490c03ae4d7",
    },
    File {
        path: "speakrs/wespeaker-multimask-tail-b32.mlmodelc/model.mil",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/wespeaker-multimask-tail-b32.mlmodelc/model.mil",
        size: 61_635,
        sha256: "15af971119deecf2729de0c32a9f9af17cf6769cf3a10d45ada114ddd7c54d3b",
    },
    File {
        path: "speakrs/wespeaker-multimask-tail-b32.mlmodelc/coremldata.bin",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/wespeaker-multimask-tail-b32.mlmodelc/coremldata.bin",
        size: 201,
        sha256: "5e5467edd5d317c287cfa06474216990b8dfff1b7f6e725fb2e238a2264d8b16",
    },
    File {
        path: "speakrs/wespeaker-multimask-tail-b32.mlmodelc/analytics/coremldata.bin",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/wespeaker-multimask-tail-b32.mlmodelc/analytics/coremldata.bin",
        size: 243,
        sha256: "77b60e64442b2140adebed82813bf619f76d92c9f419639093a50a27744ac14b",
    },
    File {
        path: "speakrs/wespeaker-multimask-tail-b32.mlmodelc/weights/weight.bin",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/wespeaker-multimask-tail-b32.mlmodelc/weights/weight.bin",
        size: 26_525_120,
        sha256: "18f777be6e47d2d9d5792d475457add3b71a677814ac66cadc90e5410d14b252",
    },
    File {
        path: "speakrs/wespeaker-voxceleb-resnet34-tail.mlmodelc/model.mil",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/wespeaker-voxceleb-resnet34-tail.mlmodelc/model.mil",
        size: 60_706,
        sha256: "630709365400a429358025104c67a240ae9dc5c555dd9f9c72203be33ca51fc6",
    },
    File {
        path: "speakrs/wespeaker-voxceleb-resnet34-tail.mlmodelc/coremldata.bin",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/wespeaker-voxceleb-resnet34-tail.mlmodelc/coremldata.bin",
        size: 218,
        sha256: "7e35919b7985082e3fe0fa8554679632383ef815f863f29a133a23a0bf17898a",
    },
    File {
        path: "speakrs/wespeaker-voxceleb-resnet34-tail.mlmodelc/analytics/coremldata.bin",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/wespeaker-voxceleb-resnet34-tail.mlmodelc/analytics/coremldata.bin",
        size: 243,
        sha256: "07abd12d7cdb8af793b6d439b40bd9e5c1f44b9eb69cbb3d3d272f494a77a556",
    },
    File {
        path: "speakrs/wespeaker-voxceleb-resnet34-tail.mlmodelc/weights/weight.bin",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/wespeaker-voxceleb-resnet34-tail.mlmodelc/weights/weight.bin",
        size: 26_525_120,
        sha256: "18f777be6e47d2d9d5792d475457add3b71a677814ac66cadc90e5410d14b252",
    },
    File {
        path: "speakrs/wespeaker-voxceleb-resnet34-tail-b3.mlmodelc/model.mil",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/wespeaker-voxceleb-resnet34-tail-b3.mlmodelc/model.mil",
        size: 60_706,
        sha256: "630709365400a429358025104c67a240ae9dc5c555dd9f9c72203be33ca51fc6",
    },
    File {
        path: "speakrs/wespeaker-voxceleb-resnet34-tail-b3.mlmodelc/coremldata.bin",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/wespeaker-voxceleb-resnet34-tail-b3.mlmodelc/coremldata.bin",
        size: 218,
        sha256: "7e35919b7985082e3fe0fa8554679632383ef815f863f29a133a23a0bf17898a",
    },
    File {
        path: "speakrs/wespeaker-voxceleb-resnet34-tail-b3.mlmodelc/analytics/coremldata.bin",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/wespeaker-voxceleb-resnet34-tail-b3.mlmodelc/analytics/coremldata.bin",
        size: 243,
        sha256: "07abd12d7cdb8af793b6d439b40bd9e5c1f44b9eb69cbb3d3d272f494a77a556",
    },
    File {
        path: "speakrs/wespeaker-voxceleb-resnet34-tail-b3.mlmodelc/weights/weight.bin",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/wespeaker-voxceleb-resnet34-tail-b3.mlmodelc/weights/weight.bin",
        size: 26_525_120,
        sha256: "18f777be6e47d2d9d5792d475457add3b71a677814ac66cadc90e5410d14b252",
    },
    File {
        path: "speakrs/wespeaker-voxceleb-resnet34-tail-b32.mlmodelc/model.mil",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/wespeaker-voxceleb-resnet34-tail-b32.mlmodelc/model.mil",
        size: 60_706,
        sha256: "630709365400a429358025104c67a240ae9dc5c555dd9f9c72203be33ca51fc6",
    },
    File {
        path: "speakrs/wespeaker-voxceleb-resnet34-tail-b32.mlmodelc/coremldata.bin",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/wespeaker-voxceleb-resnet34-tail-b32.mlmodelc/coremldata.bin",
        size: 218,
        sha256: "7e35919b7985082e3fe0fa8554679632383ef815f863f29a133a23a0bf17898a",
    },
    File {
        path: "speakrs/wespeaker-voxceleb-resnet34-tail-b32.mlmodelc/analytics/coremldata.bin",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/wespeaker-voxceleb-resnet34-tail-b32.mlmodelc/analytics/coremldata.bin",
        size: 243,
        sha256: "07abd12d7cdb8af793b6d439b40bd9e5c1f44b9eb69cbb3d3d272f494a77a556",
    },
    File {
        path: "speakrs/wespeaker-voxceleb-resnet34-tail-b32.mlmodelc/weights/weight.bin",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/wespeaker-voxceleb-resnet34-tail-b32.mlmodelc/weights/weight.bin",
        size: 26_525_120,
        sha256: "18f777be6e47d2d9d5792d475457add3b71a677814ac66cadc90e5410d14b252",
    },
    File {
        path: "speakrs/wespeaker-chunk-emb-p1s-w21.mlmodelc/model.mil",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/wespeaker-chunk-emb-p1s-w21.mlmodelc/model.mil",
        size: 114_391,
        sha256: "61f48809599ce8e7ebbb6bbd02c5298d61089e4f092f764c8b066e5988da169f",
    },
    File {
        path: "speakrs/wespeaker-chunk-emb-p1s-w21.mlmodelc/coremldata.bin",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/wespeaker-chunk-emb-p1s-w21.mlmodelc/coremldata.bin",
        size: 170,
        sha256: "31e3299dfb25d8165c0e7783f753d323572f988406dda7970310b9cc8f9d313f",
    },
    File {
        path: "speakrs/wespeaker-chunk-emb-p1s-w21.mlmodelc/analytics/coremldata.bin",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/wespeaker-chunk-emb-p1s-w21.mlmodelc/analytics/coremldata.bin",
        size: 243,
        sha256: "1387900f3e56403c2f81861c3b93ec69028cb9a29fe73ad2fbc03433a4003e5b",
    },
    File {
        path: "speakrs/wespeaker-chunk-emb-p1s-w21.mlmodelc/weights/weight.bin",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/wespeaker-chunk-emb-p1s-w21.mlmodelc/weights/weight.bin",
        size: 27_185_408,
        sha256: "a2e7c10340339c18bb7bddeb952f9b36b081d09bb15b372d6003a46667fb7d7e",
    },
    File {
        path: "speakrs/wespeaker-chunk-emb-p1s-w36.mlmodelc/model.mil",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/wespeaker-chunk-emb-p1s-w36.mlmodelc/model.mil",
        size: 114_293,
        sha256: "1c9618aba9457b30e095f6fbbeed9b40787d415854150146b7ec1f91e048edf2",
    },
    File {
        path: "speakrs/wespeaker-chunk-emb-p1s-w36.mlmodelc/coremldata.bin",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/wespeaker-chunk-emb-p1s-w36.mlmodelc/coremldata.bin",
        size: 170,
        sha256: "a16ac9accd7f945b02d03cd3b15885152d698cdb87e4ac3641350882d308f87c",
    },
    File {
        path: "speakrs/wespeaker-chunk-emb-p1s-w36.mlmodelc/analytics/coremldata.bin",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/wespeaker-chunk-emb-p1s-w36.mlmodelc/analytics/coremldata.bin",
        size: 243,
        sha256: "7a5a434d7a10e7903a8a4ccd52227139870f41dc5a76e907d6a83ca7ba24779e",
    },
    File {
        path: "speakrs/wespeaker-chunk-emb-p1s-w36.mlmodelc/weights/weight.bin",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/wespeaker-chunk-emb-p1s-w36.mlmodelc/weights/weight.bin",
        size: 27_644_672,
        sha256: "35c2339da4c98b2f76140611c549608b3087277cd0de4e81456e55b0d4c15cd3",
    },
    File {
        path: "speakrs/wespeaker-chunk-emb-p1s-w51.mlmodelc/model.mil",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/wespeaker-chunk-emb-p1s-w51.mlmodelc/model.mil",
        size: 114_489,
        sha256: "9390b75222de14826f908633fe3d5cf5cb17f37069edf94d5c66e903aeb0d047",
    },
    File {
        path: "speakrs/wespeaker-chunk-emb-p1s-w51.mlmodelc/coremldata.bin",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/wespeaker-chunk-emb-p1s-w51.mlmodelc/coremldata.bin",
        size: 172,
        sha256: "d67e24cf8b151090f18a99031511f4a7fe8185b32503afcdadbb986b12f0c901",
    },
    File {
        path: "speakrs/wespeaker-chunk-emb-p1s-w51.mlmodelc/analytics/coremldata.bin",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/wespeaker-chunk-emb-p1s-w51.mlmodelc/analytics/coremldata.bin",
        size: 243,
        sha256: "9b46b26429aa1b0f585cf919335995bfa272c49398d5f06bf8e882ff446cb2a9",
    },
    File {
        path: "speakrs/wespeaker-chunk-emb-p1s-w51.mlmodelc/weights/weight.bin",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/wespeaker-chunk-emb-p1s-w51.mlmodelc/weights/weight.bin",
        size: 28_122_176,
        sha256: "c4cd6f10933cf89fbcc6e2c7b5060b584e33cd7f446dadfbcabca3cea6ada4ee",
    },
    File {
        path: "speakrs/wespeaker-chunk-emb-p1s-w81.mlmodelc/model.mil",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/wespeaker-chunk-emb-p1s-w81.mlmodelc/model.mil",
        size: 114_525,
        sha256: "ad8a5a72c351a532d30dde0cf13c9e951c1e7cab5bbfc43c18bf7cc174fd6878",
    },
    File {
        path: "speakrs/wespeaker-chunk-emb-p1s-w81.mlmodelc/coremldata.bin",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/wespeaker-chunk-emb-p1s-w81.mlmodelc/coremldata.bin",
        size: 172,
        sha256: "420854829b005a33bb865bfe9f7796124f7c4b368c440302155f94764e365b97",
    },
    File {
        path: "speakrs/wespeaker-chunk-emb-p1s-w81.mlmodelc/analytics/coremldata.bin",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/wespeaker-chunk-emb-p1s-w81.mlmodelc/analytics/coremldata.bin",
        size: 243,
        sha256: "2c6ee3bf20d79830dafbf6c082d6183ab9487684896549ccc0db3f4a866edf79",
    },
    File {
        path: "speakrs/wespeaker-chunk-emb-p1s-w81.mlmodelc/weights/weight.bin",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/wespeaker-chunk-emb-p1s-w81.mlmodelc/weights/weight.bin",
        size: 29_058_880,
        sha256: "a7b2228cae71118b65b325492fc7a8d4c3120913d532ef234ad2857c2f5a82c1",
    },
    File {
        path: "speakrs/wespeaker-chunk-emb-p1s-w111.mlmodelc/model.mil",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/wespeaker-chunk-emb-p1s-w111.mlmodelc/model.mil",
        size: 114_574,
        sha256: "46f7ede43705b74e626693fc7d01b7cc21652bf0fdf95888f388374932d9ee34",
    },
    File {
        path: "speakrs/wespeaker-chunk-emb-p1s-w111.mlmodelc/coremldata.bin",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/wespeaker-chunk-emb-p1s-w111.mlmodelc/coremldata.bin",
        size: 172,
        sha256: "7ca6093680e486d7c0a9c13bfdf21fafd77dc08ec345a43981aa0ac1131e54a1",
    },
    File {
        path: "speakrs/wespeaker-chunk-emb-p1s-w111.mlmodelc/analytics/coremldata.bin",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/wespeaker-chunk-emb-p1s-w111.mlmodelc/analytics/coremldata.bin",
        size: 243,
        sha256: "0182f87233c3333be26446c1134f21145d0dc7c8a0ea97e09fb88fdafdcffce4",
    },
    File {
        path: "speakrs/wespeaker-chunk-emb-p1s-w111.mlmodelc/weights/weight.bin",
        url: "https://huggingface.co/avencera/speakrs-models/resolve/67f963b6a1d94fcd4678948237b3cc8bd69bf1d8/wespeaker-chunk-emb-p1s-w111.mlmodelc/weights/weight.bin",
        size: 29_995_520,
        sha256: "b4c0cb7d03b8fd43923a3a4c002321d381113789a27273aa070da4eae693f3c7",
    },
];
pub const WHISPER_SMALL: &[File] = &[File {
    path: "ggml-small.en.bin",
    url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-small.en.bin",
    size: 487_614_201,
    sha256: "c6138d6d58ecc8322097e0f987c32f1be8bb0a18532a3f88f734d1bbf9c41e5d",
}];
pub const VAD: &[File] = &[File {
    path: "silero_vad.onnx",
    url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/silero_vad.onnx",
    size: 643_854,
    sha256: "9e2449e1087496d8d4caba907f23e0bd3f78d91fa552479bb9c23ac09cbb1fd6",
}];
