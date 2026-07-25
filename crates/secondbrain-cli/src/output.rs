//! How commands say what they did.
//!
//! Every command produces one value that is both `Serialize` — the `--json`
//! machine contract — and renderable as plain text. Writing the two from a
//! single value is what keeps them from drifting into disagreeing about what
//! happened.
//!
//! Neither form ever emits ANSI. This binary writes no escape sequences at all,
//! rather than deciding per-stream whether to, because the only consumer that
//! would benefit is a terminal and the cost of getting the check wrong is a
//! machine contract with control bytes in it.

use std::io::Write;

use serde::Serialize;

use crate::exit::CliError;

/// Which form of output the operator asked for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Format {
    /// Plain text for a person.
    Human,
    /// Stable JSON for a program.
    Json,
}

impl Format {
    /// The format selected by the global `--json` flag.
    #[must_use]
    pub const fn from_flag(json: bool) -> Self {
        if json { Self::Json } else { Self::Human }
    }
}

/// A command's result, in both the forms this binary can speak.
pub trait Report: Serialize {
    /// The human-readable rendering, without a trailing newline and without
    /// ANSI.
    fn render(&self) -> String;
}

/// Writes a report to stdout in the requested form.
pub fn emit(format: Format, report: &impl Report) -> Result<(), CliError> {
    let rendered = match format {
        Format::Human => report.render(),
        Format::Json => serde_json::to_string_pretty(report)?,
    };
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "{rendered}").map_err(|source| CliError::Io {
        operation: "write to",
        path: "stdout".into(),
        source,
    })
}

/// Writes a failure to stderr in the requested form.
///
/// Failures never touch stdout: a caller reading `--json` from a pipe must be
/// able to treat an empty stdout as "this produced nothing", instead of parsing
/// an error object it did not ask for as a result.
pub fn emit_error(format: Format, error: &CliError) {
    let text = match format {
        Format::Human => format!("error [{}]: {error}", error.code()),
        Format::Json => serde_json::json!({
            "error": { "code": error.code(), "message": error.to_string() }
        })
        .to_string(),
    };
    let mut stderr = std::io::stderr().lock();
    let _ = writeln!(stderr, "{text}");
}

/// Renders a count with a correctly pluralized noun.
#[must_use]
pub fn plural(count: usize, singular: &str, plural: &str) -> String {
    if count == 1 {
        format!("{count} {singular}")
    } else {
        format!("{count} {plural}")
    }
}

/// Renders an optional value, naming its absence rather than printing nothing.
#[must_use]
pub fn or_none(value: Option<&String>) -> &str {
    value.map_or("(none)", String::as_str)
}
