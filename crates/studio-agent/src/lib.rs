//! Manim Studio's assistant layer: drives the Claude Code and Codex CLIs the teacher is
//! already signed in to. Studio never sees or stores credentials; it starts the official
//! CLI in non-interactive mode and reads its JSON event stream.
//!
//! - [`detect`]: is the CLI installed, which version, signed in, which plan.
//! - [`turn`]: run one assistant turn in a video folder and stream [`AgentEvent`]s.
//! - [`claude`] / [`codex`]: command lines and event parsers for each CLI.
//! - [`prompt`]: the compact prompts (API digest, storyboard, build, edit, repair).

pub mod claude;
pub mod codex;
pub mod detect;
pub mod events;
mod process;
pub mod prompt;
pub mod turn;

pub use events::AgentEvent;

/// Which assistant CLI.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Provider {
    Claude,
    Codex,
}

impl Provider {
    pub const ALL: [Provider; 2] = [Provider::Claude, Provider::Codex];

    /// The executable name.
    pub fn exe(self) -> &'static str {
        match self {
            Provider::Claude => "claude",
            Provider::Codex => "codex",
        }
    }
}

/// Model size, independent of provider.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Size {
    Small,
    Medium,
    Large,
}

impl Size {
    /// The `--model` value, or `None` to keep the CLI's own default (Codex picks the
    /// model from the teacher's ChatGPT plan; forcing a name could pick one their plan
    /// doesn't include).
    pub fn model(self, p: Provider) -> Option<&'static str> {
        match (p, self) {
            (Provider::Claude, Size::Small) => Some("haiku"),
            (Provider::Claude, Size::Medium) => Some("sonnet"),
            (Provider::Claude, Size::Large) => Some("opus"),
            (Provider::Codex, _) => None,
        }
    }
}

/// How long the model thinks. `None` lets the CLI decide.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effort {
    Low,
    Medium,
    High,
    Max,
}

impl Effort {
    pub fn claude(self) -> &'static str {
        match self {
            Effort::Low => "low",
            Effort::Medium => "medium",
            Effort::High => "high",
            Effort::Max => "max",
        }
    }

    pub fn codex(self) -> &'static str {
        match self {
            Effort::Low => "low",
            Effort::Medium => "medium",
            Effort::High | Effort::Max => "high",
        }
    }

    /// One step more (repairs that keep failing think harder).
    pub fn raise(self) -> Effort {
        match self {
            Effort::Low => Effort::Medium,
            Effort::Medium => Effort::High,
            Effort::High | Effort::Max => Effort::Max,
        }
    }
}
