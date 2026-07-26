<script lang="ts">
  import { noteLabel, workspaceName } from './lib/presentation';
  import {
    openWorkspace,
    readNote,
    searchWorkspace,
    type NoteDocument,
    type NoteSummary,
    type SearchHit,
    type WorkspaceSummary
  } from './lib/workspace';

  let root = $state('');
  let query = $state('');
  let workspace = $state<WorkspaceSummary | null>(null);
  let results = $state<SearchHit[]>([]);
  let selected = $state<NoteSummary | null>(null);
  let document = $state<NoteDocument | null>(null);
  let busy = $state(false);
  let error = $state('');

  async function open() {
    if (!root.trim()) return;
    busy = true;
    error = '';
    try {
      workspace = await openWorkspace(root.trim());
      results = [];
      const first = workspace.notes[0] ?? null;
      selected = first;
      document = null;
      if (first) await load(first);
    } catch (cause) {
      error = String(cause);
    } finally {
      busy = false;
    }
  }

  async function search() {
    if (!workspace || !query.trim()) {
      results = [];
      return;
    }
    busy = true;
    error = '';
    try {
      results = await searchWorkspace(workspace.root, query.trim());
    } catch (cause) {
      error = String(cause);
    } finally {
      busy = false;
    }
  }

  async function load(note: NoteSummary) {
    if (!workspace) return;
    const workspaceRoot = workspace.root;
    document = null;
    busy = true;
    error = '';
    try {
      const loaded = await readNote(workspaceRoot, note.path);
      if (selected?.noteId === note.noteId && workspace?.root === workspaceRoot) document = loaded;
    } catch (cause) {
      error = String(cause);
    } finally {
      busy = false;
    }
  }

  function choose(note: NoteSummary) {
    selected = note;
    void load(note);
  }
</script>

<svelte:head><title>{workspace ? workspaceName(workspace.root) : 'SecondBrain'}</title></svelte:head>

<main class:unopened={!workspace}>
  {#if !workspace}
    <section class="welcome" aria-labelledby="welcome-title">
      <p class="eyebrow">LOCAL KNOWLEDGE, DURABLE BY DEFAULT</p>
      <h1 id="welcome-title">Open your<br /><em>second brain.</em></h1>
      <p class="lede">Markdown stays yours. The index is disposable. Every future edit will pass through one recoverable transaction engine.</p>
      <form onsubmit={(event) => { event.preventDefault(); open(); }}>
        <label for="workspace-root">Workspace folder</label>
        <div class="open-row">
          <input id="workspace-root" bind:value={root} placeholder="/Users/you/Notes" autocomplete="off" />
          <button disabled={busy || !root.trim()}>{busy ? 'Opening...' : 'Open workspace'}</button>
        </div>
      </form>
      {#if error}<p class="error" role="alert">{error}</p>{/if}
    </section>
  {:else}
    <header>
      <div class="brand"><span class="mark">S</span><strong>SecondBrain</strong></div>
      <form class="search" onsubmit={(event) => { event.preventDefault(); search(); }}>
        <span aria-hidden="true">⌕</span>
        <input aria-label="Search workspace" bind:value={query} placeholder="Search this workspace" />
        <kbd>↵</kbd>
      </form>
      <div class="health"><i></i> local</div>
    </header>

    <aside>
      <div class="workspace-title">
        <p>WORKSPACE</p>
        <h2>{workspaceName(workspace.root)}</h2>
        <span>{workspace.indexed} notes · {workspace.brokenLinks} broken links</span>
      </div>
      <nav aria-label="Notes">
        <p>{results.length ? `RESULTS · ${results.length}` : `ALL NOTES · ${workspace.notes.length}`}</p>
        {#each results.length ? results : workspace.notes as note (note.noteId)}
          <button class:active={selected?.noteId === note.noteId} onclick={() => choose(note)}>
            <span>{noteLabel(note)}</span><small>{note.path}</small>
          </button>
        {/each}
      </nav>
    </aside>

    <section class="canvas">
      {#if selected}
        <p class="path">{selected.path}</p>
        <h1>{noteLabel(selected)}</h1>
        <div class="rule"></div>
        {#if document}
          <pre class="note-source" aria-label="Markdown source">{document.source}</pre>
        {:else if busy}
          <p class="placeholder">Loading Markdown source...</p>
        {:else if !error}
          <p class="placeholder">Select the note again to read its Markdown source.</p>
        {/if}
        <dl>
          <div><dt>Note identity</dt><dd>{selected.noteId}</dd></div>
          <div><dt>Derived index</dt><dd>Ready</dd></div>
          <div><dt>Workspace state</dt><dd>Local only</dd></div>
        </dl>
      {:else}
        <p class="placeholder">This workspace contains no Markdown notes yet.</p>
      {/if}
      {#if error}<p class="error" role="alert">{error}</p>{/if}
    </section>

    <footer><span>{busy ? 'Working…' : 'Ready'}</span><span>SecondBrain · Phase 1</span></footer>
  {/if}
</main>
