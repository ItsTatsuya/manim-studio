//! Chatterbox Turbo (Resemble AI, MIT) narration on this computer: the same ONNX Runtime
//! Kokoro uses, with the model's own four graphs. A reference clip of any voice makes the
//! narrator sound like it (voice cloning), so a school can use its own teacher.
//!
//! The pipeline is the one on the model's card: a speech encoder reads the reference clip
//! (once), a GPT-2 style language model writes speech tokens for the text one at a time
//! (greedy, with a repetition penalty), and a one-step decoder turns the tokens into
//! 24 kHz audio. Files are the 4-bit set, about 725 MB, pinned by revision and SHA-256.
//!
//! The ONNX export has no watermark step. The model card says its audio carries Resemble's
//! Perth watermark, but that is applied by a separate Python package that is not run here;
//! Settings asks the teacher to confirm they may use the voice they clone.

mod tokenizer;

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::AtomicBool;

use anyhow::{Context, Result, bail};
use ort::value::DynValue;

use crate::kokoro::SAMPLE_RATE;
use tokenizer::Tokenizer;

const REVISION: &str = "d21799bd0354adb85e348b8a0442a8405110a2cf";

/// A downloadable file: its path in the repository (the local file is its last part),
/// size and SHA-256.
pub struct Asset {
    pub path: &'static str,
    pub size: u64,
    sha256: &'static str,
}

impl Asset {
    pub fn name(&self) -> &'static str {
        self.path.rsplit('/').next().unwrap_or(self.path)
    }
}

const fn asset(path: &'static str, size: u64, sha256: &'static str) -> Asset {
    Asset { path, size, sha256 }
}

/// Each graph is a small `.onnx` next to a `.onnx_data` holding its weights; both must be
/// in the same folder.
pub const ASSETS: &[Asset] = &[
    asset("tokenizer.json", 3_562_272, "3f04e34bea22f9144d1a19151154095bc9ce0430bf421304f5797e716288a906"),
    asset("onnx/speech_encoder_q4.onnx", 1_200_346, "37956c20b67bed85a0da4bc83509d67b5969a1b257d1c546516a5236a17ad71e"),
    asset("onnx/speech_encoder_q4.onnx_data", 229_560_112, "58956db217c6443e49c91bdd54d7cf76b4a243f225c748b7bf746459fc27bc7d"),
    asset("onnx/embed_tokens_q4.onnx", 2_844, "fd6ba1d22902e8f539d3dd6d7c1c44b98ebb4c84ebbb5e47fcb826ddcf667561"),
    asset("onnx/embed_tokens_q4.onnx_data", 37_286_384, "f54a51e234b509b64c3a03bb79e1149fba7e2eba6c2d9c222f18883379e1f5d8"),
    asset("onnx/language_model_q4.onnx", 274_572, "b39d03d3f8b943b9e60c6fce3fb41191dbc1df4589f913291db1e214eef669b1"),
    asset("onnx/language_model_q4.onnx_data", 204_456_572, "2c029dc0acf48752473d8c74c72b5ceaaad76b9886fe106eaf2022142d5b5d5e"),
    asset("onnx/conditional_decoder_q4.onnx", 2_179_022, "dccb7a6cea3472dc7f7d070eeb70ade18e6327fb4ec61a3d62cf211bfed90ea2"),
    asset("onnx/conditional_decoder_q4.onnx_data", 246_397_384, "b5c5317e0b79a1a19dd3d5e2b2091ea06b15716716ab801a54eaeb906c6971ec"),
];

/// Language model input ids that start and stop the speech, and the one the decoder reads
/// as silence (appended so the last word isn't cut).
const START_SPEECH: i64 = 6561;
const STOP_SPEECH: i64 = 6562;
const SILENCE: i64 = 4299;
const KV_HEADS: i64 = 16;
const HEAD_DIM: i64 = 64;
const REPETITION_PENALTY: f32 = 1.2;
/// The longest reference clip used (seconds): more only makes every line slower.
pub const REFERENCE_SECONDS: f64 = 10.0;

pub fn model_dir() -> Option<&'static Path> {
    crate::voice::chatterbox_dir()
}

fn present(dir: &Path, a: &Asset) -> bool {
    std::fs::metadata(dir.join(a.name())).is_ok_and(|m| m.len() == a.size)
}

/// Everything the model needs is in `dir` (sizes are checked; hashes were checked on
/// download), and the ONNX Runtime library is in `runtime_dir`.
pub fn installed(dir: &Path, runtime_dir: &Path) -> bool {
    crate::kokoro::runtime_present(runtime_dir) && ASSETS.iter().all(|a| present(dir, a))
}

/// What a full download transfers (the runtime library is counted if it is missing).
pub fn total_size() -> u64 {
    ASSETS.iter().map(|a| a.size).sum::<u64>() + crate::kokoro::RUNTIME_DOWNLOAD
}

/// Bytes still to download.
pub fn download_size(dir: &Path, runtime_dir: &Path) -> u64 {
    if !crate::kokoro::supported() {
        return 0;
    }
    let runtime = if crate::kokoro::runtime_present(runtime_dir) { 0 } else { crate::kokoro::RUNTIME_DOWNLOAD };
    ASSETS.iter().filter(|a| !present(dir, a)).map(|a| a.size).sum::<u64>() + runtime
}

/// Download what is missing, resuming and verifying like the Kokoro download.
/// `progress(done_bytes, total_bytes)`.
pub fn download(dir: &Path, runtime_dir: &Path, cancel: &AtomicBool, mut progress: impl FnMut(u64, u64)) -> Result<()> {
    if !crate::kokoro::supported() {
        bail!("{}", crate::kokoro::UNSUPPORTED);
    }
    std::fs::create_dir_all(dir)?;
    let runtime_left = if crate::kokoro::runtime_present(runtime_dir) { 0 } else { crate::kokoro::RUNTIME_DOWNLOAD };
    let total = ASSETS.iter().map(|a| a.size).sum::<u64>() + runtime_left;
    let mut done = 0u64;
    for a in ASSETS {
        if present(dir, a) {
            done += a.size;
            progress(done, total);
            continue;
        }
        let url = format!("https://huggingface.co/ResembleAI/chatterbox-turbo-ONNX/resolve/{REVISION}/{}", a.path);
        crate::fetch::download_file(&url, &dir.join(a.name()), a.size, a.sha256, cancel, |n| {
            done += n;
            progress(done.min(total), total);
        })?;
    }
    if runtime_left > 0 {
        crate::kokoro::download_runtime(runtime_dir, cancel, |n| {
            done += n;
            progress(done.min(total), total);
        })?;
    }
    Ok(())
}

/// What the speech encoder makes of a reference clip: everything the voice needs.
struct Conditioning {
    /// Reference features for the language model, `[1, n, 1024]`.
    features: Vec<f32>,
    /// The clip as speech tokens (prepended for the decoder).
    prompt: Vec<i64>,
    /// `[1, 192]`
    speaker: Vec<f32>,
    /// `[1, n, 80]`
    mel: Vec<f32>,
    mel_len: usize,
}

/// The loaded model. Not thread-safe to run concurrently; keep one behind a mutex.
pub struct Chatterbox {
    encoder: ort::session::Session,
    embed: ort::session::Session,
    language: ort::session::Session,
    decoder: ort::session::Session,
    tokenizer: Tokenizer,
    /// Each layer's cache input and the output that feeds it next time.
    cache: Vec<(String, String)>,
    /// Reference clips already encoded, by content.
    voices: HashMap<u64, std::sync::Arc<Conditioning>>,
}

fn err(e: ort::Error) -> anyhow::Error {
    anyhow::anyhow!("voice model: {e}")
}

impl Chatterbox {
    pub fn load(dir: &Path, runtime_dir: &Path) -> Result<Self> {
        if !crate::kokoro::supported() {
            bail!("{}", crate::kokoro::UNSUPPORTED);
        }
        if !installed(dir, runtime_dir) {
            bail!("the Chatterbox voice isn't downloaded yet");
        }
        let env = crate::kokoro::environment(runtime_dir)?;
        let session = |name: &str| -> Result<ort::session::Session> {
            ort::session::Session::builder(env)
                .map_err(err)?
                .commit_from_file(dir.join(format!("{name}_q4.onnx")))
                .map_err(|e| anyhow::anyhow!("loading the {name} model: {e}"))
        };
        let language = session("language_model")?;
        let cache = language
            .inputs()
            .iter()
            .filter_map(|i| i.name().strip_prefix("past_key_values.").map(|rest| (i.name().to_string(), format!("present.{rest}"))))
            .collect::<Vec<_>>();
        if cache.is_empty() {
            bail!("the language model has no cache inputs: the files don't match this version of Studio");
        }
        Ok(Self {
            encoder: session("speech_encoder")?,
            embed: session("embed_tokens")?,
            language,
            decoder: session("conditional_decoder")?,
            tokenizer: Tokenizer::from_json(&std::fs::read(dir.join("tokenizer.json")).context("reading the tokenizer")?)?,
            cache,
            voices: HashMap::new(),
        })
    }

    fn conditioning(&mut self, reference: &[f32]) -> Result<std::sync::Arc<Conditioning>> {
        let mut h = crate::cache::Fnv::default();
        for s in reference {
            h.write(&s.to_le_bytes());
        }
        let key = h.finish();
        if let Some(c) = self.voices.get(&key) {
            return Ok(c.clone());
        }
        let n = reference.len().min((REFERENCE_SECONDS * f64::from(SAMPLE_RATE)) as usize);
        if n < SAMPLE_RATE as usize / 2 {
            bail!("the reference clip is too short: use at least a few seconds of clear speech");
        }
        let audio = ort::value::Tensor::from_array(([1usize, n], reference[..n].to_vec())).map_err(err)?;
        let out = self.encoder.run(vec![("audio_values", audio.into_dyn())]).map_err(err)?;
        let floats = |name: &str| -> Result<(Vec<i64>, Vec<f32>)> {
            let (shape, data) = out[name].try_extract_tensor::<f32>().map_err(err)?;
            Ok((shape.to_vec(), data.to_vec()))
        };
        let (_, features) = floats("audio_features")?;
        let (ms, mel) = floats("speaker_features")?;
        let (_, speaker) = floats("speaker_embeddings")?;
        let (_, prompt) = out["audio_tokens"].try_extract_tensor::<i64>().map_err(err)?;
        let c = std::sync::Arc::new(Conditioning {
            features,
            prompt: prompt.to_vec(),
            speaker,
            mel_len: *ms.get(1).context("odd speaker features")? as usize,
            mel,
        });
        if self.voices.len() > 4 {
            self.voices.clear();
        }
        self.voices.insert(key, c.clone());
        Ok(c)
    }

    /// Speak `text` in the voice of `reference` (24 kHz mono samples): 24 kHz mono samples,
    /// with whatever quiet the model leaves around the speech.
    pub fn speak(&mut self, text: &str, reference: &[f32]) -> Result<Vec<f32>> {
        let voice = self.conditioning(reference)?;
        let ids = self.tokenizer.encode(text)?;
        let longest = (ids.len() * 14 + 80).min(1024);
        let mut generated: Vec<i64> = vec![START_SPEECH];
        let mut feed = ids;
        let mut seen = 0usize;
        let mut past: Vec<DynValue> = Vec::new();
        for step in 0..longest {
            let tokens = ort::value::Tensor::from_array(([1usize, feed.len()], feed.clone())).map_err(err)?;
            let out = self.embed.run(vec![("input_ids", tokens.into_dyn())]).map_err(err)?;
            let (_, text_embeds) = out["inputs_embeds"].try_extract_tensor::<f32>().map_err(err)?;
            let mut embeds = Vec::new();
            if step == 0 {
                embeds.extend_from_slice(&voice.features);
            }
            embeds.extend_from_slice(text_embeds);
            let rows = embeds.len() / 1024;
            drop(out);
            seen += rows;
            let positions: Vec<i64> = if step == 0 { (0..rows as i64).collect() } else { vec![seen as i64 - 1] };
            if step == 0 {
                past = self
                    .cache
                    .iter()
                    .map(|_| Ok(ort::value::Tensor::from_array(([1usize, KV_HEADS as usize, 0, HEAD_DIM as usize], Vec::<f32>::new())).map_err(err)?.into_dyn()))
                    .collect::<Result<_>>()?;
            }
            let mut inputs: Vec<(String, DynValue)> = vec![
                ("inputs_embeds".into(), ort::value::Tensor::from_array(([1usize, rows, 1024], embeds)).map_err(err)?.into_dyn()),
                ("attention_mask".into(), ort::value::Tensor::from_array(([1usize, seen], vec![1i64; seen])).map_err(err)?.into_dyn()),
                ("position_ids".into(), ort::value::Tensor::from_array(([1usize, positions.len()], positions)).map_err(err)?.into_dyn()),
            ];
            inputs.extend(self.cache.iter().map(|(name, _)| name.clone()).zip(past.drain(..)));
            let mut out = self.language.run(inputs).map_err(err)?;
            let next = {
                let (shape, logits) = out["logits"].try_extract_tensor::<f32>().map_err(err)?;
                let vocab = *shape.last().context("odd logits")? as usize;
                let mut last = logits[logits.len() - vocab..].to_vec();
                // Tokens already spoken are made less likely, so the voice doesn't loop.
                let mut seen_tokens: Vec<i64> = generated.clone();
                seen_tokens.sort_unstable();
                seen_tokens.dedup();
                for t in seen_tokens {
                    if let Some(v) = last.get_mut(t as usize) {
                        *v = if *v < 0.0 { *v * REPETITION_PENALTY } else { *v / REPETITION_PENALTY };
                    }
                }
                last.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).map_or(STOP_SPEECH, |(i, _)| i as i64)
            };
            generated.push(next);
            if next == STOP_SPEECH {
                break;
            }
            past = self
                .cache
                .iter()
                .map(|(_, present)| out.remove(present.as_str()).with_context(|| format!("the language model gave no {present}")))
                .collect::<Result<_>>()?;
            feed = vec![next];
        }
        let mut speech: Vec<i64> = voice.prompt.clone();
        speech.extend(generated.iter().skip(1).filter(|&&t| t != STOP_SPEECH));
        speech.extend([SILENCE; 3]);
        let tensors = vec![
            ("speech_tokens", ort::value::Tensor::from_array(([1usize, speech.len()], speech)).map_err(err)?.into_dyn()),
            ("speaker_embeddings", ort::value::Tensor::from_array(([1usize, voice.speaker.len()], voice.speaker.clone())).map_err(err)?.into_dyn()),
            ("speaker_features", ort::value::Tensor::from_array(([1usize, voice.mel_len, 80], voice.mel.clone())).map_err(err)?.into_dyn()),
        ];
        let out = self.decoder.run(tensors).map_err(err)?;
        let (_, wave) = out["waveform"].try_extract_tensor::<f32>().map_err(err)?;
        Ok(wave.to_vec())
    }
}

/// A reference clip as the model reads it: mono 24 kHz, at most [`REFERENCE_SECONDS`].
/// Any WAV works; other formats are converted with ffmpeg when it is available.
pub fn prepare_reference(src: &Path, dst: &Path) -> Result<f64> {
    let mut samples = match crate::voice::read_wav(src) {
        Ok(s) => s,
        Err(wav_error) => {
            let ffmpeg = crate::ffmpeg_path().with_context(|| format!("{wav_error:#} (and ffmpeg isn't available to convert it)"))?;
            let tmp = dst.with_extension("convert.wav");
            let mut cmd = std::process::Command::new(ffmpeg);
            cmd.args(["-y", "-loglevel", "error", "-i"])
                .arg(src)
                .args(["-t", &format!("{REFERENCE_SECONDS}"), "-ac", "1", "-ar", &SAMPLE_RATE.to_string(), "-c:a", "pcm_s16le"])
                .arg(&tmp);
            let status = crate::export::quiet(&mut cmd).status().context("running ffmpeg")?;
            if !status.success() {
                let _ = std::fs::remove_file(&tmp);
                bail!("couldn't read {} as audio", src.display());
            }
            let s = crate::voice::read_wav(&tmp);
            let _ = std::fs::remove_file(&tmp);
            s?
        }
    };
    samples.truncate((REFERENCE_SECONDS * f64::from(SAMPLE_RATE)) as usize);
    // Quiet leading and trailing room only makes the model's prompt longer.
    let trimmed = crate::kokoro::trim_silence(&samples).to_vec();
    if trimmed.len() < SAMPLE_RATE as usize / 2 {
        bail!("the clip has less than half a second of speech");
    }
    if let Some(dir) = dst.parent() {
        std::fs::create_dir_all(dir)?;
    }
    crate::kokoro::write_wav(dst, &trimmed)?;
    Ok(trimmed.len() as f64 / f64::from(SAMPLE_RATE))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assets_are_pinned_and_flat() {
        assert!(ASSETS.iter().all(|a| a.sha256.len() == 64 && a.sha256.chars().all(|c| c.is_ascii_hexdigit()) && a.size > 0));
        // Graphs sit next to their weights.
        for a in ASSETS.iter().filter(|a| a.path.ends_with(".onnx_data")) {
            let graph = a.path.strip_suffix("_data").unwrap();
            assert!(ASSETS.iter().any(|b| b.path == graph), "{graph}");
        }
        assert_eq!(ASSETS.iter().map(Asset::name).collect::<std::collections::HashSet<_>>().len(), ASSETS.len());
        // Roughly 725 MB plus the shared runtime, the size the settings page promises.
        let mb = ASSETS.iter().map(|a| a.size).sum::<u64>() / 1_000_000;
        assert!((700..760).contains(&mb), "{mb} MB");
    }

    /// With the real model files (set `CHATTERBOX_DIR`, `CHATTERBOX_RUNTIME` and
    /// `CHATTERBOX_REFERENCE`; skipped otherwise): a sentence is spoken, stops by itself,
    /// and comes out as real speech. `CHATTERBOX_OUT` keeps the WAV for listening.
    #[test]
    fn speaks_with_the_real_model() {
        let var = |k: &str| std::env::var_os(k).map(std::path::PathBuf::from);
        let (Some(models), Some(runtime), Some(clip)) = (var("CHATTERBOX_DIR"), var("CHATTERBOX_RUNTIME"), var("CHATTERBOX_REFERENCE")) else { return };
        let reference = crate::voice::read_wav(&clip).unwrap();
        let mut model = Chatterbox::load(&models, &runtime).unwrap();
        let started = std::time::Instant::now();
        let wave = model.speak("If r is ten, c is twenty. They line up exactly.", &reference).unwrap();
        let took = started.elapsed().as_secs_f64();
        let seconds = wave.len() as f64 / f64::from(SAMPLE_RATE);
        eprintln!("spoke {seconds:.2} s of audio in {took:.1} s");
        assert!((1.5..9.0).contains(&seconds), "a two-sentence line is a few seconds: {seconds}");
        let rms = (wave.iter().map(|s| s * s).sum::<f32>() / wave.len() as f32).sqrt();
        assert!(rms > 0.02, "not silence: {rms}");
        let trimmed = crate::kokoro::trim_silence(&wave);
        assert!(trimmed.len() as f64 > wave.len() as f64 * 0.6, "mostly speech");
        if let Some(out) = var("CHATTERBOX_OUT") {
            crate::kokoro::write_wav(&out, &wave).unwrap();
        }
    }

    /// The reference clip is trimmed and capped; a clip with no speech is refused.
    #[test]
    fn references_are_prepared_for_the_model() {
        let dir = std::env::temp_dir().join(format!("studio-reference-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (src, dst) = (dir.join("in.wav"), dir.join("out.wav"));
        let rate = SAMPLE_RATE as usize;
        let mut clip = vec![0.0f32; rate];
        clip.extend((0..rate * 14).map(|i| 0.3 * (i as f32 * 0.07).sin()));
        clip.extend(vec![0.0f32; rate]);
        crate::kokoro::write_wav(&src, &clip).unwrap();
        let seconds = prepare_reference(&src, &dst).unwrap();
        assert!((seconds - REFERENCE_SECONDS + 1.0).abs() < 0.2, "the quiet start goes, the rest is capped: {seconds}");
        assert!((crate::voice::wav_seconds(&dst).unwrap() - seconds).abs() < 0.01);
        crate::kokoro::write_wav(&src, &vec![0.0; rate * 3]).unwrap();
        assert!(prepare_reference(&src, &dst).unwrap_err().to_string().contains("speech"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
