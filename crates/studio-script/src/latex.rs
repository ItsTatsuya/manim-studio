//! LaTeX math to the Typst math the engine's `MathTex` takes. Teachers' scripts give
//! equations in LaTeX; the converter covers what lesson scripts use (fractions, scripts,
//! roots, spacing, relations, Greek letters, text, matrices, cases) and says what it
//! doesn't know. Whether the result renders is for the engine to say: Studio checks every
//! converted equation before building (`studio_render::equations`).
//!
//! Two things differ from a plain symbol swap. LaTeX reads a run of letters as separate
//! variables (`kx` is k times x) while Typst reads it as one name, so letters are spaced
//! out. And Typst writes a fraction as `a/b` and drops the parentheses around a numerator
//! or denominator, so groups become parentheses only when they have more than one token.

/// A converted equation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Converted {
    pub typst: String,
    /// Commands the converter doesn't know (kept by name in `typst`).
    pub warnings: Vec<String>,
}

/// `\name` mapped to Typst, for commands that stand for one symbol, space or function name.
pub const SYMBOLS: &[(&str, &str)] = &[
    // Spacing
    ("quad", "quad"), ("qquad", "wide"), (",", "thin"), (":", "med"), (";", "thick"), (" ", "space"),
    ("thinspace", "thin"), ("medspace", "med"), ("thickspace", "thick"),
    // Relations and arithmetic
    ("neq", "!="), ("ne", "!="), ("leq", "<="), ("le", "<="), ("geq", ">="), ("ge", ">="), ("approx", "approx"),
    ("equiv", "equiv"), ("sim", "tilde"), ("simeq", "tilde.eq"), ("cong", "tilde.equiv"), ("propto", "prop"),
    ("ll", "<<"), ("gg", ">>"), ("pm", "plus.minus"), ("mp", "minus.plus"), ("cdot", "dot"), ("times", "times"),
    ("div", "div"), ("ast", "*"), ("star", "star"), ("circ", "circle.small"), ("bullet", "bullet"), ("oplus", "plus.o"),
    ("otimes", "times.o"), ("perp", "perp"), ("parallel", "parallel"), ("angle", "angle"), ("triangle", "triangle.stroked.t"),
    ("degree", "degree"),
    // Arrows and logic
    ("to", "arrow.r"), ("rightarrow", "arrow.r"), ("leftarrow", "arrow.l"), ("gets", "arrow.l"),
    ("leftrightarrow", "arrow.l.r"), ("Rightarrow", "arrow.r.double"), ("Leftarrow", "arrow.l.double"),
    ("Leftrightarrow", "arrow.l.r.double"), ("implies", "arrow.r.double.long"), ("iff", "arrow.l.r.double.long"),
    ("impliedby", "arrow.l.double.long"), ("mapsto", "arrow.r.bar"), ("longrightarrow", "arrow.r.long"),
    ("uparrow", "arrow.t"), ("downarrow", "arrow.b"), ("therefore", "therefore"), ("because", "because"),
    ("forall", "forall"), ("exists", "exists"), ("neg", "not"), ("lnot", "not"), ("land", "and"), ("lor", "or"),
    ("wedge", "and"), ("vee", "or"),
    // Sets
    ("in", "in"), ("notin", "in.not"), ("ni", "in.rev"), ("subset", "subset"), ("subseteq", "subset.eq"),
    ("supset", "supset"), ("supseteq", "supset.eq"), ("cup", "union"), ("cap", "inter"), ("setminus", "without"),
    ("emptyset", "emptyset"), ("varnothing", "emptyset"), ("infty", "infinity"), ("partial", "partial"), ("nabla", "nabla"),
    // Dots
    ("ldots", "dots.h"), ("dots", "dots.h"), ("cdots", "dots.h.c"), ("vdots", "dots.v"), ("ddots", "dots.down"),
    // Big operators and functions (Typst spells them the same, or close to it)
    ("sum", "sum"), ("prod", "product"), ("int", "integral"), ("iint", "integral.double"), ("oint", "integral.cont"),
    ("lim", "lim"), ("sin", "sin"), ("cos", "cos"), ("tan", "tan"), ("cot", "cot"), ("sec", "sec"), ("csc", "csc"),
    ("arcsin", "arcsin"), ("arccos", "arccos"), ("arctan", "arctan"), ("sinh", "sinh"), ("cosh", "cosh"), ("tanh", "tanh"),
    ("log", "log"), ("ln", "ln"), ("exp", "exp"), ("max", "max"), ("min", "min"), ("sup", "sup"), ("inf", "inf"),
    ("det", "det"), ("dim", "dim"), ("gcd", "gcd"), ("deg", "deg"), ("ker", "ker"), ("arg", "arg"), ("Pr", "Pr"),
    ("mod", "mod"), ("bmod", "mod"), ("hbar", "ℏ"), ("ell", "ell"), ("Re", "Re"), ("Im", "Im"),
    ("aleph", "aleph"),
    // Greek
    ("alpha", "alpha"), ("beta", "beta"), ("gamma", "gamma"), ("delta", "delta"), ("epsilon", "epsilon"),
    ("varepsilon", "epsilon.alt"), ("zeta", "zeta"), ("eta", "eta"), ("theta", "theta"), ("vartheta", "theta.alt"),
    ("iota", "iota"), ("kappa", "kappa"), ("lambda", "lambda"), ("mu", "mu"), ("nu", "nu"), ("xi", "xi"), ("pi", "pi"),
    ("varpi", "pi.alt"), ("rho", "rho"), ("varrho", "rho.alt"), ("sigma", "sigma"), ("varsigma", "sigma.alt"),
    ("tau", "tau"), ("upsilon", "upsilon"), ("phi", "phi.alt"), ("varphi", "phi"), ("chi", "chi"), ("psi", "psi"),
    ("omega", "omega"), ("Gamma", "Gamma"), ("Delta", "Delta"), ("Theta", "Theta"), ("Lambda", "Lambda"), ("Xi", "Xi"),
    ("Pi", "Pi"), ("Sigma", "Sigma"), ("Upsilon", "Upsilon"), ("Phi", "Phi"), ("Psi", "Psi"), ("Omega", "Omega"),
    // Punctuation and escapes
    ("{", "{"), ("}", "}"), ("%", "%"), ("$", "dollar"), ("#", "\\#"), ("&", "\\&"), ("_", "\\_"), ("|", "parallel"),
    ("langle", "chevron.l"), ("rangle", "chevron.r"), ("lfloor", "floor.l"), ("rfloor", "floor.r"), ("lceil", "ceil.l"),
    ("rceil", "ceil.r"), ("lvert", "|"), ("rvert", "|"), ("vert", "|"), ("mid", "|"), ("backslash", "backslash"),
    ("lbrace", "{"), ("rbrace", "}"), ("lbrack", "["), ("rbrack", "]"), ("colon", ":"),
];

/// Commands that take one braced argument and become a Typst function of it.
pub const WRAPPERS: &[(&str, &str)] = &[
    ("vec", "arrow"), ("hat", "hat"), ("widehat", "hat"), ("bar", "macron"), ("overline", "overline"), ("underline", "underline"),
    ("dot", "dot"), ("ddot", "dot.double"), ("tilde", "tilde"), ("widetilde", "tilde"), ("overrightarrow", "arrow"),
    ("mathbf", "bold"), ("boldsymbol", "bold"), ("bm", "bold"), ("mathit", "italic"), ("mathcal", "cal"),
    ("mathscr", "scr"), ("mathfrak", "frak"), ("mathsf", "sans"), ("mathtt", "mono"), ("overbrace", "overbrace"),
    ("underbrace", "underbrace"),
];

/// Commands that only change the look in a way Typst's defaults already cover.
const IGNORED: &[&str] = &["displaystyle", "textstyle", "scriptstyle", "limits", "nolimits", "big", "Big", "bigg", "Bigg", "bigl", "bigr", "!", "relax", "protect"];

struct Parser {
    chars: Vec<char>,
    i: usize,
    warnings: Vec<String>,
}

/// Where a sequence ends.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Until {
    End,
    /// A closing `}`.
    Brace,
    /// An `\end{...}`, cells (`&`) and rows (`\\`) are kept apart.
    Environment,
}

/// Output tokens. A token marked `attached` follows its neighbour without a space
/// (superscripts, subscripts, function arguments).
#[derive(Default)]
struct Tokens(Vec<(String, bool)>);

impl Tokens {
    fn push(&mut self, t: impl Into<String>) {
        self.0.push((t.into(), false));
    }

    fn attach(&mut self, t: impl Into<String>) {
        self.0.push((t.into(), true));
    }

    fn join(&self) -> String {
        let mut out = String::new();
        for (t, attached) in &self.0 {
            // Nothing is set off from an opening bracket.
            // A sign at the start belongs to what follows it.
            if !out.is_empty() && !attached && !out.ends_with(['(', '[']) && out != "-" && out != "+" {
                out.push(' ');
            }
            out.push_str(t);
        }
        out
    }

}

/// A script or fraction argument: bare when it is one character or number, else in
/// parentheses (Typst drops them when it draws the result).
fn script_arg(body: &str) -> String {
    if body == "degree" || body.chars().count() == 1 && body.chars().all(char::is_alphanumeric) {
        body.to_string()
    } else {
        format!("({body})")
    }
}

/// A fraction part: bare when it has no spaces (one token), else in parentheses.
fn frac_arg(body: &str) -> String {
    if body.is_empty() || body.contains(' ') { format!("({body})") } else { body.to_string() }
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.i).copied()
    }

    fn eat_space(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.i += 1;
        }
    }

    fn starts_with(&self, s: &str) -> bool {
        s.chars().enumerate().all(|(k, c)| self.chars.get(self.i + k) == Some(&c))
    }

    /// `\name` (letters) or `\` and one character; the cursor is on the backslash.
    fn command(&mut self) -> String {
        self.i += 1;
        let Some(c) = self.peek() else { return String::new() };
        if c.is_ascii_alphabetic() {
            let start = self.i;
            while self.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
                self.i += 1;
            }
            // `\text{` and `\text {` are the same; so is a star on an environment name.
            return self.chars[start..self.i].iter().collect();
        }
        self.i += 1;
        c.to_string()
    }

    /// The text inside the braces that follow, raw (for `\text`, `\begin` and the like).
    fn raw_group(&mut self) -> String {
        self.eat_space();
        if self.peek() != Some('{') {
            return String::new();
        }
        self.i += 1;
        let mut depth = 1;
        let mut out = String::new();
        while let Some(c) = self.peek() {
            self.i += 1;
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            out.push(c);
        }
        out
    }

    /// One argument: a braced group, a command or a single character, as Typst.
    fn argument(&mut self) -> String {
        self.eat_space();
        match self.peek() {
            Some('{') => {
                self.i += 1;
                self.sequence(Until::Brace).0
            }
            Some('\\') => {
                let mut t = Tokens::default();
                self.item(&mut t);
                t.join()
            }
            Some(c) => {
                self.i += 1;
                if c.is_ascii_alphabetic() || c.is_ascii_digit() { c.to_string() } else { self.symbol(c) }
            }
            None => String::new(),
        }
    }

    /// A character that stands alone.
    fn symbol(&mut self, c: char) -> String {
        match c {
            '"' => "\\\"".into(),
            '#' | '$' | '@' | '\\' => format!("\\{c}"),
            '~' => "space".into(),
            _ => c.to_string(),
        }
    }

    /// Parse until the end of the group. Returns the Typst and whether an environment's
    /// cell or row separator ended it (`&`, `\\`, `\end`).
    fn sequence(&mut self, until: Until) -> (String, Stop) {
        let mut t = Tokens::default();
        loop {
            self.eat_space();
            let Some(c) = self.peek() else { return (t.join(), Stop::End) };
            match c {
                '}' if until == Until::Brace => {
                    self.i += 1;
                    return (t.join(), Stop::End);
                }
                '}' => {
                    // A stray closing brace.
                    self.i += 1;
                }
                '&' if until == Until::Environment => {
                    self.i += 1;
                    return (t.join(), Stop::Cell);
                }
                '\\' if until == Until::Environment && self.chars.get(self.i + 1) == Some(&'\\') => {
                    self.i += 2;
                    // `\\[2pt]` spacing.
                    if self.peek() == Some('[') {
                        while self.peek().is_some_and(|c| c != ']') {
                            self.i += 1;
                        }
                        self.i += 1;
                    }
                    return (t.join(), Stop::Row);
                }
                '\\' if until == Until::Environment && self.starts_with("\\end") => {
                    return (t.join(), Stop::End);
                }
                _ => self.item(&mut t),
            }
        }
    }

    /// One element: a letter, a number, a command, a group, a script, a symbol.
    fn item(&mut self, t: &mut Tokens) {
        let Some(c) = self.peek() else { return };
        match c {
            '%' => {
                while self.peek().is_some_and(|c| c != '\n') {
                    self.i += 1;
                }
            }
            '$' => self.i += 1,
            '{' => {
                self.i += 1;
                let (body, _) = self.sequence(Until::Brace);
                let scripted = matches!(self.peek(), Some('^' | '_'));
                if scripted && body.contains(' ') {
                    t.push(format!("({body})"));
                } else if !body.is_empty() {
                    t.push(body);
                }
            }
            '^' | '_' => {
                self.i += 1;
                let arg = self.argument();
                // `^\circ` is a degree sign.
                let arg = if c == '^' && arg == "circle.small" { "degree".to_string() } else { arg };
                t.attach(format!("{c}{}", script_arg(&arg)));
            }
            '\'' => {
                self.i += 1;
                t.attach("'");
            }
            '\\' => self.command_item(t),
            '~' => {
                self.i += 1;
                t.push("space");
            }
            c if c.is_ascii_digit() => {
                let start = self.i;
                while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                    self.i += 1;
                }
                // A decimal point only continues the number when a digit follows.
                if self.peek() == Some('.') && self.chars.get(self.i + 1).is_some_and(char::is_ascii_digit) {
                    self.i += 1;
                    while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                        self.i += 1;
                    }
                }
                t.push(self.chars[start..self.i].iter().collect::<String>());
            }
            c if c.is_ascii_alphabetic() => {
                // A run of letters is separate variables in LaTeX.
                self.i += 1;
                t.push(c.to_string());
            }
            c => {
                self.i += 1;
                let s = self.symbol(c);
                // Punctuation and closing brackets stay with what they follow.
                if matches!(c, ',' | ';' | '.' | ')' | ']' | '!' | '?' | ':') && !t.0.is_empty() {
                    t.attach(s);
                } else {
                    t.push(s);
                }
            }
        }
    }

    fn command_item(&mut self, t: &mut Tokens) {
        let name = self.command();
        let n = name.as_str();
        match n {
            "" => {}
            // A line break outside an environment (Typst's `\`).
            "\\" => t.push("\\"),
            "frac" | "dfrac" | "tfrac" | "cfrac" => {
                let (a, b) = (self.argument(), self.argument());
                t.push(format!("{}/{}", frac_arg(&a), frac_arg(&b)));
            }
            "binom" | "dbinom" | "tbinom" => {
                let (a, b) = (self.argument(), self.argument());
                t.push(format!("binom({a}, {b})"));
            }
            "sqrt" => {
                self.eat_space();
                if self.peek() == Some('[') {
                    self.i += 1;
                    let mut index = String::new();
                    while let Some(c) = self.peek() {
                        self.i += 1;
                        if c == ']' {
                            break;
                        }
                        index.push(c);
                    }
                    let body = self.argument();
                    t.push(format!("root({}, {body})", index.trim()));
                } else {
                    let body = self.argument();
                    t.push(format!("sqrt({body})"));
                }
            }
            "text" | "textrm" | "textit" | "textbf" | "mathrm" | "mbox" | "mathop" => {
                let raw = self.raw_group();
                let raw = raw.as_str();
                // A unit or word keeps its letters together; `\mathrm{d}` before a variable is the differential.
                if n == "mathrm" && raw == "d" {
                    t.push("dif");
                } else if raw.is_empty() {
                    // Nothing to write.
                } else if n == "textit" {
                    t.push(format!("italic(\"{}\")", escape(raw)));
                } else if n == "textbf" {
                    t.push(format!("bold(\"{}\")", escape(raw)));
                } else {
                    t.push(format!("\"{}\"", escape(raw)));
                }
            }
            "operatorname" => {
                let raw = self.raw_group();
                t.push(format!("op(\"{}\")", escape(raw.trim())));
            }
            "mathbb" => {
                let raw = self.raw_group();
                let body = raw.trim();
                if body.len() == 1 && body.chars().all(|c| c.is_ascii_uppercase()) {
                    t.push(format!("{body}{body}"));
                } else {
                    t.push(format!("bb({body})"));
                }
            }
            "left" | "right" => {
                self.eat_space();
                match self.peek() {
                    Some('.') => self.i += 1,
                    Some('\\') => {
                        let d = self.command();
                        match d.as_str() {
                            "{" => t.push("{"),
                            "}" => t.push("}"),
                            "|" | "vert" | "lvert" | "rvert" => t.push("|"),
                            "langle" => t.push("chevron.l"),
                            "rangle" => t.push("chevron.r"),
                            "lfloor" => t.push("floor.l"),
                            "rfloor" => t.push("floor.r"),
                            "lceil" => t.push("ceil.l"),
                            "rceil" => t.push("ceil.r"),
                            other => self.warnings.push(format!("unsupported delimiter \\{other}")),
                        }
                    }
                    Some(c) => {
                        self.i += 1;
                        t.push(c.to_string());
                    }
                    None => {}
                }
            }
            "not" => {
                self.eat_space();
                if self.peek() == Some('=') {
                    self.i += 1;
                    t.push("!=");
                } else {
                    let arg = self.argument();
                    t.push(format!("cancel({arg})"));
                }
            }
            "hspace" | "vspace" | "label" | "tag" | "nonumber" | "notag" => {
                if !matches!(n, "nonumber" | "notag") {
                    self.raw_group();
                }
            }
            "begin" => self.environment(t),
            "end" => {
                self.raw_group();
            }
            "boxed" => {
                let body = self.argument();
                t.push(body);
                self.warnings.push("\\boxed is drawn without its box".into());
            }
            _ if IGNORED.contains(&n) => {}
            _ => {
                if let Some((_, typst)) = WRAPPERS.iter().find(|(k, _)| *k == n) {
                    let body = self.argument();
                    t.push(format!("{typst}({body})"));
                } else if let Some((_, typst)) = SYMBOLS.iter().find(|(k, _)| *k == n) {
                    t.push(*typst);
                } else {
                    self.warnings.push(format!("unsupported LaTeX command \\{n}"));
                    t.push(n);
                }
            }
        }
    }

    /// `\begin{name} ... \end{name}`: cases, matrices and aligned equations.
    fn environment(&mut self, t: &mut Tokens) {
        let name = self.raw_group();
        let name = name.trim().trim_end_matches('*').to_string();
        // `\begin{array}{cc}` carries a column spec.
        if name == "array" {
            self.raw_group();
        }
        let mut rows: Vec<Vec<String>> = vec![Vec::new()];
        loop {
            let (cell, stop) = self.sequence(Until::Environment);
            rows.last_mut().expect("a row").push(cell);
            match stop {
                Stop::Cell => {}
                Stop::Row => rows.push(Vec::new()),
                Stop::End => break,
            }
        }
        if self.starts_with("\\end") {
            self.command();
            self.raw_group();
        }
        // A trailing `\\` leaves an empty last row.
        while rows.len() > 1 && rows.last().is_some_and(|r| r.iter().all(String::is_empty)) {
            rows.pop();
        }
        let join_cells = |r: &Vec<String>, sep: &str| r.iter().filter(|c| !c.is_empty()).cloned().collect::<Vec<_>>().join(sep);
        let typst = match name.as_str() {
            "cases" => format!("cases({})", rows.iter().map(|r| join_cells(r, " ")).collect::<Vec<_>>().join(", ")),
            "matrix" | "pmatrix" | "bmatrix" | "Bmatrix" | "vmatrix" | "Vmatrix" | "array" | "smallmatrix" => {
                let delim = match name.as_str() {
                    "pmatrix" | "array" => "\"(\"",
                    "bmatrix" => "\"[\"",
                    "Bmatrix" => "\"{\"",
                    "vmatrix" => "\"|\"",
                    "Vmatrix" => "\"||\"",
                    _ => "#none",
                };
                let body = rows.iter().map(|r| r.join(", ")).collect::<Vec<_>>().join("; ");
                format!("mat(delim: {delim}, {body})")
            }
            "aligned" | "align" | "alignat" | "gathered" | "gather" | "split" | "eqnarray" | "equation" | "displaymath" | "math" => {
                rows.iter().map(|r| r.join(" & ").trim().to_string()).collect::<Vec<_>>().join(" \\ ")
            }
            other => {
                self.warnings.push(format!("unsupported environment {other}"));
                rows.iter().map(|r| join_cells(r, " ")).collect::<Vec<_>>().join(" ")
            }
        };
        t.push(typst);
    }
}

/// Why a sequence stopped.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Stop {
    End,
    Cell,
    Row,
}

/// Text for a Typst string literal.
fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// LaTeX math (with or without `$ $` or `\[ \]` around it) as Typst math.
pub fn to_typst(latex: &str) -> Converted {
    let src = latex.trim().trim_start_matches("\\[").trim_end_matches("\\]");
    let mut p = Parser { chars: src.chars().collect(), i: 0, warnings: Vec::new() };
    // `$` signs and stray closing braces are skipped on the way.
    let (typst, _) = p.sequence(Until::End);
    Converted { typst, warnings: p.warnings }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(latex: &str) -> String {
        to_typst(latex).typst
    }

    #[test]
    fn letters_are_separate_variables() {
        assert_eq!(t("kx + 2y = 3"), "k x + 2 y = 3");
        assert_eq!(t("a_1x + b_1y = c_1"), "a_1 x + b_1 y = c_1");
        assert_eq!(t("p = 0.5q"), "p = 0.5 q");
    }

    #[test]
    fn fractions_scripts_and_roots() {
        assert_eq!(t(r"\frac{a_1}{a_2} \neq \frac{b_1}{b_2}"), "a_1/a_2 != b_1/b_2");
        assert_eq!(t(r"\frac{x + 1}{2}"), "(x + 1)/2");
        assert_eq!(t("x^{2} + y^{10} + e^{-x}"), "x^2 + y^(10) + e^(-x)");
        assert_eq!(t(r"\sqrt{x^2 + y^2}"), "sqrt(x^2 + y^2)");
        assert_eq!(t(r"\sqrt[3]{x}"), "root(3, x)");
        assert_eq!(t(r"k^2 - 36 \neq 0 \;\Rightarrow\; k \neq \pm 6"), "k^2 - 36 != 0 thick arrow.r.double thick k != plus.minus 6");
        assert_eq!(t(r"\{x \mid x > 0\}"), "{ x | x > 0 }");
    }

    #[test]
    fn spacing_text_and_symbols() {
        assert_eq!(t(r"3r + 2c = 70 \qquad r + 4c = 90"), "3 r + 2 c = 70 wide r + 4 c = 90");
        assert_eq!(t(r"r = 10, \quad c = 20"), "r = 10, quad c = 20");
        assert_eq!(t(r"A = \pi r^2 \text{ cm}^2"), "A = pi r^2 \" cm\"^2");
        assert_eq!(t(r"\int_0^1 x \, \mathrm{d}x"), "integral_0^1 x thin dif x");
        assert_eq!(t(r"x \in \mathbb{R}, \; 90^\circ"), "x in RR, thick 90^degree");
        assert_eq!(t(r"\vec{v} \cdot \hat{n} \times \overline{AB}"), "arrow(v) dot hat(n) times overline(A B)");
        assert_eq!(t("$a + b$"), "a + b");
    }

    #[test]
    fn environments() {
        assert_eq!(t(r"\begin{cases} x + y = 1 \\ x - y = 3 \end{cases}"), "cases(x + y = 1, x - y = 3)");
        assert_eq!(t(r"\begin{pmatrix} a & b \\ c & d \end{pmatrix}"), "mat(delim: \"(\", a, b; c, d)");
        assert_eq!(t(r"\begin{aligned} a &= b + 1 \\ &= c \end{aligned}"), "a & = b + 1 \\ & = c");
    }

    #[test]
    fn unknown_commands_are_reported() {
        let c = to_typst(r"\foo{x} + \frac{1}{2}");
        assert_eq!(c.warnings, ["unsupported LaTeX command \\foo"]);
        assert!(c.typst.contains("1/2"));
    }
}
