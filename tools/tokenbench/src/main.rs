//! Token benchmark for the assistant: for fixed lessons, run build → engine check →
//! repairs with each prompt recipe, and compare tokens against quality.
//!
//! ```sh
//! cargo run -p tokenbench --release -- --dry                      # prompt sizes only, free
//! cargo run -p tokenbench --release -- --cases fractions --variants baseline,example
//! cargo run -p tokenbench --release -- --runs 2                   # everything in bench.json, twice
//! cargo run -p tokenbench --release -- --check <video folder> [s]  # recheck (optionally narrated)
//! ```
//!
//! Storyboards are planned once per case and kept in `tools/tokenbench/plans/`, so every
//! variant and every rerun builds the same storyboard (`--replan` makes new ones). Results
//! go to `target/tokenbench/<time>/`: `results.json` (one record per run, written as it
//! goes), `summary.md` (per-variant means) and `report.html` (thumbnails of every scene, to
//! judge the videos by eye). This spends real plan usage: start with one case and Haiku.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use studio_agent::events::AgentEvent;
use studio_agent::prompt::{self, Recipe, Storyboard};
use studio_agent::turn::{self, EDIT_TOOLS, TurnRequest};
use studio_agent::{Effort, Provider};
use studio_render::preview::{OpenScene, check, load, thumbnail_time};
use studio_render::{Output, style};

const ROOT: &str = env!("CARGO_MANIFEST_DIR");
/// Narration pace used to estimate voice length without synthesizing it.
const WORDS_PER_SECOND: f64 = 2.6;

#[derive(Deserialize)]
struct Bench {
    planner: Planner,
    repair_tries: u32,
    cases: Vec<Case>,
    variants: Vec<Variant>,
}

#[derive(Deserialize)]
struct Planner {
    model: String,
}

#[derive(Deserialize, Clone)]
struct Case {
    id: String,
    request: String,
    seconds: u32,
}

#[derive(Deserialize, Clone)]
struct Variant {
    id: String,
    model: Option<String>,
    effort: Option<String>,
    #[serde(default)]
    recipe: Recipe,
}

#[derive(Serialize, Deserialize, Default, Clone, Copy, Debug)]
struct Usage {
    /// Fresh input, not from or into the cache.
    input: u64,
    cache_write: u64,
    cache_read: u64,
    output: u64,
    /// Model responses (round trips).
    calls: u32,
    cost_usd: f64,
    seconds: f64,
}

impl Usage {
    fn add(&mut self, o: &Usage) {
        self.input += o.input;
        self.cache_write += o.cache_write;
        self.cache_read += o.cache_read;
        self.output += o.output;
        self.calls += o.calls;
        self.cost_usd += o.cost_usd;
        self.seconds += o.seconds;
    }

    /// What counts against a plan in practice: everything but cache reads.
    fn billed(&self) -> u64 {
        self.input + self.cache_write + self.output
    }
}

#[derive(Serialize, Clone)]
struct SceneResult {
    file: String,
    planned_s: u32,
    duration_s: Option<f64>,
    first_ok: bool,
    ok: bool,
    problems: usize,
    /// The first layout problem left, if any.
    problem: Option<String>,
    error: Option<String>,
    lines: usize,
    thumb: Option<String>,
}

#[derive(Serialize, Clone)]
struct RunResult {
    case: String,
    variant: String,
    run: u32,
    build: Usage,
    repair: Usage,
    repairs: u32,
    missing: usize,
    scenes: Vec<SceneResult>,
    error: Option<String>,
}

struct Args {
    config: PathBuf,
    cases: Option<Vec<String>>,
    variants: Option<Vec<String>>,
    runs: u32,
    out: Option<PathBuf>,
    replan: bool,
    dry: bool,
}

fn args() -> Result<Args> {
    let mut a = Args { config: Path::new(ROOT).join("bench.json"), cases: None, variants: None, runs: 1, out: None, replan: false, dry: false };
    let mut it = std::env::args().skip(1);
    let list = |s: String| s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect::<Vec<_>>();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--config" => a.config = it.next().context("--config needs a file")?.into(),
            "--cases" => a.cases = Some(list(it.next().context("--cases needs ids")?)),
            "--variants" => a.variants = Some(list(it.next().context("--variants needs ids")?)),
            "--runs" => a.runs = it.next().context("--runs needs a number")?.parse()?,
            "--out" => a.out = Some(it.next().context("--out needs a folder")?.into()),
            "--replan" => a.replan = true,
            "--dry" => a.dry = true,
            other => bail!("unknown argument {other}"),
        }
    }
    Ok(a)
}

fn model(name: Option<&str>) -> Result<Option<&'static str>> {
    Ok(match name {
        None => None,
        Some("haiku") => Some("haiku"),
        Some("sonnet") => Some("sonnet"),
        Some("opus") => Some("opus"),
        Some(m) => bail!("unknown model {m} (haiku, sonnet, opus)"),
    })
}

fn effort(name: Option<&str>) -> Result<Option<Effort>> {
    Ok(match name {
        None => None,
        Some("low") => Some(Effort::Low),
        Some("medium") => Some(Effort::Medium),
        Some("high") => Some(Effort::High),
        Some("max") => Some(Effort::Max),
        Some(e) => bail!("unknown effort {e}"),
    })
}

struct TurnOut {
    text: String,
    session: Option<String>,
    usage: Usage,
    error: Option<String>,
}

/// Run one CLI turn to the end and total its usage (each response counted once).
fn run_turn(req: TurnRequest) -> Result<TurnOut> {
    let started = Instant::now();
    let t = turn::start(req)?;
    let mut per_message: HashMap<String, studio_agent::events::Tokens> = HashMap::new();
    let mut text = String::new();
    while let Ok(ev) = t.events.recv_blocking() {
        match ev {
            AgentEvent::Usage { message, tokens } => {
                per_message.insert(message, tokens);
            }
            AgentEvent::Text(s) => {
                text.push_str(&s);
                text.push('\n');
            }
            AgentEvent::Done { session, tokens, cost_usd, error } => {
                let mut u = Usage { calls: per_message.len() as u32, cost_usd: cost_usd.unwrap_or(0.0), seconds: started.elapsed().as_secs_f64(), ..Usage::default() };
                for m in per_message.values() {
                    u.input += m.input - m.cache_write;
                    u.cache_write += m.cache_write;
                    u.cache_read += m.cached;
                }
                // Output is only final in the result event (streamed messages report partial counts).
                u.output = tokens.output;
                if per_message.is_empty() {
                    u.input = tokens.input - tokens.cache_write;
                    u.cache_write = tokens.cache_write;
                    u.cache_read = tokens.cached;
                }
                return Ok(TurnOut { text, session, usage: u, error });
            }
            _ => {}
        }
    }
    bail!("the assistant stopped without a result")
}

fn plan_for(case: &Case, bench: &Bench, exe: &Path, replan: bool, work: &Path) -> Result<(Storyboard, Option<Usage>)> {
    let file = Path::new(ROOT).join("plans").join(format!("{}.json", case.id));
    if !replan && let Ok(text) = std::fs::read_to_string(&file) {
        return Ok((serde_json::from_str(&text)?, None));
    }
    let dir = work.join("plans");
    std::fs::create_dir_all(&dir)?;
    let system = dir.join("plan-system.md");
    std::fs::write(&system, prompt::PLAN_SYSTEM)?;
    let p = prompt::plan(&prompt::PlanOptions { request: &case.request, sources: &[], target_seconds: case.seconds, style_name: "Chalk", approach: "", level: "", words_per_second: 2.5, ending: "" });
    let out = run_turn(TurnRequest {
        provider: Provider::Claude,
        exe: exe.to_path_buf(),
        cwd: dir.clone(),
        system_file: system,
        prompt: p,
        model: model(Some(&bench.planner.model))?,
        effort: None,
        resume: None,
        tools: vec![],
        ephemeral: true,
    })?;
    if let Some(e) = out.error {
        bail!("planning {} failed: {e}", case.id);
    }
    let sb = prompt::parse_storyboard(&out.text)?;
    std::fs::create_dir_all(file.parent().unwrap())?;
    std::fs::write(&file, serde_json::to_string_pretty(&sb)?)?;
    Ok((sb, Some(out.usage)))
}

fn prepare(dir: &Path, v: &Variant) -> Result<PathBuf> {
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir.join("scenes"))?;
    std::fs::create_dir_all(dir.join("lib"))?;
    let kit = studio_render::sample::AREA_OF_A_CIRCLE.iter().find(|(p, _)| *p == "lib/kit.ts").map(|(_, b)| *b).context("no kit")?;
    std::fs::write(dir.join("lib/kit.ts"), kit)?;
    let system = dir.join("AGENTS.md");
    std::fs::write(&system, system_prompt(v.recipe))?;
    Ok(system)
}

fn system_prompt(r: Recipe) -> String {
    prompt::build_system_with(&prompt::digest_for(studio_render::TYPES, r.digest), r)
}

fn scene_request(dir: &Path, path: &Path, narration: Option<f64>) -> OpenScene {
    OpenScene { path: path.to_path_buf(), root: dir.to_path_buf(), theme_json: style::json(0).into(), scene: None, globals: vec![], narration, narration_lines: vec![] }
}

/// A scene's narration as the app gives it: estimated sentence lengths.
fn narrated_request(dir: &Path, path: &Path, say: &str) -> OpenScene {
    let lines: Vec<f64> = prompt::sentences(say).iter().map(|s| s.split_whitespace().count() as f64 / WORDS_PER_SECOND).collect();
    let total = studio_render::preview::narration_end(&studio_render::voice::sequential_starts(&lines), &lines);
    OpenScene { narration: Some(total), narration_lines: lines, ..scene_request(dir, path, None) }
}


/// Check every expected scene: (index, path, Ok(problems, duration) | Err(message, line)).
type Checked = Vec<(usize, PathBuf, std::result::Result<(usize, f64, String), (String, Option<u32>)>)>;

fn check_all(dir: &Path, sb: &Storyboard) -> Checked {
    let mut out = Vec::new();
    for (i, scene) in sb.scenes.iter().enumerate() {
        let path = dir.join(prompt::scene_file(i, &scene.title));
        if !path.is_file() {
            out.push((i, path, Err(("the scene file was not written".to_string(), None))));
            continue;
        }
        let r = match check(&narrated_request(dir, &path, &scene.say)) {
            Ok(t) => Ok((t.problems.len(), t.duration, t.problems.first().map(|p| format!("{} at {:.1} s", p.message, p.time)).unwrap_or_default())),
            Err(e) => {
                let e = format!("{e:#}");
                Err((studio_render::short_error(&e), studio_render::error_line(&e)))
            }
        };
        out.push((i, path, r));
    }
    out
}

/// Every scene that fails, in order (missing files are not repairable).
fn failing(c: &Checked) -> Vec<(usize, PathBuf, String, Option<u32>)> {
    c.iter()
        .filter(|(_, p, _)| p.is_file())
        .filter_map(|(i, p, r)| match r {
            Ok((0, _, _)) => None,
            Ok((_, _, first)) => Some((*i, p.clone(), first.clone(), None)),
            Err((msg, line)) => Some((*i, p.clone(), msg.clone(), *line)),
        })
        .collect()
}

fn run_one(case: &Case, sb: &Storyboard, v: &Variant, run: u32, bench: &Bench, exe: &Path, work: &Path) -> Result<RunResult> {
    let dir = work.join(&case.id).join(format!("{}-{run}", v.id));
    let system = prepare(&dir, v)?;
    let req = |prompt: String, resume: Option<String>| -> Result<TurnRequest> {
        Ok(TurnRequest {
            provider: Provider::Claude,
            exe: exe.to_path_buf(),
            cwd: dir.clone(),
            system_file: system.clone(),
            prompt,
            model: model(v.model.as_deref())?,
            effort: effort(v.effort.as_deref())?,
            resume,
            tools: EDIT_TOOLS.to_vec(),
            ephemeral: false,
        })
    };
    let mut build = Usage::default();
    let mut sessions: Vec<Option<String>> = vec![None; sb.scenes.len()];
    let mut error = None;
    if v.recipe.per_scene {
        for (i, s) in sessions.iter_mut().enumerate() {
            let out = run_turn(req(prompt::build_scene(sb, i), None)?)?;
            build.add(&out.usage);
            *s = out.session;
            error = error.or(out.error);
        }
    } else {
        let out = run_turn(req(prompt::build(sb), None)?)?;
        build = out.usage;
        sessions.iter_mut().for_each(|s| *s = out.session.clone());
        error = out.error;
    }
    let first = check_all(&dir, sb);
    let mut checked = first.clone();
    let mut repair = Usage::default();
    let mut repairs = 0;
    // Like the app: one repair turn per round fixes every failing scene.
    loop {
        let fails = failing(&checked);
        if fails.is_empty() || repairs >= bench.repair_tries {
            break;
        }
        repairs += 1;
        let list: Vec<prompt::Failure> = fails
            .iter()
            .map(|(_, path, msg, line)| prompt::Failure {
                file: path.strip_prefix(&dir).unwrap_or(path).to_string_lossy().replace('\\', "/"),
                diagnostic: msg.clone(),
                excerpt: prompt::repair_excerpt(path, *line).or_else(|| prompt::layout_excerpt(path, msg)),
            })
            .collect();
        let first = fails[0].0;
        let out = run_turn(req(prompt::repair_all(&list), sessions[first].clone())?)?;
        repair.add(&out.usage);
        if let Some(s) = out.session {
            sessions[first] = Some(s);
        }
        checked = check_all(&dir, sb);
    }
    let thumbs = dir.join("thumbs");
    std::fs::create_dir_all(&thumbs)?;
    let mut scenes = Vec::new();
    for ((i, path, r), (_, _, r0)) in checked.iter().zip(&first) {
        let ok = matches!(r, Ok((0, _, _)));
        let thumb = (|| -> Result<String> {
            let output = Output::new(480, 270, 30.0);
            let (host, _, t) = load(&narrated_request(&dir, path, &sb.scenes[*i].say), output)?;
            let rgba = host.frame_at(thumbnail_time(&t))?;
            let file = thumbs.join(format!("{:02}.png", i + 1));
            studio_render::manim::output::save_png(&file, output.width, output.height, &rgba)?;
            Ok(file.strip_prefix(work)?.to_string_lossy().replace('\\', "/"))
        })()
        .ok();
        scenes.push(SceneResult {
            file: path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default(),
            planned_s: sb.scenes[*i].seconds,
            duration_s: r.as_ref().ok().map(|x| x.1),
            first_ok: matches!(r0, Ok((0, _, _))),
            ok,
            problems: r.as_ref().map(|x| x.0).unwrap_or(0),
            problem: r.as_ref().ok().filter(|x| x.0 > 0).map(|x| x.2.clone()),
            error: r.as_ref().err().map(|e| e.0.clone()),
            lines: std::fs::read_to_string(path).map(|s| s.lines().count()).unwrap_or(0),
            thumb,
        });
    }
    let missing = scenes.iter().filter(|s| s.lines == 0).count();
    Ok(RunResult { case: case.id.clone(), variant: v.id.clone(), run, build, repair, repairs, missing, scenes, error })
}

#[derive(Default)]
struct Agg {
    runs: u32,
    usage: Usage,
    scenes: u32,
    first_ok: u32,
    ok: u32,
    repairs: u32,
    missing: u32,
    problems: u32,
    dur_err: f64,
    dur_n: u32,
    lines: u32,
}

fn summary(results: &[RunResult], plan_usage: &Usage) -> String {
    let mut order: Vec<String> = Vec::new();
    let mut by: HashMap<String, Agg> = HashMap::new();
    for r in results {
        if !order.contains(&r.variant) {
            order.push(r.variant.clone());
        }
        let a = by.entry(r.variant.clone()).or_default();
        a.runs += 1;
        a.usage.add(&r.build);
        a.usage.add(&r.repair);
        a.repairs += r.repairs;
        a.missing += r.missing as u32;
        for s in &r.scenes {
            a.scenes += 1;
            a.first_ok += s.first_ok as u32;
            a.ok += s.ok as u32;
            a.problems += s.problems as u32;
            a.lines += s.lines as u32;
            if let Some(d) = s.duration_s {
                a.dur_err += (d - s.planned_s as f64).abs() / s.planned_s.max(1) as f64;
                a.dur_n += 1;
            }
        }
    }
    let base = order.first().and_then(|v| by.get(v)).map(|a| (a.usage.billed() as f64 / a.runs as f64, a.usage.cost_usd / a.runs as f64));
    let mut md = String::from("# Token benchmark\n\nPer run means (one run = one storyboard built, checked and repaired). \"Billed\" is fresh input + cache writes + output; cache reads are listed separately (cheap, but not free). Cost is the CLI's API-price estimate. Quality: scenes passing the engine check first time / after repairs, layout problems left, and how far scene lengths land from the plan.\n\n");
    md.push_str("| variant | runs | billed | vs first | cache read | output | calls | cost $ | vs first | time s | first pass | final pass | repairs | missing | layout left | length off | lines/scene |\n|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|\n");
    for v in &order {
        let a = &by[v];
        let n = a.runs as f64;
        let billed = a.usage.billed() as f64 / n;
        let cost = a.usage.cost_usd / n;
        let rel = |x: f64, b: f64| if b > 0.0 { format!("{:+.0}%", (x / b - 1.0) * 100.0) } else { "".into() };
        let (vb, vc) = base.map_or((String::new(), String::new()), |(b, c)| (rel(billed, b), rel(cost, c)));
        let pct = |k: u32| if a.scenes > 0 { format!("{:.0}%", k as f64 * 100.0 / a.scenes as f64) } else { "-".into() };
        md.push_str(&format!(
            "| {v} | {} | {:.0} | {vb} | {:.0} | {:.0} | {:.1} | {cost:.3} | {vc} | {:.0} | {} | {} | {:.1} | {} | {} | {} | {:.0} |\n",
            a.runs,
            billed,
            a.usage.cache_read as f64 / n,
            a.usage.output as f64 / n,
            a.usage.calls as f64 / n,
            a.usage.seconds / n,
            pct(a.first_ok),
            pct(a.ok),
            a.repairs as f64 / n,
            a.missing,
            a.problems,
            if a.dur_n > 0 { format!("{:.0}%", a.dur_err * 100.0 / a.dur_n as f64) } else { "-".into() },
            a.lines as f64 / a.scenes.max(1) as f64,
        ));
    }
    if plan_usage.calls > 0 {
        md.push_str(&format!("\nPlanning (new storyboards this run): {} billed, ${:.3}.\n", plan_usage.billed(), plan_usage.cost_usd));
    }
    md
}

fn report(results: &[RunResult], plans: &[(Case, Storyboard)]) -> String {
    let mut h = String::from("<!doctype html><meta charset=utf-8><title>Token benchmark</title><style>body{font:14px system-ui;background:#16181c;color:#e8e8e8;margin:24px}table{border-collapse:collapse;margin-bottom:32px}td,th{padding:6px;vertical-align:top;text-align:left;border-bottom:1px solid #333}img{width:240px;border-radius:4px;display:block}.bad{color:#ff8a80}.ok{color:#9ccc65}small{color:#999}</style><h1>Token benchmark</h1>");
    for (case, sb) in plans {
        h.push_str(&format!("<h2>{} <small>{}</small></h2><table><tr><th>variant</th>", case.id, sb.title));
        for (i, b) in sb.scenes.iter().enumerate() {
            h.push_str(&format!("<th>{}. {}<br><small>{} s</small></th>", i + 1, b.title, b.seconds));
        }
        h.push_str("</tr>");
        for r in results.iter().filter(|r| r.case == case.id) {
            h.push_str(&format!("<tr><td>{} #{}<br><small>{} billed · ${:.3}</small></td>", r.variant, r.run, r.build.billed() + r.repair.billed(), r.build.cost_usd + r.repair.cost_usd));
            for s in &r.scenes {
                let status = match (&s.error, s.ok) {
                    (Some(e), _) => format!("<span class=bad>{e}</span>"),
                    (None, true) => format!("<span class=ok>ok</span> {:.0} s", s.duration_s.unwrap_or(0.0)),
                    (None, false) => format!("<span class=bad>{} layout: {}</span>", s.problems, s.problem.clone().unwrap_or_default()),
                };
                let img = s.thumb.as_ref().map(|t| format!("<img src=\"{t}\">")).unwrap_or_default();
                h.push_str(&format!("<td>{img}<small>{status}</small></td>"));
            }
            h.push_str("</tr>");
        }
        h.push_str("</table>");
    }
    h
}

/// `--check <video folder>`: run every scene through the engine check and print problems.
fn recheck(dir: &Path, narration: Option<f64>) -> Result<()> {
    for path in studio_render::sample::scene_files(dir) {
        let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        match check(&scene_request(dir, &path, narration)) {
            Ok(t) if t.problems.is_empty() => println!("{name}: ok, {:.1} s", t.duration),
            Ok(t) => {
                for p in &t.problems {
                    println!("{name}: {} at {:.1} s", p.message, p.time);
                }
            }
            Err(e) => println!("{name}: {}", studio_render::short_error(&format!("{e:#}"))),
        }
    }
    Ok(())
}

fn main() -> Result<()> {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    if raw.first().is_some_and(|f| f == "--check") && raw.len() >= 2 {
        return recheck(Path::new(&raw[1]), raw.get(2).and_then(|n| n.parse().ok()));
    }
    let a = args()?;
    let bench: Bench = serde_json::from_str(&std::fs::read_to_string(&a.config).with_context(|| format!("reading {}", a.config.display()))?)?;
    let cases: Vec<Case> = bench.cases.iter().filter(|c| a.cases.as_ref().is_none_or(|l| l.contains(&c.id))).cloned().collect();
    let variants: Vec<Variant> = bench.variants.iter().filter(|v| a.variants.as_ref().is_none_or(|l| l.contains(&v.id))).cloned().collect();
    if cases.is_empty() || variants.is_empty() {
        bail!("no cases or variants selected");
    }
    if a.dry {
        println!("System prompt per variant (≈ chars / 4 tokens; written to the cache once per fresh session):");
        for v in &variants {
            let s = system_prompt(v.recipe);
            println!("  {:<16} {:>6} chars ≈ {:>5} tokens{}", v.id, s.len(), s.len() / 4, if v.recipe.per_scene { "  (one fresh session per scene)" } else { "" });
        }
        for c in &cases {
            let file = Path::new(ROOT).join("plans").join(format!("{}.json", c.id));
            match std::fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str::<Storyboard>(&t).ok()) {
                Some(sb) => println!("  build prompt for {:<14} {:>5} chars, {} scenes", c.id, prompt::build(&sb).len(), sb.scenes.len()),
                None => println!("  {} has no storyboard yet (planned on the first real run)", c.id),
            }
        }
        return Ok(());
    }
    let exe = studio_agent::detect::find(Provider::Claude).context("Claude Code isn't installed")?;
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs();
    let work = a.out.clone().unwrap_or_else(|| Path::new(ROOT).join("../../target/tokenbench").join(stamp.to_string()));
    std::fs::create_dir_all(&work)?;
    // Plain absolute path: canonicalize gives verbatim (\?) paths on Windows.
    let work = std::path::absolute(&work)?;
    println!("Writing to {}", work.display());
    let mut plan_usage = Usage::default();
    let mut plans = Vec::new();
    for c in &cases {
        let (sb, u) = plan_for(c, &bench, &exe, a.replan, &work)?;
        if let Some(u) = u {
            println!("planned {}: {} scenes, {} billed", c.id, sb.scenes.len(), u.billed());
            plan_usage.add(&u);
        }
        plans.push((c.clone(), sb));
    }
    let mut results: Vec<RunResult> = Vec::new();
    for run in 1..=a.runs {
        for (c, sb) in &plans {
            for v in &variants {
                print!("{} · {} · run {run}: ", c.id, v.id);
                let r = match run_one(c, sb, v, run, &bench, &exe, &work) {
                    Ok(r) => r,
                    Err(e) => {
                        println!("failed: {e:#}");
                        continue;
                    }
                };
                let ok = r.scenes.iter().filter(|s| s.ok).count();
                println!(
                    "{} billed, {} cache read, ${:.3}, {}/{} scenes pass ({} first time), {} repairs",
                    r.build.billed() + r.repair.billed(),
                    r.build.cache_read + r.repair.cache_read,
                    r.build.cost_usd + r.repair.cost_usd,
                    ok,
                    r.scenes.len(),
                    r.scenes.iter().filter(|s| s.first_ok).count(),
                    r.repairs
                );
                results.push(r);
                std::fs::write(work.join("results.json"), serde_json::to_string_pretty(&results)?)?;
            }
        }
    }
    let md = summary(&results, &plan_usage);
    std::fs::write(work.join("summary.md"), &md)?;
    std::fs::write(work.join("report.html"), report(&results, &plans))?;
    println!("\n{md}\nReport: {}", work.join("report.html").display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The prompt's sentence numbering must match the voice's sentences exactly.
    #[test]
    fn sentence_numbering_matches_the_voice() {
        for text in [
            "Here's a pizza. If we double its radius, do we get twice as much pizza? It feels like we should.",
            "Pi is about 3.14, so it's a little more than three squares. Ready? Well done!",
            "No full stop at the end",
        ] {
            assert_eq!(prompt::sentences(text), studio_render::voice::sentences(text), "{text}");
        }
    }

    /// The worked example in the system prompt must itself pass the engine check.
    #[test]
    fn example_scene_passes_the_check() {
        let dir = std::env::temp_dir().join(format!("tokenbench-example-{}", std::process::id()));
        let v = Variant { id: "t".into(), model: None, effort: None, recipe: Recipe { example: true, ..Recipe::default() } };
        prepare(&dir, &v).unwrap();
        let path = dir.join("scenes/02-diameter.ts");
        std::fs::write(&path, prompt::EXAMPLE_SCENE).unwrap();
        let t = check(&scene_request(&dir, &path, Some(9.0))).unwrap();
        assert!(t.problems.is_empty(), "{:?}", t.problems.iter().map(|p| &p.message).collect::<Vec<_>>());
        assert!((t.duration - (9.0 + studio_render::preview::NARRATION_TAIL)).abs() < 0.6, "paced to the narration: {}", t.duration);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
