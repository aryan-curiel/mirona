//! NGR-129 spike: run speakrs on WAV files, write RTTM + per-speaker centroid embeddings.
//!
//! speakrs-spike download <cache_dir>                         -> prints the models dir
//! speakrs-spike run <cpu|coreml|coreml-fast> <models_dir> <out_dir> <wav>...

use std::{fmt::Write as _, fs, path::Path, time::Instant};

use speakrs::{CoreMlComputeUnits, ExecutionMode, ModelManager, PipelineBuilder, RuntimeConfig};

type R<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

fn main() -> R<()> {
  let args: Vec<String> = std::env::args().collect();
  match args.get(1).map(String::as_str) {
    Some("download") => {
      let mgr = ModelManager::with_cache_dir(args[2].clone().into())?;
      for mode in [ExecutionMode::CoreMl, ExecutionMode::CoreMlFast, ExecutionMode::Cpu] {
        println!("{mode:?}: {}", mgr.ensure(mode)?.display());
      }
      Ok(())
    }
    Some("run") => run(&args[2], Path::new(&args[3]), Path::new(&args[4]), &args[5..]),
    _ => Err("usage: download <cache_dir> | run <mode> <models_dir> <out_dir> <wav>...".into()),
  }
}

fn run(mode: &str, models: &Path, out: &Path, wavs: &[String]) -> R<()> {
  let mode = match mode {
    "cpu" => ExecutionMode::Cpu,
    "coreml" => ExecutionMode::CoreMl,
    "coreml-fast" => ExecutionMode::CoreMlFast,
    m => return Err(format!("unknown mode {m}").into()),
  };
  fs::create_dir_all(out)?;
  let t = Instant::now();
  // SPEAKRS_CU=cpu-ne|cpu: CoreML compute units for the embedding models (memory experiment).
  let units = match std::env::var("SPEAKRS_CU").as_deref() {
    Ok("cpu-ne") => CoreMlComputeUnits::CpuAndNeuralEngine,
    Ok("cpu") => CoreMlComputeUnits::CpuOnly,
    _ => CoreMlComputeUnits::All,
  };
  let rt = RuntimeConfig { chunk_emb_compute_units: units, ..RuntimeConfig::default() };
  let mut pipe = PipelineBuilder::from_dir(models, mode)?.runtime(rt).build()?;
  let load = t.elapsed().as_secs_f64();
  // Warm-up on 10 s of silence+noise so the first file isn't charged for lazy init.
  let warm: Vec<f32> = (0..160_000).map(|i| ((i * 7919 % 1000) as f32 / 1000.0 - 0.5) * 0.01).collect();
  let t = Instant::now();
  pipe.run(&warm)?;
  println!("mode={mode:?} load_s={load:.3} warmup_s={:.3}", t.elapsed().as_secs_f64());

  for wav in wavs {
    let id = Path::new(wav).file_stem().unwrap().to_string_lossy().to_string();
    let audio = load_wav(wav)?;
    let dur = audio.len() as f64 / 16_000.0;
    let t = Instant::now();
    let res = pipe.run(&audio)?;
    let secs = t.elapsed().as_secs_f64();
    let n_spk = res.segments.iter().map(|s| &s.speaker).collect::<std::collections::HashSet<_>>().len();
    println!(
      "file={id} audio_s={dur:.1} proc_s={secs:.3} rtf={:.4} speed_x={:.0} speakers={n_spk}",
      secs / dur,
      dur / secs
    );
    fs::write(out.join(format!("{id}.rttm")), res.rttm(&id))?;
    fs::write(out.join(format!("{id}.emb.json")), centroids(&res))?;
  }
  Ok(())
}

/// Per-cluster mean of L2-normalised chunk embeddings (raw WeSpeaker 256-d, pre-PLDA).
/// Column k of `discrete_diarization` == cluster k == RTTM label SPEAKER_k.
/// Also emits `emb3`/`emb10`: means of only the first 3/10 chunk embeddings (short-enrollment test;
/// chunks are 10 s windows at 1 s step, so 3 chunks ~ 12 s of audio, 10 ~ 20 s).
fn centroids(res: &speakrs::DiarizationResult) -> String {
  let (chunks, spk, dim) = res.embeddings.dim();
  if res.hard_clusters.dim() != (chunks, spk) {
    return "{}".into(); // no speech found -> pipeline returns empty clusters
  }
  let mut sums: std::collections::BTreeMap<i32, (Vec<f64>, usize)> = Default::default();
  let mut firsts: std::collections::BTreeMap<(i32, usize), Vec<f64>> = Default::default();
  for c in 0..chunks {
    for s in 0..spk {
      let k = res.hard_clusters[[c, s]];
      if k < 0 {
        continue;
      }
      let v: Vec<f64> = (0..dim).map(|d| f64::from(res.embeddings[[c, s, d]])).collect();
      let n = v.iter().map(|x| x * x).sum::<f64>().sqrt();
      if !n.is_finite() || n == 0.0 {
        continue;
      }
      let e = sums.entry(k).or_insert_with(|| (vec![0.0; dim], 0));
      e.0.iter_mut().zip(&v).for_each(|(a, b)| *a += b / n);
      e.1 += 1;
      for cap in [3, 10] {
        if e.1 <= cap {
          firsts.insert((k, cap), e.0.iter().map(|x| x / e.1 as f64).collect());
        }
      }
    }
  }
  let mut json = String::from("{");
  for (i, (k, (sum, n))) in sums.iter().enumerate() {
    let fmt = |v: &mut dyn Iterator<Item = f64>| v.map(|x| format!("{x:.6}")).collect::<Vec<_>>().join(",");
    let sep = if i == 0 { "" } else { "," };
    let (e3, e10) = (&firsts[&(*k, 3)], &firsts[&(*k, 10)]);
    write!(
      json,
      "{sep}\"SPEAKER_{k:02}\":{{\"n\":{n},\"emb\":[{}],\"emb3\":[{}],\"emb10\":[{}]}}",
      fmt(&mut sum.iter().map(|x| x / *n as f64)),
      fmt(&mut e3.iter().copied()),
      fmt(&mut e10.iter().copied())
    )
    .unwrap();
  }
  json + "}"
}

fn load_wav(path: &str) -> R<Vec<f32>> {
  let mut r = hound::WavReader::open(path)?;
  let s = r.spec();
  if s.channels != 1 || s.sample_rate != 16_000 || s.bits_per_sample != 16 {
    return Err(format!("{path}: need 16 kHz mono s16, got {s:?}").into());
  }
  Ok(r.samples::<i16>().map(|x| x.map(|v| f32::from(v) / 32768.0)).collect::<Result<_, _>>()?)
}
