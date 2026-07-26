//! `secondbrain graph` - export the versioned graph derived from the index.

use std::path::Path;

use secondbrain_index::WorkspaceGraph;

use crate::exit::{CliError, OK};
use crate::output::{Format, Report, emit, plural};
use crate::workspace::Workspace;

struct GraphReport(WorkspaceGraph);

impl serde::Serialize for GraphReport {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.0.serialize(serializer)
    }
}

impl Report for GraphReport {
    fn render(&self) -> String {
        format!(
            "Workspace graph\n  {}, {}\n  {}, {} ambiguous",
            plural(self.0.nodes.len(), "node", "nodes"),
            plural(self.0.edges.len(), "edge", "edges"),
            plural(self.0.broken_links.len(), "broken target", "broken targets"),
            self.0.ambiguous_links.len(),
        )
    }
}

pub fn run(format: Format, workspace: &Path) -> Result<u8, CliError> {
    let workspace = Workspace::open(workspace)?;
    let graph = workspace.open_index()?.workspace_graph()?;
    emit(format, &GraphReport(graph))?;
    Ok(OK)
}
