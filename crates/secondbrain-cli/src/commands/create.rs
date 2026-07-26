//! Explicit note and daily-note creation commands.

use std::path::Path;

use secondbrain_core::actor::{ActorId, DeviceId};
use secondbrain_transaction::{
    DailyDate, DailyNote, NoteCreateOutcome, NoteCreatePreview, apply_note_creation,
    open_or_preview_daily_note, preview_note_creation,
};

use crate::commands::{CLI_ACTOR, CLI_DEVICE};
use crate::exit::{CliError, OK, read_file};
use crate::output::{Format, Report, emit};

impl Report for NoteCreatePreview {
    fn render(&self) -> String {
        format!("{}\n  note id: {}", self.summary, self.note_id)
    }
}

impl Report for NoteCreateOutcome {
    fn render(&self) -> String {
        format!(
            "{} {}\n  note id: {}",
            if self.created {
                "Created"
            } else {
                "Already created"
            },
            self.path,
            self.note_id
        )
    }
}

impl Report for DailyNote {
    fn render(&self) -> String {
        match self {
            Self::Existing { path, .. } => format!("Open existing daily note {path}"),
            Self::Create { preview } => {
                format!("Previewed daily note creation for {}", preview.path)
            }
        }
    }
}

fn identities() -> Result<(ActorId, DeviceId), CliError> {
    Ok((ActorId::new(CLI_ACTOR)?, DeviceId::new(CLI_DEVICE)?))
}

pub fn preview(
    format: Format,
    workspace: &Path,
    path: &str,
    source_file: &Path,
    out: Option<&Path>,
) -> Result<u8, CliError> {
    let source = read_file("read note source", source_file)?;
    let (actor, device) = identities()?;
    let preview = preview_note_creation(workspace, path, &source, actor, device)?;
    output_preview(format, &preview, out)
}

pub fn daily(
    format: Format,
    workspace: &Path,
    date: &str,
    out: Option<&Path>,
) -> Result<u8, CliError> {
    let (actor, device) = identities()?;
    let daily = open_or_preview_daily_note(workspace, DailyDate::new(date)?, actor, device)?;
    if let (DailyNote::Create { preview }, Some(out)) = (&daily, out) {
        std::fs::write(out, serde_json::to_string_pretty(preview)?).map_err(|source| {
            CliError::Io {
                operation: "write note creation preview to",
                path: out.to_path_buf(),
                source,
            }
        })?;
    } else {
        emit(format, &daily)?;
    }
    Ok(OK)
}

fn output_preview(
    format: Format,
    preview: &NoteCreatePreview,
    out: Option<&Path>,
) -> Result<u8, CliError> {
    if let Some(out) = out {
        std::fs::write(out, serde_json::to_string_pretty(preview)?).map_err(|source| {
            CliError::Io {
                operation: "write note creation preview to",
                path: out.to_path_buf(),
                source,
            }
        })?;
    } else {
        emit(format, preview)?;
    }
    Ok(OK)
}

pub fn apply(format: Format, workspace: &Path, preview_file: &Path) -> Result<u8, CliError> {
    let preview: NoteCreatePreview =
        serde_json::from_str(&read_file("read note creation preview", preview_file)?)
            .map_err(|source| CliError::PlanUnreadable { source })?;
    if preview.actor.as_str() != CLI_ACTOR || preview.device.as_str() != CLI_DEVICE {
        return Err(CliError::ReviewRequired(
            "note creation preview attribution is not the local CLI".into(),
        ));
    }
    emit(format, &apply_note_creation(workspace, &preview)?)?;
    Ok(OK)
}
