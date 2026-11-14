//! Plain-language explanations for Claude Code and Codex failures.
//!
//! Both CLIs report problems as free text (Claude Code adds an API error code to its
//! `result` event, which [`crate::claude`] appends as `[code, HTTP n]`). [`explain`] turns
//! that text into a short title, what it means for the teacher, and the actions that fix
//! it. The raw text stays available as details.

use crate::Provider;

/// Something the app can offer to fix a failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fix {
    /// Run the CLI's sign-in again.
    SignIn,
    /// Install the CLI.
    Install,
    /// Update the CLI (its official installer also updates).
    Update,
    /// Open a web page (billing, usage, status).
    Open { label: &'static str, url: &'static str },
    /// Run the same request again.
    Retry,
    /// Use the other connected assistant.
    SwitchProvider,
    /// Settings → Assistants & usage (model choice, sign-in state).
    Settings,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    NotInstalled,
    SignedOut,
    /// The subscription can't be used (payment due, organization policy, no credits).
    Billing,
    UsageLimit,
    Overloaded,
    Network,
    Model,
    TooLong,
    OutOfSteps,
    Region,
    OldCli,
    EmptyAnswer,
    Unknown,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Explained {
    pub kind: Kind,
    pub title: String,
    pub body: String,
    pub fixes: Vec<Fix>,
}

fn has(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| text.contains(n))
}

/// A Unix time Claude Code puts after `|` ("usage limit reached|1794650000"), as
/// "in about 2 h 10 min".
fn reset_in(raw: &str, now: u64) -> Option<String> {
    let secs: u64 = raw.split('|').nth(1)?.trim().chars().take_while(char::is_ascii_digit).collect::<String>().parse().ok()?;
    let left = secs.checked_sub(now)?;
    let (h, m) = (left / 3600, (left % 3600).div_ceil(60).min(59));
    Some(match (h, m) {
        (0, m) => format!("in about {} min", m.max(1)),
        (h, 0) => format!("in about {h} h"),
        (h, m) => format!("in about {h} h {m} min"),
    })
}

/// The provider's own words about when a limit resets ("resets 3pm", "try again in 2
/// days 3 hours"), if it gave any.
fn reset_phrase(raw: &str) -> Option<String> {
    let lower = raw.to_lowercase();
    for key in ["resets ", "try again in ", "try again at "] {
        if let Some(i) = lower.find(key) {
            let rest = &raw[i..];
            let end = rest.find(['.', '·', '∙', '\n', '|', '[']).unwrap_or(rest.len());
            let phrase = rest[..end].trim();
            if phrase.len() > key.len() {
                return Some(phrase.to_string());
            }
        }
    }
    None
}

/// Explain a failure reported by `provider`. `now` is the current Unix time (for
/// usage-limit resets).
pub fn explain(provider: Provider, raw: &str, now: u64) -> Explained {
    let t = raw.to_lowercase();
    let (cli, plan, billing, usage, status) = match provider {
        Provider::Claude => (
            "Claude Code",
            "Claude",
            "https://claude.ai/settings/billing",
            "https://claude.ai/settings/usage",
            "https://status.anthropic.com",
        ),
        Provider::Codex => ("Codex", "ChatGPT", "https://chatgpt.com/#settings/Account", "https://chatgpt.com/codex/settings/usage", "https://status.openai.com"),
    };
    let e = |kind, title: String, body: String, fixes: Vec<Fix>| Explained { kind, title, body, fixes };

    // Order matters: billing and limit codes also carry 401/403/429-ish words.
    if has(&t, &["program not found", "cannot find the file", "no such file or directory", "os error 2", "is not recognized as"])
        && has(&t, &["starting", "couldn't start", "spawn"])
    {
        return e(
            Kind::NotInstalled,
            format!("{cli} isn't installed"),
            format!("Studio uses {cli} to write lessons with your {plan} plan, and it couldn't be found on this computer."),
            vec![Fix::Install, Fix::Settings],
        );
    }
    if has(&t, &["oauth_not_allowed_for_organization", "payment is past due", "past due", "credit balance is too low", "insufficient_quota", "exceeded your current quota", "billing", "organization has disabled", "disabled claude code", "subscription"])
        && !has(&t, &["usage limit", "limit reached"])
    {
        return e(
            Kind::Billing,
            format!("Your {plan} plan can't be used right now"),
            format!(
                "{plan} turned the request down because of the account, not the lesson: usually a payment that's due, a plan that has ended, or an organization setting. Check your {plan} account, then try again."
            ),
            vec![Fix::Open { label: "Open billing", url: billing }, Fix::SwitchProvider, Fix::Retry],
        );
    }
    if has(&t, &["usage limit", "limit reached", "usage_limit", "rate_limit", "rate limit", "429", "too many requests", "hit your limit", "weekly limit", "5-hour limit"]) {
        // "It resets in about 2 h." / "It resets 3pm." / "You can try again in 2 days."
        let when = reset_in(raw, now).map(|w| format!("It resets {w}.")).or_else(|| {
            reset_phrase(raw).map(|p| if p.to_lowercase().starts_with("resets") { format!("It {p}.") } else { format!("You can {p}.") })
        });
        let body = match when {
            Some(w) => format!("You've used your {plan} plan's allowance for now. {w} Studio can carry on then, or use the other assistant if it's connected."),
            None => format!("You've used your {plan} plan's allowance for now. Try again once it resets, or use the other assistant if it's connected."),
        };
        return e(Kind::UsageLimit, format!("{plan} usage limit reached"), body, vec![Fix::SwitchProvider, Fix::Open { label: "See usage", url: usage }, Fix::Retry]);
    }
    if has(&t, &["not logged in", "please run /login", "/login", "invalid api key", "authentication_error", "oauth token has expired", "token has expired", "token_expired", "could not be refreshed", "refresh_token", "401", "unauthorized", "sign in again", "log in again", "logged out"]) {
        return e(
            Kind::SignedOut,
            format!("Sign in to {plan} again"),
            format!("{cli} isn't signed in any more (sign-ins expire now and then). Sign in again and retry; your lesson is kept."),
            vec![Fix::SignIn, Fix::Retry],
        );
    }
    if has(&t, &["unsupported_country", "not available in your country", "not available in your region", "region is not supported"]) {
        return e(
            Kind::Region,
            format!("{plan} isn't available here"),
            format!("{plan} doesn't serve requests from your current country or region. A VPN or the other assistant may work."),
            vec![Fix::SwitchProvider],
        );
    }
    if has(&t, &["prompt is too long", "context length", "context_length_exceeded", "context window", "input is too long", "maximum context", "too many tokens"]) {
        return e(
            Kind::TooLong,
            "The request was too long".into(),
            "Together with the attached documents, this was more than the model can read at once. Attach fewer or shorter documents, or split the lesson into smaller videos.".into(),
            vec![Fix::Retry],
        );
    }
    if has(&t, &["issue with the selected model", "model not found", "not_found_error", "does not exist", "not have access to", "model is not supported", "unsupported model", "invalid model", "run /model"]) {
        return e(
            Kind::Model,
            "That model isn't available".into(),
            format!("Your {plan} plan can't use the model picked in Settings. Choose another model under Assistants & usage and try again."),
            vec![Fix::Settings, Fix::Retry],
        );
    }
    if has(&t, &["unknown option", "unexpected argument", "unrecognized option", "unrecognized argument", "unknown command"]) {
        return e(
            Kind::OldCli,
            format!("{cli} needs an update"),
            format!("This version of {cli} doesn't understand what Studio asked it to do. Updating it usually fixes this."),
            vec![Fix::Update, Fix::Retry],
        );
    }
    if has(&t, &["overloaded", "529", "500 ", "502", "503", "504", "internal server error", "server_error", "service unavailable", "bad gateway", "api_error"]) {
        return e(
            Kind::Overloaded,
            format!("{plan} is busy right now"),
            format!("{plan}'s servers had a problem on their side. Nothing is wrong with your lesson; trying again in a minute usually works."),
            vec![Fix::Retry, Fix::Open { label: "Service status", url: status }, Fix::SwitchProvider],
        );
    }
    if has(&t, &["econnrefused", "enotfound", "econnreset", "getaddrinfo", "connection error", "unable to connect", "fetch failed", "network", "timed out", "timeout", "stream disconnected", "error sending request", "dns", "connection reset", "connection refused", "offline"]) {
        return e(
            Kind::Network,
            format!("Couldn't reach {plan}"),
            "The connection dropped or timed out. Check that you're online (and that no firewall or proxy blocks it), then try again.".into(),
            vec![Fix::Retry, Fix::Open { label: "Service status", url: status }],
        );
    }
    if has(&t, &["error_max_turns", "max turns", "maximum number of turns"]) {
        return e(
            Kind::OutOfSteps,
            "The assistant ran out of steps".into(),
            "It took more steps than allowed before finishing. Try again, or ask for a smaller change.".into(),
            vec![Fix::Retry],
        );
    }
    if has(&t, &["ended without an answer", "without an answer"]) {
        return e(
            Kind::EmptyAnswer,
            "The assistant didn't answer".into(),
            format!("{cli} finished without writing anything back. Trying again usually works."),
            vec![Fix::Retry],
        );
    }
    e(
        Kind::Unknown,
        format!("{cli} stopped unexpectedly"),
        format!("{cli} ended with an error Studio doesn't recognise (details below). Try again; if it keeps happening, updating {cli} or using the other assistant may help."),
        vec![Fix::Retry, Fix::Update, Fix::SwitchProvider],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(p: Provider, raw: &str) -> Kind {
        explain(p, raw, 1_794_640_000).kind
    }

    #[test]
    fn claude_errors() {
        use Kind::*;
        let c = Provider::Claude;
        assert_eq!(kind(c, "Claude Code can't use your Claude subscription, usually because a payment is past due [oauth_not_allowed_for_organization, HTTP 403]"), Billing);
        assert_eq!(kind(c, "Credit balance is too low"), Billing);
        assert_eq!(kind(c, "Not logged in · Please run /login"), SignedOut);
        assert_eq!(kind(c, "Invalid API key · Please run /login"), SignedOut);
        assert_eq!(kind(c, "API Error: 401 {\"type\":\"error\",\"error\":{\"type\":\"authentication_error\",\"message\":\"OAuth token has expired.\"}}"), SignedOut);
        assert_eq!(kind(c, "Claude AI usage limit reached|1794647800"), UsageLimit);
        assert_eq!(kind(c, "5-hour limit reached ∙ resets 3pm"), UsageLimit);
        assert_eq!(kind(c, "API Error: 529 {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"Overloaded\"}}"), Overloaded);
        assert_eq!(kind(c, "API Error (Connection error.)"), Network);
        assert_eq!(kind(c, "API Error: Request timed out."), Network);
        assert_eq!(kind(c, "API Error: 400 prompt is too long: 210000 tokens > 200000 maximum"), TooLong);
        assert_eq!(kind(c, "There's an issue with the selected model (claude-x). It may not exist or you may not have access to it. Run /model to pick a different model."), Model);
        assert_eq!(kind(c, "error: unknown option '--effort'"), OldCli);
        assert_eq!(kind(c, "error_max_turns"), OutOfSteps);
        assert_eq!(kind(c, "starting C:\\x\\claude.exe: The system cannot find the file specified. (os error 2)"), NotInstalled);
        assert_eq!(kind(c, "the assistant ended without an answer"), EmptyAnswer);
        assert_eq!(kind(c, "something odd"), Unknown);
    }

    #[test]
    fn codex_errors() {
        use Kind::*;
        let x = Provider::Codex;
        assert_eq!(kind(x, "You've hit your usage limit. Upgrade to Pro or try again in 2 days 3 hours."), UsageLimit);
        assert_eq!(kind(x, "stream error: exceeded retry limit, last status: 429 Too Many Requests"), UsageLimit);
        assert_eq!(kind(x, "unexpected status 401 Unauthorized: Your access token could not be refreshed. Please log out and sign in again."), SignedOut);
        assert_eq!(kind(x, "stream disconnected before completion: error sending request for url (https://chatgpt.com/backend-api/codex/responses)"), Network);
        assert_eq!(kind(x, "unexpected status 400 Bad Request: The 'gpt-x' model is not supported when using Codex with a ChatGPT account."), Model);
        assert_eq!(kind(x, "You exceeded your current quota, please check your plan and billing details."), Billing);
        assert_eq!(kind(x, "context_length_exceeded"), TooLong);
        assert_eq!(kind(x, "unexpected status 503 Service Unavailable"), Overloaded);
        assert_eq!(kind(x, "error: unexpected argument '--foo' found"), OldCli);
        assert_eq!(kind(x, "unsupported_country_region_territory"), Region);
    }

    #[test]
    fn says_when_a_limit_resets() {
        let e = explain(Provider::Claude, "Claude AI usage limit reached|1794647800", 1_794_640_000);
        assert!(e.body.contains("in about 2 h 10 min"), "{}", e.body);
        let e = explain(Provider::Claude, "5-hour limit reached ∙ resets 3pm", 0);
        assert!(e.body.contains("It resets 3pm."), "{}", e.body);
        let e = explain(Provider::Codex, "You've hit your usage limit. Upgrade to Pro or try again in 2 days 3 hours.", 0);
        assert!(e.body.contains("try again in 2 days 3 hours"), "{}", e.body);
    }
}
