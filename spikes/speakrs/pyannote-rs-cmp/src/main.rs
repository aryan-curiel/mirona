//! NGR-129: pyannote-rs 0.3.4 baseline, same flow as its `infinite` example (cosine threshold 0.5).
//! pyannote-rs-cmp <models_dir> <out_dir> <wav>...
use std::{fs, path::Path, time::Instant};

use pyannote_rs::{EmbeddingExtractor, EmbeddingManager};

fn main() -> Result<(), Box<dyn std::error::Error>> {
  let args: Vec<String> = std::env::args().collect();
  let (models, out) = (Path::new(&args[1]), Path::new(&args[2]));
  fs::create_dir_all(out)?;
  let seg_model = models.join("segmentation-3.0.onnx");
  let mut extractor = EmbeddingExtractor::new(models.join("wespeaker_en_voxceleb_CAM++.onnx"))?;
  for wav in &args[3..] {
    let id = Path::new(wav).file_stem().unwrap().to_string_lossy().to_string();
    let (samples, rate) = pyannote_rs::read_wav(wav)?;
    let t = Instant::now();
    let mut manager = EmbeddingManager::new(usize::MAX);
    let mut rttm = String::new();
    for seg in pyannote_rs::get_segments(&samples, rate, &seg_model)? {
      let Ok(seg) = seg else { continue };
      let Ok(emb) = extractor.compute(&seg.samples) else { continue };
      let emb: Vec<f32> = emb.collect();
      let spk = manager
        .search_speaker(emb.clone(), 0.5)
        .or_else(|| manager.search_speaker(emb, 0.0))
        .unwrap_or(usize::MAX);
      rttm += &format!(
        "SPEAKER {id} 1 {:.3} {:.3} <NA> <NA> spk{spk} <NA> <NA>\n",
        seg.start,
        seg.end - seg.start
      );
    }
    let secs = t.elapsed().as_secs_f64();
    let dur = samples.len() as f64 / f64::from(rate);
    println!("file={id} audio_s={dur:.1} proc_s={secs:.3} speed_x={:.0} speakers={}", dur / secs, manager.get_all_speakers().len());
    fs::write(out.join(format!("{id}.rttm")), rttm)?;
  }
  Ok(())
}
