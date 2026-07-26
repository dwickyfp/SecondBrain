<script lang="ts">
  import { noteLabel } from './presentation';
  import type { NoteSummary } from './workspace';
  import type { WorkspaceGraphV1 } from './graph';

  let { graph, onOpen }: { graph: WorkspaceGraphV1; onOpen: (note: NoteSummary) => void } = $props();
</script>

<section class="graph-view" aria-labelledby="graph-title">
  <header class="graph-heading">
    <div><p>DERIVED WORKSPACE MAP</p><h1 id="graph-title">Graph</h1></div>
    <p>{graph.nodes.length} nodes · {graph.edges.length} edges</p>
  </header>
  {#if graph.nodes.length}
    <div class="graph-list" role="region" aria-label="Workspace graph table">
      <table>
        <thead><tr><th scope="col">Note</th><th scope="col">Incoming</th><th scope="col">Outgoing</th><th scope="col">Status</th></tr></thead>
        <tbody>
          {#each graph.nodes as node (node.noteId)}
            <tr>
              <th scope="row"><button onclick={() => onOpen(node)}><span>{noteLabel(node)}</span><small>{node.path}</small></button></th>
              <td>{node.incoming_occurrences}</td><td>{node.outgoing_occurrences}</td><td>{node.orphan ? 'Orphan' : 'Connected'}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
    <section class="graph-diagnostics" aria-labelledby="graph-diagnostics-title">
      <h2 id="graph-diagnostics-title">Link diagnostics</h2>
      {#if !graph.broken_links.length && !graph.ambiguous_links.length}<p>No unresolved links.</p>{/if}
      {#each graph.broken_links as link}<p><strong>Broken:</strong> {link.source_path} → {link.target} ({link.occurrences})</p>{/each}
      {#each graph.ambiguous_links as link}<p><strong>Ambiguous:</strong> {link.source_path} → {link.target} ({link.occurrences}); candidates: {link.candidates.map((candidate) => candidate.path).join(', ')}</p>{/each}
    </section>
  {:else}
    <p class="graph-empty">The index contains no notes to graph.</p>
  {/if}
</section>
