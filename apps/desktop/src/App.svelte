<script lang="ts">
  import { tick } from 'svelte';
  import {
    createCommandRegistry,
    shouldSuppressShortcut,
    type CommandPlatform
  } from './lib/commands';
  import { noteLabel, workspaceName } from './lib/presentation';
  import {
    activeNote,
    closeSplit as closeNavigationSplit,
    closeTab as closeNavigationTab,
    createNavigation,
    goBack as navigateBack,
    goForward as navigateForward,
    openNote as openNavigationNote,
    splitPane as splitNavigationPane,
    type NavigationState,
    type PaneId
  } from './lib/navigation';
  import {
    openWorkspace,
    readNote,
    readNoteContext,
    searchWorkspace,
    type NoteContext,
    type NoteDocument,
    type NoteSummary,
    type SearchHit,
    type WorkspaceSummary
  } from './lib/workspace';

  let root = $state('');
  let query = $state('');
  let workspace = $state<WorkspaceSummary | null>(null);
  let results = $state<SearchHit[]>([]);
  let navigation = $state<NavigationState>(createNavigation());
  let documents = $state<Record<string, NoteDocument>>({});
  let contexts = $state<Record<string, NoteContext>>({});
  let paneErrors = $state<Record<PaneId, string>>({ primary: '', secondary: '' });
  let paneBusy = $state<Record<PaneId, boolean>>({ primary: false, secondary: false });
  let requests: Record<PaneId, number> = { primary: 0, secondary: 0 };
  let busy = $state(false);
  let error = $state('');
  let paletteOpen = $state(false);
  let paletteQuery = $state('');
  let paletteIndex = $state(0);
  let paletteInput = $state<HTMLInputElement | null>(null);
  let searchInput = $state<HTMLInputElement | null>(null);
  let returnFocus = $state<HTMLElement | null>(null);

  type CommandId = 'palette' | 'search' | 'split' | 'close-split' | 'back' | 'forward' | 'close-tab';
  const platform: CommandPlatform = typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform)
    ? 'macos'
    : typeof navigator !== 'undefined' && /Win/.test(navigator.platform)
      ? 'windows'
      : 'linux';
  const commandRegistry = createCommandRegistry<CommandId, { workspace: boolean; split: boolean; note: boolean }>([
    { id: 'palette', label: 'Open command palette', shortcut: 'Mod+K' },
    { id: 'search', label: 'Focus workspace search', shortcut: 'Mod+P', enabled: (context) => context.workspace },
    { id: 'split', label: 'Split active pane', shortcut: 'Mod+\\', enabled: (context) => context.workspace && !context.split },
    { id: 'close-split', label: 'Close split pane', enabled: (context) => context.split },
    { id: 'back', label: 'Navigate back', shortcut: 'Alt+ArrowLeft', enabled: (context) => context.note },
    { id: 'forward', label: 'Navigate forward', shortcut: 'Alt+ArrowRight', enabled: (context) => context.note },
    { id: 'close-tab', label: 'Close active tab', shortcut: 'Mod+W', enabled: (context) => context.note }
  ], platform);

  let selected = $derived(
    activeNote(navigation.panes.find((pane) => pane.id === navigation.activePaneId) ?? navigation.panes[0])
  );
  let commandContext = $derived({ workspace: workspace !== null, split: navigation.panes.length === 2, note: selected !== null });
  let paletteMatches = $derived(commandRegistry.match(paletteQuery)
    .filter(({ command }) => commandRegistry.isEnabled(command.id, commandContext)));

  async function open() {
    if (!root.trim()) return;
    busy = true;
    error = '';
    try {
      workspace = await openWorkspace(root.trim());
      results = [];
      const first = workspace.notes[0] ?? null;
      navigation = createNavigation(first ?? undefined);
      documents = {};
      contexts = {};
      paneErrors = { primary: '', secondary: '' };
      if (first) await load(first, 'primary');
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

  async function load(note: NoteSummary, paneId: PaneId) {
    if (!workspace) return;
    const workspaceRoot = workspace.root;
    const request = ++requests[paneId];
    paneBusy[paneId] = true;
    paneErrors[paneId] = '';
    try {
      const [loaded, context] = await Promise.all([
        readNote(workspaceRoot, note.path),
        readNoteContext(workspaceRoot, note.noteId)
      ]);
      const pane = navigation.panes.find((candidate) => candidate.id === paneId);
      if (request === requests[paneId] && pane && activeNote(pane)?.noteId === note.noteId && workspace?.root === workspaceRoot) {
        documents[loaded.noteId] = loaded;
        contexts[context.noteId] = context;
      }
    } catch (cause) {
      const pane = navigation.panes.find((candidate) => candidate.id === paneId);
      if (request === requests[paneId] && pane && activeNote(pane)?.noteId === note.noteId) {
        paneErrors[paneId] = String(cause);
      }
    } finally {
      if (request === requests[paneId]) paneBusy[paneId] = false;
    }
  }

  function choose(note: NoteSummary) {
    const paneId = navigation.activePaneId;
    navigation = openNavigationNote(navigation, note, paneId);
    void load(note, paneId);
  }

  function selectTab(note: NoteSummary, paneId: PaneId) {
    navigation = openNavigationNote(navigation, note, paneId);
    void load(note, paneId);
  }

  function closeTab(noteId: string, paneId: PaneId) {
    navigation = closeNavigationTab(navigation, noteId, paneId);
    const pane = navigation.panes.find((candidate) => candidate.id === paneId);
    const note = pane ? activeNote(pane) : null;
    if (note && !documents[note.noteId]) void load(note, paneId);
  }

  function split() {
    navigation = splitNavigationPane(navigation);
    const pane = navigation.panes.find((candidate) => candidate.id === 'secondary');
    const note = pane ? activeNote(pane) : null;
    if (note && !documents[note.noteId]) void load(note, 'secondary');
  }

  function closeSplit() {
    navigation = closeNavigationSplit(navigation);
  }

  function moveHistory(paneId: PaneId, direction: 'back' | 'forward') {
    navigation = direction === 'back'
      ? navigateBack(navigation, paneId)
      : navigateForward(navigation, paneId);
    const pane = navigation.panes.find((candidate) => candidate.id === paneId);
    const note = pane ? activeNote(pane) : null;
    if (note && !documents[note.noteId]) void load(note, paneId);
  }

  function executeCommand(id: CommandId) {
    if (!commandRegistry.isEnabled(id, commandContext)) return;
    if (id === 'palette') void openPalette();
    else if (id === 'search') searchInput?.focus();
    else if (id === 'split') split();
    else if (id === 'close-split') closeSplit();
    else if (id === 'back') moveHistory(navigation.activePaneId, 'back');
    else if (id === 'forward') moveHistory(navigation.activePaneId, 'forward');
    else if (id === 'close-tab' && selected) closeTab(selected.noteId, navigation.activePaneId);
    if (id !== 'palette') closePalette();
  }

  async function openPalette() {
    returnFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    paletteOpen = true;
    paletteQuery = '';
    paletteIndex = 0;
    await tick();
    paletteInput?.focus();
  }

  function closePalette() {
    if (!paletteOpen) return;
    paletteOpen = false;
    void tick().then(() => returnFocus?.focus());
  }

  function handleKeydown(event: KeyboardEvent) {
    if (paletteOpen) {
      if (event.key === 'Escape') { event.preventDefault(); closePalette(); }
      else if (event.key === 'ArrowDown') { event.preventDefault(); paletteIndex = Math.min(paletteIndex + 1, paletteMatches.length - 1); }
      else if (event.key === 'ArrowUp') { event.preventDefault(); paletteIndex = Math.max(paletteIndex - 1, 0); }
      else if (event.key === 'Enter' && paletteMatches[paletteIndex]) { event.preventDefault(); executeCommand(paletteMatches[paletteIndex].command.id); }
      return;
    }
    if (shouldSuppressShortcut(event)) return;
    const command = commandRegistry.commandForShortcut(event);
    if (command && commandRegistry.isEnabled(command.id, commandContext)) {
      event.preventDefault();
      executeCommand(command.id);
    }
  }
</script>

<svelte:window onkeydown={handleKeydown} />

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
        <input bind:this={searchInput} aria-label="Search workspace" bind:value={query} placeholder="Search this workspace" />
        <kbd>↵</kbd>
      </form>
      {#if error}<p class="error global-error" role="alert">{error}</p>{/if}
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

    <section class:split={navigation.panes.length === 2} class="workspace-panes" aria-label="Document workspace">
      {#each navigation.panes as pane (pane.id)}
        {@const paneNote = activeNote(pane)}
        {@const document = paneNote ? documents[paneNote.noteId] : null}
        <article
          class:active-pane={navigation.activePaneId === pane.id}
          class="pane"
          aria-label={pane.id === 'primary' ? 'Primary pane' : 'Secondary pane'}
        >
          <div class="pane-bar">
            <div class="history-controls" aria-label={`${pane.id} navigation history`}>
              <button aria-label={`Back in ${pane.id} pane`} disabled={pane.historyIndex <= 0} onclick={() => moveHistory(pane.id, 'back')}>←</button>
              <button aria-label={`Forward in ${pane.id} pane`} disabled={pane.historyIndex >= pane.history.length - 1} onclick={() => moveHistory(pane.id, 'forward')}>→</button>
            </div>
            <div class="tabs" role="tablist" aria-label={`${pane.id} pane tabs`}>
              {#each pane.tabs as tab (tab.noteId)}
                <div class:active={pane.activeNoteId === tab.noteId} class="tab">
                  <button role="tab" aria-selected={pane.activeNoteId === tab.noteId} onclick={() => selectTab(tab, pane.id)}>{noteLabel(tab)}</button>
                  <button class="close-tab" aria-label={`Close ${noteLabel(tab)}`} onclick={(event) => { event.stopPropagation(); closeTab(tab.noteId, pane.id); }}>×</button>
                </div>
              {/each}
            </div>
            <button class="split-control" onclick={navigation.panes.length === 1 ? split : closeSplit}>
              {navigation.panes.length === 1 ? 'Split' : 'Close split'}
            </button>
          </div>
          <div class="canvas">
            {#if paneNote}
              <p class="path">{paneNote.path}</p>
              <h1>{noteLabel(paneNote)}</h1>
              <div class="rule"></div>
              {#if document}
                <pre class="note-source" aria-label={`${noteLabel(paneNote)} Markdown source`}>{document.source}</pre>
              {:else if paneBusy[pane.id]}
                <p class="placeholder">Loading Markdown source...</p>
              {:else if !paneErrors[pane.id]}
                <p class="placeholder">Select the note again to read its Markdown source.</p>
              {/if}
              <dl>
                <div><dt>Note identity</dt><dd>{paneNote.noteId}</dd></div>
                <div><dt>Derived index</dt><dd>Ready</dd></div>
                <div><dt>Workspace state</dt><dd>Local only</dd></div>
              </dl>
            {:else}
              <p class="placeholder">{workspace.notes.length ? 'Open a note in this pane.' : 'This workspace contains no Markdown notes yet.'}</p>
            {/if}
            {#if paneErrors[pane.id]}<p class="error" role="alert">{paneErrors[pane.id]}</p>{/if}
          </div>
        </article>
      {/each}
    </section>

    <aside class="context-panel" aria-label="Note context">
      {#if selected && contexts[selected.noteId]}
        {@const context = contexts[selected.noteId]}
        <section aria-labelledby="outline-title"><h2 id="outline-title">Outline</h2>
          {#if context.outline.length}<ol class="outline-list">{#each context.outline as heading}<li style={`--level: ${heading.level}`}><button title={`Line ${heading.line}`}>{heading.text}</button></li>{/each}</ol>
          {:else}<p class="context-empty">No headings</p>{/if}
        </section>
        <section aria-labelledby="backlinks-title"><h2 id="backlinks-title">Backlinks</h2>
          {#if context.backlinks.length}<ul class="backlink-list">{#each context.backlinks as backlink (backlink.noteId)}<li><button onclick={() => choose(backlink)}><span>{noteLabel(backlink)}</span><small>{backlink.path}</small></button></li>{/each}</ul>
          {:else}<p class="context-empty">No backlinks</p>{/if}
        </section>
      {:else if selected && paneBusy[navigation.activePaneId]}<p class="context-empty">Loading context...</p>
      {:else}<p class="context-empty">Select a note</p>{/if}
    </aside>

    <footer><span>{busy || paneBusy.primary || paneBusy.secondary ? 'Working…' : 'Ready'}</span><span>SecondBrain · Phase 1</span></footer>
  {/if}
  {#if paletteOpen}
    <div class="palette-backdrop" role="presentation" onclick={(event) => { if (event.target === event.currentTarget) closePalette(); }}>
      <div class="command-palette" role="dialog" aria-modal="true" aria-labelledby="palette-title">
        <h2 id="palette-title">Command palette</h2>
        <input bind:this={paletteInput} aria-label="Find a command" bind:value={paletteQuery} oninput={() => { paletteIndex = 0; }} autocomplete="off" />
        <ul role="listbox" aria-label="Commands">{#each paletteMatches as match, index (match.command.id)}<li><button class:active={index === paletteIndex} role="option" aria-selected={index === paletteIndex} onclick={() => executeCommand(match.command.id)}>{match.command.label}<kbd>{match.command.normalizedShortcut?.signature ?? ''}</kbd></button></li>{:else}<li class="context-empty">No matching commands</li>{/each}</ul>
      </div>
    </div>
  {/if}
</main>
