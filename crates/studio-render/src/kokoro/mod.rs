//! Kokoro-82M (hexgrad, Apache-2.0) narration on this computer: ONNX Runtime on the CPU,
//! with phonemes from a port of misaki ([`g2p`]). The model, voices and dictionaries are
//! downloaded once (about 105 MB, plus the ~6 MB ONNX Runtime DLL) into a folder the app
//! chooses, pinned by SHA-256.

pub mod espeak;
mod g2p;

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

pub use g2p::G2p;

pub const SAMPLE_RATE: u32 = 24_000;
/// Kokoro's context: 512 tokens including the two pads.
const MAX_TOKENS: usize = 510;

const HF: &str = "https://huggingface.co/onnx-community/Kokoro-82M-v1.0-ONNX/resolve/1939ad2a8e416c0acfeecc08a694d14ef25f2231";
const MISAKI: &str = "https://raw.githubusercontent.com/hexgrad/misaki/fba1236595f2d2bf21d414ba6e57d25256afada3/misaki/data";

/// A downloadable file: local name, URL, size, SHA-256.
pub struct Asset {
    pub name: &'static str,
    base: &'static str,
    path: &'static str,
    pub size: u64,
    sha256: &'static str,
}

const fn asset(name: &'static str, base: &'static str, path: &'static str, size: u64, sha256: &'static str) -> Asset {
    Asset { name, base, path, size, sha256 }
}

const VOICE_SIZE: u64 = 522_240;

/// Which model file: the full-precision original or the 8-bit one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Quality {
    /// int8, 92 MB: the default. In listening tests it was indistinguishable from High.
    #[default]
    Standard,
    /// fp32, 326 MB: the original model.
    High,
}

impl Quality {
    /// In the order Settings lists them.
    pub const ALL: [Quality; 2] = [Quality::Standard, Quality::High];

    pub fn model(self) -> &'static Asset {
        match self {
            Quality::High => &MODELS[0],
            Quality::Standard => &MODELS[1],
        }
    }
}

const MODELS: [Asset; 2] = [
    asset("model.onnx", HF, "onnx/model.onnx", 325_532_232, "8fbea51ea711f2af382e88c833d9e288c6dc82ce5e98421ea61c058ce21a34cb"),
    asset("model_quantized.onnx", HF, "onnx/model_quantized.onnx", 92_361_116, "fbae9257e1e05ffc727e951ef9b9c98418e6d79f1c9b6b13bd59f5c9028a1478"),
];

/// Voices and dictionaries, shared by both models.
pub const ASSETS: &[Asset] = &[
    asset("us_gold.json", MISAKI, "us_gold.json", 3_000_469, "dc414872a49a28ae6c141463d502fd945f3b2fde040484fdc47d00cc4612686f"),
    asset("us_silver.json", MISAKI, "us_silver.json", 3_099_517, "de8f67be911bb6c659187b4a65fd966b6a30e56350e0f790d763210b053ac475"),
    asset("gb_gold.json", MISAKI, "gb_gold.json", 2_838_552, "29e62f4b60261c88f7f3c2c7811ca3825978948090b72d2b27d565b729282f71"),
    asset("gb_silver.json", MISAKI, "gb_silver.json", 3_663_898, "48131e2d92ccc41655f4543e87e0f938e71463eb5a54be7f0693bb712ebb6bce"),
    asset("af_heart.bin", HF, "voices/af_heart.bin", VOICE_SIZE, "d583ccff3cdca2f7fae535cb998ac07e9fcb90f09737b9a41fa2734ec44a8f0b"),
    asset("af_bella.bin", HF, "voices/af_bella.bin", VOICE_SIZE, "f69d836209b78eb8c66e75e3cda491e26ea838a3674257e9d4e5703cbaf55c8b"),
    asset("af_nicole.bin", HF, "voices/af_nicole.bin", VOICE_SIZE, "cd2191ab31b914ed7b318416b0e4440fdf392ddad9106a060819aa600a64f59a"),
    asset("af_sarah.bin", HF, "voices/af_sarah.bin", VOICE_SIZE, "4409fbc125afabacc615d94db5398d847006a737b0247d6892b7a9a0007a2f0a"),
    asset("am_michael.bin", HF, "voices/am_michael.bin", VOICE_SIZE, "1d1f21dd8da39c30705cd4c75d039d265e9bc4a2a93ed09bc9e1b1225eb95ba1"),
    asset("am_fenrir.bin", HF, "voices/am_fenrir.bin", VOICE_SIZE, "c27989f741f7ee34d273a39d8a595cc0837d35f5ced9a29b7cc162614616df43"),
    asset("am_puck.bin", HF, "voices/am_puck.bin", VOICE_SIZE, "fcf73c989033e9233e0b98713eca600c8c74dcc1614b37009d5450ff4a2274a0"),
    asset("bf_emma.bin", HF, "voices/bf_emma.bin", VOICE_SIZE, "669fe0647f9dd04fcab92f1439a40eeb4c8b4ab1f82e4996fe3d918ce4a63b73"),
    asset("bf_isabella.bin", HF, "voices/bf_isabella.bin", VOICE_SIZE, "3754352c4aaa46d17f27654ab7518d65b62ad6163a0f55a5f4330c2da2c4e94f"),
    asset("bm_george.bin", HF, "voices/bm_george.bin", VOICE_SIZE, "c4b235a4c1f2cd3b939fed08b899ce9385638b763f7b73a59616c4fc9bd6c9bc"),
    asset("bm_fable.bin", HF, "voices/bm_fable.bin", VOICE_SIZE, "f889083196807b4adb15e9204252165f503b8d33d3982e681c52443c49d798f1"),
];

/// Voice id and the name shown in Settings. `b*` voices are British.
pub const VOICES: &[(&str, &str)] = &[
    ("af_heart", "Heart · US"),
    ("af_bella", "Bella · US"),
    ("af_nicole", "Nicole · US"),
    ("af_sarah", "Sarah · US"),
    ("am_michael", "Michael · US"),
    ("am_fenrir", "Fenrir · US"),
    ("am_puck", "Puck · US"),
    ("bf_emma", "Emma · UK"),
    ("bf_isabella", "Isabella · UK"),
    ("bm_george", "George · UK"),
    ("bm_fable", "Fable · UK"),
];
pub const DEFAULT_VOICE: &str = "af_heart";

pub fn is_voice(id: &str) -> bool {
    VOICES.iter().any(|(v, _)| *v == id)
}

fn present(dir: &Path, a: &Asset) -> bool {
    std::fs::metadata(dir.join(a.name)).is_ok_and(|m| m.len() == a.size)
}

/// Bytes still to download for `quality`.
pub fn download_size(dir: &Path, quality: Quality) -> u64 {
    let runtime = if runtime::present(dir) { 0 } else { runtime::DOWNLOAD };
    ASSETS.iter().chain([quality.model()]).filter(|a| !present(dir, a)).map(|a| a.size).sum::<u64>() + runtime
}

/// Everything `quality` needs is downloaded (sizes are checked; hashes were checked on
/// download).
pub fn installed(dir: &Path, quality: Quality) -> bool {
    runtime::present(dir) && ASSETS.iter().chain([quality.model()]).all(|a| present(dir, a))
}

/// Everything but the runtime is here: a voice downloaded before the runtime moved out
/// of the program. Fetching it is a quick (~6 MB) top-up.
pub fn needs_runtime_only(dir: &Path, quality: Quality) -> bool {
    !runtime::present(dir) && ASSETS.iter().chain([quality.model()]).all(|a| present(dir, a))
}

/// The quality to speak with: `wanted` if downloaded, else the other one if that is.
pub fn usable(dir: &Path, wanted: Quality) -> Option<Quality> {
    std::iter::once(wanted).chain(Quality::ALL).find(|&q| installed(dir, q))
}

/// Delete the model file `keep` doesn't use, once `keep` is fully installed, so only one
/// model (the bulk of the voice download) stays on disk. Returns the bytes freed.
pub fn remove_other_models(dir: &Path, keep: Quality) -> u64 {
    if !installed(dir, keep) {
        return 0;
    }
    let mut freed = 0;
    for q in Quality::ALL.into_iter().filter(|&q| q != keep) {
        let path = dir.join(q.model().name);
        if let Ok(meta) = std::fs::metadata(&path)
            && std::fs::remove_file(&path).is_ok()
        {
            freed += meta.len();
        }
    }
    freed
}

/// Download what `quality` is missing. `progress(done_bytes, total_bytes)`.
pub fn download(dir: &Path, quality: Quality, mut progress: impl FnMut(u64, u64)) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let runtime_left = if runtime::present(dir) { 0 } else { runtime::DOWNLOAD };
    let total: u64 = ASSETS.iter().chain([quality.model()]).map(|a| a.size).sum::<u64>() + runtime_left;
    let mut done = 0u64;
    for a in ASSETS.iter().chain([quality.model()]) {
        let path = dir.join(a.name);
        if std::fs::metadata(&path).is_ok_and(|m| m.len() == a.size) {
            done += a.size;
            progress(done, total);
            continue;
        }
        let url = format!("{}/{}", a.base, a.path);
        let resp = ureq::get(&url).call().with_context(|| format!("downloading {}", a.name))?;
        let mut reader = resp.into_reader();
        let part = dir.join(format!("{}.part", a.name));
        let mut file = std::fs::File::create(&part)?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 1 << 16];
        loop {
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            file.write_all(&buf[..n])?;
            done += n as u64;
            progress(done, total);
        }
        drop(file);
        let got = format!("{:x}", hasher.finalize());
        if got != a.sha256 {
            let _ = std::fs::remove_file(&part);
            bail!("{} failed its checksum", a.name);
        }
        std::fs::rename(&part, &path)?;
    }
    if runtime_left > 0 {
        runtime::download(dir, |n| {
            done += n;
            progress(done, total);
        })?;
    }
    Ok(())
}

/// ONNX Runtime (CPU build), loaded at run time from the voice folder. Only
/// `onnxruntime.dll` is fetched out of Microsoft's 83 MB release zip (about 6 MB).
mod runtime {
    use super::*;
    use crate::fetch::ZipEntry;

    const DLL: ZipEntry = ZipEntry {
        url: "https://github.com/microsoft/onnxruntime/releases/download/v1.31.1/onnxruntime-win-x64-1.31.1.zip",
        offset: 221_047,
        compressed: 6_178_062,
        size: 17_253_728,
        sha256: "cb2fa518b18356617f6f9f460c012501da2819ec7e5c1d80bd8f5de20492db0c",
        name_suffix: "lib/onnxruntime.dll",
    };
    /// What a download transfers, for progress.
    pub const DOWNLOAD: u64 = DLL.compressed;

    pub const FILE: &str = "onnxruntime.dll";

    pub fn present(dir: &Path) -> bool {
        DLL.present(&dir.join(FILE))
    }

    pub fn download(dir: &Path, progress: impl FnMut(u64)) -> Result<()> {
        DLL.download(&dir.join(FILE), progress).context("downloading the voice runtime")
    }
}

/// ONNX Runtime allows one environment per process; share it between model loads.
fn environment(dir: &Path) -> Result<&'static ort::environment::Environment> {
    static ENV: std::sync::OnceLock<ort::environment::Environment> = std::sync::OnceLock::new();
    if let Some(env) = ENV.get() {
        return Ok(env);
    }
    let builder = ort::init_from(dir.join(runtime::FILE)).map_err(|e| anyhow::anyhow!("loading the voice runtime: {e}"))?;
    let env = builder.with_name("manim-studio").with_telemetry(false).build()?;
    Ok(ENV.get_or_init(|| env))
}

/// The loaded model. Not thread-safe to run concurrently; keep one behind a mutex.
pub struct Kokoro {
    pub quality: Quality,
    dir: PathBuf,
    session: ort::session::Session,
    vocab: HashMap<char, i64>,
    us: Option<G2p>,
    gb: Option<G2p>,
    voices: HashMap<String, Vec<f32>>,
}

impl Kokoro {
    pub fn load(dir: &Path, quality: Quality) -> Result<Self> {
        if !installed(dir, quality) {
            bail!("the natural voice isn't downloaded yet");
        }
        let session = ort::session::Session::builder(environment(dir)?)
            .map_err(|e| anyhow::anyhow!("{e}"))?
            .commit_from_file(dir.join(quality.model().name))
            .map_err(|e| anyhow::anyhow!("loading the voice model: {e}"))?;
        let vocab: HashMap<String, i64> = serde_json::from_str(include_str!("vocab.json"))?;
        let vocab = vocab.into_iter().filter_map(|(k, v)| Some((k.chars().next()?, v))).collect();
        Ok(Self { quality, dir: dir.to_path_buf(), session, vocab, us: None, gb: None, voices: HashMap::new() })
    }

    fn g2p(&mut self, british: bool) -> Result<&G2p> {
        let slot = if british { &mut self.gb } else { &mut self.us };
        if slot.is_none() {
            *slot = Some(G2p::load(&self.dir, british)?);
        }
        Ok(slot.as_ref().expect("loaded"))
    }

    pub fn phonemes(&mut self, text: &str, voice: &str) -> Result<String> {
        Ok(self.g2p(voice.starts_with('b'))?.phonemize(text))
    }

    fn voice(&mut self, id: &str) -> Result<&[f32]> {
        if !self.voices.contains_key(id) {
            if !is_voice(id) {
                bail!("unknown voice {id}");
            }
            let b = std::fs::read(self.dir.join(format!("{id}.bin")))?;
            let v = b.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect();
            self.voices.insert(id.to_string(), v);
        }
        Ok(&self.voices[id])
    }

    /// Speak `text`: 24 kHz mono samples.
    pub fn speak(&mut self, text: &str, voice: &str, speed: f32) -> Result<Vec<f32>> {
        let ps = self.phonemes(text, voice)?;
        let ids: Vec<i64> = ps.chars().filter_map(|c| self.vocab.get(&c).copied()).collect();
        // Each piece comes back with its own silence around it; trim that and join the
        // pieces with one natural breath, so narration flows without dead air.
        let mut out: Vec<f32> = Vec::new();
        for chunk in split(&ids, &self.vocab) {
            let wave = self.run(&chunk, voice, speed)?;
            let piece = trim_silence(&wave);
            if !out.is_empty() && !piece.is_empty() {
                out.extend(std::iter::repeat_n(0.0, (BREATH * SAMPLE_RATE as f32) as usize));
            }
            out.extend_from_slice(piece);
        }
        Ok(out)
    }

    fn run(&mut self, ids: &[i64], voice: &str, speed: f32) -> Result<Vec<f32>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let row = ids.len().min(MAX_TOKENS - 1);
        let style: Vec<f32> = self.voice(voice)?[row * 256..(row + 1) * 256].to_vec();
        let mut input = Vec::with_capacity(ids.len() + 2);
        input.push(0);
        input.extend_from_slice(ids);
        input.push(0);
        let n = input.len();
        let err = |e: ort::Error| anyhow::anyhow!("voice model: {e}");
        let inputs = ort::inputs![
            "input_ids" => ort::value::Tensor::from_array(([1usize, n], input)).map_err(err)?,
            "style" => ort::value::Tensor::from_array(([1usize, 256], style)).map_err(err)?,
            "speed" => ort::value::Tensor::from_array(([1usize], vec![speed.clamp(0.5, 2.0)])).map_err(err)?,
        ];
        let outputs = self.session.run(inputs).map_err(err)?;
        let (_, wave) = outputs[0].try_extract_tensor::<f32>().map_err(err)?;
        Ok(wave.to_vec())
    }
}

/// Split token ids into pieces Kokoro can take, preferring sentence ends, then commas, then
/// spaces.
fn split(ids: &[i64], vocab: &HashMap<char, i64>) -> Vec<Vec<i64>> {
    let id = |c: char| vocab.get(&c).copied().unwrap_or(-1);
    let strong: Vec<i64> = ".!?".chars().map(id).collect();
    let weak: Vec<i64> = ",;:—…".chars().map(id).collect();
    let space = id(' ');
    let mut out = Vec::new();
    let mut rest = ids;
    while rest.len() > MAX_TOKENS {
        let window = &rest[..MAX_TOKENS];
        let cut = [&strong, &weak]
            .iter()
            .find_map(|set| window.iter().rposition(|t| set.contains(t)).filter(|&i| i > MAX_TOKENS / 4))
            .or_else(|| window.iter().rposition(|&t| t == space).filter(|&i| i > 0))
            .map_or(MAX_TOKENS, |i| i + 1);
        out.push(rest[..cut].to_vec());
        rest = &rest[cut..];
        while rest.first() == Some(&space) {
            rest = &rest[1..];
        }
    }
    if !rest.is_empty() {
        out.push(rest.to_vec());
    }
    out
}

/// Pause between the pieces of a long line.
const BREATH: f32 = 0.28;
/// Kept before the first and after the last sound of a piece, so words aren't clipped.
const EDGE: f32 = 0.06;

/// The samples between the first and last audible sound, with a short edge kept.
fn trim_silence(wave: &[f32]) -> &[f32] {
    const THRESHOLD: f32 = 0.012;
    let edge = (EDGE * SAMPLE_RATE as f32) as usize;
    match (wave.iter().position(|s| s.abs() > THRESHOLD), wave.iter().rposition(|s| s.abs() > THRESHOLD)) {
        (Some(a), Some(b)) => &wave[a.saturating_sub(edge)..(b + edge + 1).min(wave.len())],
        _ => &[],
    }
}

/// Write 24 kHz mono samples as 16-bit PCM WAV.
pub fn write_wav(path: &Path, samples: &[f32]) -> Result<()> {
    let data = (samples.len() * 2) as u32;
    let mut b = Vec::with_capacity(44 + data as usize);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    b.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data.to_le_bytes());
    for s in samples {
        b.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    std::fs::write(path, b)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> Option<PathBuf> {
        let d = PathBuf::from(std::env::var_os("APPDATA")?).join("ManimStudio/Manim Studio/data/voices/kokoro");
        usable(&d, Quality::High).is_some().then_some(d)
    }

    #[test]
    fn trims_silence_but_keeps_edges() {
        let mut w = vec![0.0f32; 24_000];
        w.extend(vec![0.5f32; 2_400]);
        w.extend(vec![0.0f32; 24_000]);
        let t = trim_silence(&w);
        let edge = (EDGE * SAMPLE_RATE as f32) as usize;
        assert_eq!(t.len(), 2_400 + 2 * edge);
        assert!(trim_silence(&[0.0; 100]).is_empty());
    }

    #[test]
    fn splits_long_input() {
        let vocab: HashMap<char, i64> = [(' ', 16), ('.', 4), ('a', 43)].into();
        let ids: Vec<i64> = std::iter::repeat_n([43, 43, 43, 4, 16], 300).flatten().collect();
        let parts = split(&ids, &vocab);
        assert!(parts.len() > 1 && parts.iter().all(|p| p.len() <= MAX_TOKENS));
        assert_eq!(parts.iter().map(Vec::len).sum::<usize>() + parts.len() - 1, ids.len());
    }

    /// Speed of each downloaded model: `cargo test --release -p studio-render compare_qualities -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn compare_qualities() {
        let Some(d) = dir() else { return };
        let text = "If you double a pizza's radius, do you get twice the pizza? Let's find out. We cut the circle into thin slices and lay them side by side. The slices make a shape that looks more and more like a rectangle.";
        for q in Quality::ALL.into_iter().filter(|&q| installed(&d, q)) {
            let mut k = Kokoro::load(&d, q).unwrap();
            k.speak("Warm up.", "af_heart", 1.0).unwrap();
            let t = std::time::Instant::now();
            let wave = k.speak(text, "af_heart", 1.0).unwrap();
            let secs = wave.len() as f32 / SAMPLE_RATE as f32;
            eprintln!("{q:?}: {secs:.1}s of audio in {:.2}s ({:.0}x real time)", t.elapsed().as_secs_f32(), secs / t.elapsed().as_secs_f32());
            if let Ok(out) = std::env::var("KOKORO_WAV") {
                write_wav(Path::new(&format!("{out}-{q:?}.wav")), &wave).unwrap();
            }
        }
    }

    /// `KOKORO_RUNTIME=<dir>`: fetch just the ONNX Runtime DLL there and check it.
    #[test]
    fn downloads_runtime() {
        let Some(d) = std::env::var_os("KOKORO_RUNTIME") else { return };
        let d = PathBuf::from(d);
        std::fs::create_dir_all(&d).unwrap();
        let mut got = 0;
        runtime::download(&d, |n| got += n).unwrap();
        assert!(runtime::present(&d) && got > 0);
    }

    /// `KOKORO_DOWNLOAD=<dir>`: fetch what is missing there and check it.
    #[test]
    fn downloads_missing_files() {
        let Some(d) = std::env::var_os("KOKORO_DOWNLOAD") else { return };
        let d = PathBuf::from(d);
        let mut calls = 0;
        download(&d, Quality::Standard, |done, total| {
            calls += 1;
            assert!(done <= total);
        })
        .unwrap();
        assert!(installed(&d, Quality::Standard) && calls > 0);
    }

    /// Needs the downloaded model (skipped otherwise).
    #[test]
    fn speaks_with_kokoro() {
        let Some(d) = dir() else { return };
        let q = usable(&d, Quality::High).unwrap();
        let mut k = Kokoro::load(&d, q).unwrap();
        let ps = k.phonemes("The area of a circle is πr². Let's see why, step by step.", "af_heart").unwrap();
        eprintln!("{ps}");
        assert!(ps.contains("pˈI") && ps.contains("skwˈɛɹd"), "{ps}");
        let t = std::time::Instant::now();
        let wave = k.speak("The area of a circle is pi r squared.", "af_heart", 1.0).unwrap();
        let secs = wave.len() as f32 / SAMPLE_RATE as f32;
        eprintln!("{q:?}: {secs:.2}s of audio in {:?}", t.elapsed());
        assert!((1.0..6.0).contains(&secs));
        if let Ok(out) = std::env::var("KOKORO_WAV") {
            write_wav(Path::new(&out), &wave).unwrap();
        }
    }
}
