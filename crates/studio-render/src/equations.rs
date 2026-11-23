//! Does the engine draw this equation? Teachers' scripts give LaTeX, Studio converts it to
//! Typst (`studio_script::latex`), and the converter can't know every symbol the engine's
//! Typst has. So each converted equation is built as a `MathTex` in a scratch scene, the
//! way a scene would build it, before anything is written for the video.

use anyhow::{Context, Result, anyhow};

use crate::Output;
use crate::preview::{OpenScene, load};

/// Typst layout and deep scene graphs want more than the default 2 MB.
const STACK: usize = 32 * 1024 * 1024;

/// For each equation (Typst), why the engine couldn't draw it, or `None` when it can.
/// Runs the engine on its own thread; takes a moment, so call it off the UI thread.
pub fn check(equations: &[String], theme_json: &str) -> Result<Vec<Option<String>>> {
    if equations.is_empty() {
        return Ok(Vec::new());
    }
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!("studio-equations-{}-{}", std::process::id(), NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
    std::fs::create_dir_all(dir.join("scenes")).context("making the scratch folder")?;
    let path = dir.join("scenes/01-equations.ts");
    let list = serde_json::to_string(equations)?;
    // Each equation is built on its own, so one that fails doesn't hide the others. The
    // engine's complaint (a Typst error, with its position) is kept.
    std::fs::write(
        &path,
        format!(
            "import {{ Scene, MathTex }} from \"manim\";\n\
             export default class EquationCheck extends Scene {{\n  construct() {{\n    \
             const found: (string | null)[] = [];\n    \
             for (const tex of {list}) {{\n      \
             try {{ MathTex(tex); found.push(null); }} catch (e) {{ found.push(String(e)); }}\n    }}\n    \
             (globalThis as any).__EQUATIONS = found;\n    this.wait(0.1);\n  }}\n}}\n"
        ),
    )?;
    let req = OpenScene {
        path,
        root: dir.clone(),
        theme_json: theme_json.to_string(),
        scene: None,
        globals: vec![],
        narration: None,
        narration_lines: vec![],
        narration_cues: vec![],
        critic: None,
    };
    let result = std::thread::Builder::new()
        .stack_size(STACK)
        .spawn(move || {
            crate::guard(|| {
                let (host, _, _) = load(&req, Output::new(320, 180, 30.0))?;
                let found: Vec<Option<String>> = host.eval_json("globalThis.__EQUATIONS")?;
                Ok(found)
            })
        })
        .context("spawning the equation check")?
        .join()
        .map_err(|_| anyhow!("the engine panicked"))?;
    let _ = std::fs::remove_dir_all(&dir);
    result.map(|found| found.into_iter().map(|e| e.map(|m| tidy(&m))).collect())
}

/// An engine complaint on one line, without the stack.
fn tidy(message: &str) -> String {
    let line = message.lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with("at ")).unwrap_or("it didn't render");
    line.trim_start_matches("Error: ").chars().take(200).collect()
}

/// [`check`] for a folder-less caller: the house style's theme.
pub fn check_in_style(equations: &[String], style: usize) -> Result<Vec<Option<String>>> {
    check(equations, crate::style::json(style))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eqs(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    /// The check has to be able to say no: a raw LaTeX equation, a letter run Typst reads
    /// as one unknown name and an unknown function must each fail, and in place, while the
    /// equations around them pass.
    #[test]
    fn bad_equations_fail_and_good_ones_pass() {
        let found = check_in_style(
            &eqs(&["3 r + 2 c = 70", "\\frac{a}{b} = c", "k x + 2 y = 3", "kx + 2y = 3", "a_1/a_2 != b_1/b_2", "foo(1) + 2", "sqrt(x^2 + y^2)"]),
            0,
        )
        .unwrap();
        let failed: Vec<bool> = found.iter().map(Option::is_some).collect();
        assert_eq!(failed, [false, true, false, true, false, true, false], "{found:?}");
        // The message says what is wrong, not a stack.
        assert!(found[3].as_deref().is_some_and(|m| !m.is_empty() && !m.contains('\n')), "{found:?}");
        assert!(check_in_style(&[], 0).unwrap().is_empty());
    }

    /// Every symbol, spacing command and wrapper the converter maps is a name the engine's
    /// Typst knows, and the converted equations of a lesson render.
    #[test]
    fn converted_latex_renders() {
        use studio_script::latex::{SYMBOLS, WRAPPERS, to_typst};
        let mut all: Vec<(String, String)> = Vec::new();
        for (cmd, typst) in SYMBOLS {
            all.push((format!("\\{cmd}"), format!("a {typst} b")));
        }
        for (cmd, typst) in WRAPPERS {
            all.push((format!("\\{cmd}"), format!("{typst}(x)")));
        }
        for latex in [
            r"3r + 2c = 70 \qquad r + 4c = 90",
            r"r = 10, \quad c = 20",
            r"3x = 9 \;\Rightarrow\; x = 3, \quad y = 1",
            r"0 = 2",
            r"x = 4 - 2t, \quad y = t",
            r"a_1x + b_1y = c_1 \qquad a_2x + b_2y = c_2",
            r"\frac{a_1}{a_2} \neq \frac{b_1}{b_2} \qquad \frac{a_1}{a_2} = \frac{b_1}{b_2} \neq \frac{c_1}{c_2}",
            r"a_1b_2 - a_2b_1 \neq 0",
            r"k^2 - 36 \neq 0 \;\Rightarrow\; k \neq \pm 6",
            r"\sqrt{x^2 + y^2} + \sqrt[3]{8} + \frac{x + 1}{2}",
            r"A = \pi r^2 \text{ cm}^2, \quad \theta = 90^\circ",
            r"\int_0^1 x \, \mathrm{d}x = \frac{1}{2}",
            r"\sum_{i=1}^{n} i = \frac{n(n+1)}{2}",
            r"\lim_{x \to 0} \frac{\sin x}{x} = 1",
            r"x \in \mathbb{R}, \; \vec{v} \cdot \vec{w}",
            r"\left( \frac{a}{b} \right)^{2} \geq 0",
            r"\begin{cases} x + y = 1 \\ x - y = 3 \end{cases}",
            r"\begin{pmatrix} a & b \\ c & d \end{pmatrix}",
            r"\begin{aligned} a &= b + 1 \\ &= c \end{aligned}",
            r"f'(x) = 2x, \; \overline{AB} \parallel \overline{CD}",
        ] {
            all.push((latex.to_string(), to_typst(latex).typst));
        }
        let typst: Vec<String> = all.iter().map(|(_, t)| t.clone()).collect();
        let found = check_in_style(&typst, 0).unwrap();
        let broken: Vec<String> = all.iter().zip(&found).filter_map(|((l, t), e)| e.as_ref().map(|e| format!("{l} -> {t}: {e}"))).collect();
        assert!(broken.is_empty(), "{}", broken.join("\n"));
    }
}
