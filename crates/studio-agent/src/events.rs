//! What a turn reports, independent of the CLI behind it.

/// Plan usage as fractions (0..=1) of the provider's windows.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RateLimits {
    /// The short window (5 hours for both Claude and ChatGPT plans).
    pub window: Option<f32>,
    pub week: Option<f32>,
    /// When the short window resets (Unix seconds).
    pub window_resets_at: Option<u64>,
    /// When the weekly window resets (Unix seconds).
    pub week_resets_at: Option<u64>,
}

/// Token counts of a turn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tokens {
    pub input: u64,
    pub output: u64,
    /// Input served from the provider's prompt cache (cheap).
    pub cached: u64,
    /// Part of `input` written to the prompt cache (Claude; priced above plain input).
    pub cache_write: u64,
}

impl Tokens {
    /// Tokens that count against the plan in practice (cached input is mostly free).
    pub fn billed(&self) -> u64 {
        self.input + self.output
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum AgentEvent {
    /// The CLI started; `session` resumes this conversation next turn.
    Started { session: String },
    /// A complete block of the assistant's reply.
    Text(String),
    /// The assistant used a tool. `target` is the file for file tools.
    Tool { name: String, target: Option<String> },
    /// A file was created or changed.
    FileChanged { path: String },
    RateLimit(RateLimits),
    /// Running token count of one model response (Claude reports these mid-turn; the
    /// same `message` may be reported several times, the last report wins).
    Usage { message: String, tokens: Tokens },
    /// The turn finished. `error` is set when it failed.
    Done { session: Option<String>, tokens: Tokens, cost_usd: Option<f64>, error: Option<String> },
}
