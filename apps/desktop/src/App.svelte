<script lang="ts">
  import { tick } from 'svelte';
  import {
    createCommandRegistry,
    shouldSuppressShortcut,
    type CommandPlatform
  } from './lib/commands';
  import { noteLabel, workspaceName } from './lib/presentation';
  import SourceEditor from './lib/SourceEditor.svelte';
  import WysiwygEditor from './lib/WysiwygEditor.svelte';
  import GraphView from './lib/GraphView.svelte';
  import { readWorkspaceGraph, type WorkspaceGraphV1 } from './lib/graph';
  import { inspectWorkspace, recoverWorkspace, type RecoveryReportV1, type WorkspaceReportV1 } from './lib/diagnostics';
  import { applyVaultImport, previewVaultImport, type ImportPreviewV1 } from './lib/import';
  import { chooseWorkspaceDirectory } from './lib/dialog';
  import { NoteSession, type EditorMode, type TextSelection } from './lib/note-session';
  import { applyNoteCreation, createTauriTransactionAdapter, previewDailyNote, type DailyNoteV1 } from './lib/transactions';
  import {
    applyProperty,
    previewProperty,
    readProperties,
    type PropertyEdit,
    type PropertyPreviewV1,
    type PropertyValue
  } from './lib/properties';
  import {
    activeNote,
    closeSplit as closeNavigationSplit,
    closeTab as closeNavigationTab,
    createNavigation,
    focusPane as focusNavigationPane,
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
  const sessions = new Map<string, NoteSession>();
  let sessionRevision = $state(0);
  let contexts = $state<Record<string, NoteContext>>({});
  let properties = $state<Record<string, Record<string, PropertyValue>>>({});
  let propertyValues = $state<Record<string, string>>({});
  let propertyKey = $state('');
  let propertyPreview = $state<PropertyPreviewV1 | null>(null);
  let propertyBusy = $state(false);
  let propertyError = $state('');
  let paneErrors = $state<Record<PaneId, string>>({ primary: '', secondary: '' });
  let paneBusy = $state<Record<PaneId, boolean>>({ primary: false, secondary: false });
  let requests: Record<PaneId, number> = { primary: 0, secondary: 0 };
  let busy = $state(false);
  let error = $state('');
  let importPreview = $state<ImportPreviewV1 | null>(null);
  let importBusy = $state(false);
  let chooseBusy = $state(false);
  let paletteOpen = $state(false);
  let paletteQuery = $state('');
  let paletteIndex = $state(0);
  let paletteInput = $state<HTMLInputElement | null>(null);
  let searchInput = $state<HTMLInputElement | null>(null);
  let returnFocus = $state<HTMLElement | null>(null);
  let view = $state<'notes' | 'graph'>('notes');
  let graph = $state<WorkspaceGraphV1 | null>(null);
  let graphBusy = $state(false);
  let graphError = $state('');
  let health = $state<WorkspaceReportV1 | null>(null);
  let recovery = $state<RecoveryReportV1 | null>(null);
  let healthOpen = $state(false);
  let healthBusy = $state(false);
  let healthError = $state('');
  let paletteDialog = $state<HTMLElement | null>(null);

  type CommandId = 'palette' | 'search' | 'split' | 'close-split' | 'back' | 'forward' | 'close-tab' | 'daily-note' | 'graph' | 'primary-pane' | 'secondary-pane' | 'health';
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
    ,{ id: 'daily-note', label: 'Open today\'s daily note', enabled: (context) => context.workspace },
    { id: 'graph', label: 'Open workspace graph', enabled: (context) => context.workspace },
    { id: 'primary-pane', label: 'Switch to primary pane', shortcut: 'Control+1', enabled: (context) => context.workspace },
    { id: 'secondary-pane', label: 'Switch to secondary pane', shortcut: 'Control+2', enabled: (context) => context.split },
    { id: 'health', label: 'Inspect workspace health', enabled: (context) => context.workspace }
  ], platform);

  let selected = $derived(
    activeNote(navigation.panes.find((pane) => pane.id === navigation.activePaneId) ?? navigation.panes[0])
  );
  let commandContext = $derived({ workspace: workspace !== null, split: navigation.panes.length === 2, note: selected !== null });
  let paletteMatches = $derived(commandRegistry.match(paletteQuery)
    .filter(({ command }) => commandRegistry.isEnabled(command.id, commandContext)));
  let workStatus = $derived(graphBusy ? 'Loading workspace graph...'
    : healthBusy ? 'Inspecting workspace state...'
      : busy || paneBusy.primary || paneBusy.secondary ? 'Working...'
         : 'Ready');

  function messageFrom(cause: unknown): string {
    if (typeof cause === 'string') return cause;
    if (cause instanceof Error) return cause.message;
    if (cause && typeof cause === 'object') {
      const value = cause as { message?: unknown; error?: unknown };
      if (typeof value.message === 'string') return value.message;
      if (typeof value.error === 'string') return value.error;
      try { return JSON.stringify(cause); } catch { /* fall through */ }
    }
    return 'Something went wrong. Please try again.';
  }

  function updateRoot(value: string) {
    root = value;
    importPreview = null;
    error = '';
  }

  async function chooseFolder() {
    chooseBusy = true;
    error = '';
    try {
      const selected = await chooseWorkspaceDirectory();
      if (selected) updateRoot(selected);
    } catch (cause) {
      error = `Could not choose a folder: ${messageFrom(cause)}`;
    } finally {
      chooseBusy = false;
    }
  }

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
      sessions.clear();
      sessionRevision += 1;
      contexts = {};
      properties = {};
      propertyValues = {};
      propertyKey = '';
      propertyPreview = null;
      propertyError = '';
      view = 'notes';
      graph = null;
      graphError = '';
      paneErrors = { primary: '', secondary: '' };
      if (first) await load(first, 'primary');
      await inspectHealth();
    } catch (cause) {
      const message = messageFrom(cause);
      if (/read manifest|manifest.*(?:missing|not found)|not initialized/i.test(message)) {
        try {
          importPreview = await previewVaultImport(root.trim());
          root = importPreview.root;
          error = importPreview.canApply
            ? 'This folder is not initialized yet. Review the vault summary below, then initialize and open it.'
            : 'This folder is not initialized and cannot be adopted until the reported vault issues are resolved.';
        } catch (previewCause) {
          error = `This is not an initialized SecondBrain workspace. Import review also failed: ${messageFrom(previewCause)}`;
        }
      } else {
        error = message;
      }
    } finally {
      busy = false;
    }
  }

  async function previewImport() {
    if (!root.trim()) return;
    importBusy = true;
    error = '';
    importPreview = null;
    try {
      importPreview = await previewVaultImport(root.trim());
      root = importPreview.root;
    } catch (cause) {
      error = messageFrom(cause);
    } finally {
      importBusy = false;
    }
  }

  async function applyImport() {
    if (!importPreview) return;
    importBusy = true;
    error = '';
    try {
      const outcome = await applyVaultImport(importPreview.root, importPreview);
      root = outcome.root;
      importPreview = null;
      await open();
    } catch (cause) {
      error = messageFrom(cause);
    } finally {
      importBusy = false;
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

  function localDate(): string {
    const now = new Date();
    const pad = (value: number) => String(value).padStart(2, '0');
    return `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`;
  }

  async function openDailyNote() {
    if (!workspace) return;
    error = '';
    try {
      const daily = await previewDailyNote(workspace.root, localDate());
      let path: string;
      if (daily.status === 'existing') {
        path = daily.path;
      } else {
        if (!window.confirm(`Create ${daily.preview.path}?`)) return;
        await applyNoteCreation(workspace.root, daily.preview);
        const refreshed = await openWorkspace(workspace.root);
        workspace = refreshed;
        if (graph) await refreshGraph();
        path = daily.preview.path;
      }
      const note = workspace.notes.find((candidate) => candidate.path === path);
      if (note) choose(note);
    } catch (cause) {
      error = String(cause);
    }
  }

  async function load(note: NoteSummary, paneId: PaneId) {
    if (!workspace) return;
    const workspaceRoot = workspace.root;
    const request = ++requests[paneId];
    paneBusy[paneId] = true;
    paneErrors[paneId] = '';
    try {
      const [loaded, context, loadedProperties] = await Promise.all([
        readNote(workspaceRoot, note.path),
        readNoteContext(workspaceRoot, note.noteId),
        Promise.resolve(readProperties(workspaceRoot, note.path)).then((value) => value ?? {}).catch(() => ({}))
      ]);
      const pane = navigation.panes.find((candidate) => candidate.id === paneId);
      if (request === requests[paneId] && pane && activeNote(pane)?.noteId === note.noteId && workspace?.root === workspaceRoot) {
        documents[loaded.noteId] = loaded;
        contexts[context.noteId] = context;
        properties[loaded.noteId] = loadedProperties;
        for (const [key, value] of Object.entries(loadedProperties)) {
          propertyValues[`${loaded.noteId}:${key}`] = JSON.stringify(value);
        }
        if (!sessions.has(loaded.noteId)) {
          sessions.set(loaded.noteId, new NoteSession({
            noteId: loaded.noteId,
            path: loaded.path,
            source: loaded.source,
            revision: loaded.version,
            adapter: createTauriTransactionAdapter(workspaceRoot),
            actor: 'secondbrain-desktop',
            device: 'local-desktop'
          }));
          sessionRevision += 1;
        }
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
    view = 'notes';
    const paneId = navigation.activePaneId;
    navigation = openNavigationNote(navigation, note, paneId);
    propertyPreview = null;
    propertyError = '';
    void load(note, paneId);
  }

  function selectTab(note: NoteSummary, paneId: PaneId) {
    navigation = openNavigationNote(navigation, note, paneId);
    propertyPreview = null;
    propertyError = '';
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

  async function activatePane(paneId: PaneId, focus = false) {
    navigation = focusNavigationPane(navigation, paneId);
    if (focus) {
      await tick();
      document.querySelector<HTMLElement>(`[data-pane-content="${paneId}"]`)?.focus();
    }
  }

  async function cyclePane() {
    if (navigation.panes.length < 2) return;
    await activatePane(navigation.activePaneId === 'primary' ? 'secondary' : 'primary', true);
  }

  function handleTabKey(event: KeyboardEvent, paneId: PaneId, noteId: string) {
    if (!['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) return;
    const pane = navigation.panes.find((candidate) => candidate.id === paneId);
    if (!pane?.tabs.length) return;
    event.preventDefault();
    const current = pane.tabs.findIndex((tab) => tab.noteId === noteId);
    const index = event.key === 'Home' ? 0
      : event.key === 'End' ? pane.tabs.length - 1
        : (current + (event.key === 'ArrowRight' ? 1 : -1) + pane.tabs.length) % pane.tabs.length;
    selectTab(pane.tabs[index], paneId);
    void tick().then(() => document.getElementById(`tab-${paneId}-${pane.tabs[index].noteId}`)?.focus());
  }

  function moveHistory(paneId: PaneId, direction: 'back' | 'forward') {
    navigation = direction === 'back'
      ? navigateBack(navigation, paneId)
      : navigateForward(navigation, paneId);
    const pane = navigation.panes.find((candidate) => candidate.id === paneId);
    const note = pane ? activeNote(pane) : null;
    if (note && !documents[note.noteId]) void load(note, paneId);
  }

  function refreshSession() {
    sessionRevision += 1;
  }

  function sessionView(session: NoteSession | undefined) {
    sessionRevision;
    if (!session) return null;
    return {
      session,
      mode: session.mode,
      status: session.status,
      dirty: session.dirty,
      preview: session.preview,
      error: session.error,
      source: session.draftSource,
      selection: session.selection,
      draftRevision: session.draftRevision
    };
  }

  function setMode(session: NoteSession, mode: EditorMode) {
    if (session.setMode(mode)) refreshSession();
  }

  function editSource(session: NoteSession, source: string, selection: TextSelection) {
    if (session.updateFromSource(source, selection)) refreshSession();
  }

  async function openOutlineHeading(session: NoteSession, line: number) {
    const lines = session.draftSource.split('\n');
    const offset = lines.slice(0, Math.max(0, line - 1)).reduce((total, item) => total + item.length + 1, 0);
    session.selection = { anchor: offset, head: offset };
    setMode(session, 'source');
    await tick();
    document.querySelector<HTMLElement>('.active-pane .cm-content')?.focus();
  }

  async function previewSession(session: NoteSession) {
    const pending = session.requestPreview();
    refreshSession();
    await pending;
    refreshSession();
  }

  async function applySession(session: NoteSession, paneId: PaneId) {
    const pending = session.applyPreview();
    refreshSession();
    const outcome = await pending;
    refreshSession();
    if (!outcome || !workspace) return;
    const note = workspace.notes.find((candidate) => candidate.noteId === session.noteId);
    if (!note) return;
    sessions.delete(session.noteId);
    refreshSession();
    delete documents[session.noteId];
    delete contexts[session.noteId];
    await load(note, paneId);
    if (graph) await refreshGraph();
  }

  async function previewPropertyEdit(note: NoteSummary, edit: PropertyEdit) {
    if (!workspace || sessions.get(note.noteId)?.dirty) return;
    propertyBusy = true;
    propertyError = '';
    propertyPreview = null;
    try {
      propertyPreview = await previewProperty(workspace.root, note.path, edit);
    } catch (cause) {
      propertyError = String(cause);
    } finally {
      propertyBusy = false;
    }
  }

  function previewPropertySet(note: NoteSummary, key: string, value: string) {
    try {
      void previewPropertyEdit(note, { action: 'set', key, value: JSON.parse(value) as PropertyValue });
    } catch (cause) {
      propertyError = `Property value must be valid JSON: ${String(cause)}`;
    }
  }

  async function applyPropertyEdit(note: NoteSummary) {
    if (!workspace || !propertyPreview || sessions.get(note.noteId)?.dirty) return;
    propertyBusy = true;
    propertyError = '';
    try {
      await applyProperty(workspace.root, propertyPreview);
      propertyPreview = null;
      propertyKey = '';
      sessions.delete(note.noteId);
      delete documents[note.noteId];
      delete contexts[note.noteId];
      delete properties[note.noteId];
      refreshSession();
      await load(note, navigation.activePaneId);
      if (graph) await refreshGraph();
    } catch (cause) {
      propertyError = String(cause);
    } finally {
      propertyBusy = false;
    }
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
    else if (id === 'daily-note') void openDailyNote();
    else if (id === 'graph') void openGraph();
    else if (id === 'primary-pane') void activatePane('primary', true);
    else if (id === 'secondary-pane') void activatePane('secondary', true);
    else if (id === 'health') { healthOpen = true; void inspectHealth(); }
    if (id !== 'palette') closePalette();
  }

  async function refreshGraph() {
    if (!workspace) return;
    graphBusy = true;
    graphError = '';
    try {
      graph = await readWorkspaceGraph(workspace.root);
    } catch (cause) {
      graphError = String(cause);
    } finally {
      graphBusy = false;
    }
  }

  async function openGraph() {
    view = 'graph';
    await refreshGraph();
  }

  async function openPalette() {
    returnFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    paletteOpen = true;
    paletteQuery = '';
    paletteIndex = 0;
    await tick();
    for (const child of document.querySelectorAll<HTMLElement>('main > :not(.palette-backdrop)')) child.inert = true;
    paletteInput?.focus();
  }

  function closePalette() {
    if (!paletteOpen) return;
    paletteOpen = false;
    for (const child of document.querySelectorAll<HTMLElement>('main > :not(.palette-backdrop)')) child.inert = false;
    void tick().then(() => returnFocus?.isConnected && returnFocus.focus());
  }

  async function inspectHealth() {
    if (!workspace) return;
    healthBusy = true;
    healthError = '';
    try { health = await inspectWorkspace(workspace.root); }
    catch (cause) { healthError = String(cause); }
    finally { healthBusy = false; }
  }

  async function runRecovery() {
    if (!workspace) return;
    healthBusy = true;
    healthError = '';
    try {
      recovery = await recoverWorkspace(workspace.root);
      health = recovery.diagnostics;
      const refreshed = await openWorkspace(workspace.root);
      results = [];
      contexts = {};
      properties = {};
      propertyValues = {};
      propertyPreview = null;
      for (const session of sessions.values()) {
        const note = refreshed.notes.find((candidate) => candidate.noteId === session.noteId);
        if (!note) {
          if (session.dirty) session.markRecoveryConflict('Recovery changed or removed this note; the unsaved draft was preserved.');
          else sessions.delete(session.noteId);
          delete documents[session.noteId];
          continue;
        }
        try {
          const loaded = await readNote(refreshed.root, note.path);
          session.reconcileAfterRecovery(loaded.source, loaded.version, loaded.path);
          documents[loaded.noteId] = loaded;
        } catch (cause) {
          if (session.dirty) session.markRecoveryConflict(`Recovery could not reload this note; the unsaved draft was preserved: ${String(cause)}`);
          else {
            sessions.delete(session.noteId);
            delete documents[session.noteId];
          }
        }
      }
      workspace = refreshed;
      refreshSession();
      for (const pane of navigation.panes) {
        const active = activeNote(pane);
        const note = active && refreshed.notes.find((candidate) => candidate.noteId === active.noteId);
        if (note && !sessions.has(note.noteId)) await load(note, pane.id);
      }
      if (graph) await refreshGraph();
    } catch (cause) { healthError = String(cause); }
    finally { healthBusy = false; }
  }

  function handleKeydown(event: KeyboardEvent) {
    if (paletteOpen) {
      if (event.key === 'Escape') { event.preventDefault(); closePalette(); }
      else if (event.key === 'ArrowDown') { event.preventDefault(); paletteIndex = Math.min(paletteIndex + 1, paletteMatches.length - 1); }
      else if (event.key === 'ArrowUp') { event.preventDefault(); paletteIndex = Math.max(paletteIndex - 1, 0); }
      else if (event.key === 'Enter' && paletteMatches[paletteIndex]) { event.preventDefault(); executeCommand(paletteMatches[paletteIndex].command.id); }
      else if (event.key === 'Tab' && paletteDialog) {
        const focusable = [...paletteDialog.querySelectorAll<HTMLElement>('input, button:not(:disabled), [tabindex]:not([tabindex="-1"])')];
        if (focusable.length) {
          const edge = event.shiftKey ? focusable[0] : focusable.at(-1);
          if (document.activeElement === edge) {
            event.preventDefault();
            (event.shiftKey ? focusable.at(-1) : focusable[0])?.focus();
          }
        }
      }
      return;
    }
    if (event.key === 'F6' && navigation.panes.length === 2) { event.preventDefault(); void cyclePane(); return; }
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

<main class:unopened={!workspace} aria-busy={busy || importBusy || chooseBusy || graphBusy || healthBusy}>
  {#if !workspace}
    <section class="welcome" aria-labelledby="welcome-title">
      <div class="welcome-story">
        <div class="welcome-brand"><span class="mark" aria-hidden="true">S</span><strong>SecondBrain</strong></div>
        <div>
          <p class="eyebrow">LOCAL KNOWLEDGE, DURABLE BY DEFAULT</p>
          <h1 id="welcome-title">Open your <em>second brain.</em></h1>
          <p class="lede">A private workspace for durable Markdown. Your files stay readable, the index stays disposable, and every edit remains recoverable.</p>
        </div>
        <p class="welcome-footnote">PLAIN TEXT · LOCAL FIRST · RECOVERABLE</p>
      </div>
      <div class="welcome-actions">
        <div class="action-heading">
          <p class="eyebrow">START HERE</p>
          <h2>Choose a workspace</h2>
          <p>Open an initialized SecondBrain folder, or type its full path.</p>
        </div>
        <form onsubmit={(event) => { event.preventDefault(); void open(); }}>
          <div class="folder-field">
            <button type="button" class="choose-folder" disabled={busy || importBusy || chooseBusy} onclick={() => void chooseFolder()}>
              {chooseBusy ? 'Choosing...' : 'Choose folder'}
            </button>
            <label for="workspace-root">Workspace folder</label>
            <input id="workspace-root" value={root} oninput={(event) => updateRoot(event.currentTarget.value)} placeholder="/Users/you/Notes" autocomplete="off" aria-describedby="folder-help" />
          </div>
          <p id="folder-help" class="folder-help">{root.trim() ? `Ready to open ${root.trim()}` : 'No folder selected. Choose a folder or enter a path to continue.'}</p>
          <button type="submit" class="open-workspace" disabled={busy || importBusy || chooseBusy || !root.trim()}>
            {busy ? 'Opening workspace...' : 'Open workspace'}
          </button>
        </form>
        <div class="import-action">
          <div class="import-copy">
            <h3>Bringing an Obsidian vault?</h3>
            <p>Review it before initialization. Import makes zero changes to Markdown, attachments, or <code>.obsidian</code> configuration.</p>
          </div>
          <button type="button" class="import-preview-button" disabled={busy || importBusy || chooseBusy || !root.trim()} onclick={() => void previewImport()}>
            {importBusy ? 'Checking vault...' : 'Review Obsidian import'}
          </button>
        </div>
        {#if importPreview}
          <section class="import-review" aria-labelledby="import-review-title">
            <h2 id="import-review-title">Import review</h2>
            <p>{importPreview.inventory.markdown.length} Markdown files · {importPreview.inventory.attachments.length} attachments · {importPreview.inventory.obsidianConfig.length} config files</p>
            <p>{importPreview.brokenLinks.length} broken links · {importPreview.ambiguousLinks.length} ambiguous links</p>
            {#if !importPreview.canApply}
              <p class="error" role="alert">Apply is blocked: {importPreview.parseErrors.length} parse errors, {importPreview.duplicateIds.length} duplicate IDs, {importPreview.portableCollisions.length} path collisions, {importPreview.inventory.symlinks.length} symlinks, {importPreview.inventory.unsupported.length} unsupported files.</p>
            {/if}
            <button type="button" disabled={importBusy || !importPreview.canApply} onclick={() => void applyImport()}>
              {importPreview.alreadyInitialized ? 'Refresh existing workspace' : 'Initialize and open vault'}
            </button>
          </section>
        {/if}
        {#if error}<p class="error welcome-error" role="alert">{error}</p>{/if}
        {#if busy || importBusy || chooseBusy}<p class="welcome-status" role="status">{chooseBusy ? 'Opening the folder picker...' : importBusy ? 'Inspecting the selected vault...' : 'Opening the selected workspace...'}</p>{/if}
      </div>
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
      <button class:attention={health?.status === 'attention'} class="health" onclick={() => { healthOpen = !healthOpen; if (healthOpen) void inspectHealth(); }} aria-expanded={healthOpen} aria-controls="health-panel"><i></i>{health?.status ?? 'checking'}</button>
      <button class="daily-note-control" onclick={() => void openDailyNote()}>Daily note</button>
      <button class="graph-control" aria-pressed={view === 'graph'} onclick={() => void openGraph()}>Graph</button>
    </header>

    <aside aria-label="Workspace navigation">
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

    {#if view === 'graph'}
      <section class="workspace-panes graph-workspace" aria-label="Workspace graph">
        {#if graphBusy}<p class="graph-state">Loading workspace graph...</p>
        {:else if graphError}<p class="graph-state error" role="alert">{graphError}</p>
        {:else if graph}<GraphView {graph} onOpen={choose} />
        {:else}<p class="graph-state">Open the graph again to load indexed data.</p>{/if}
      </section>
    {:else}
    <section class:split={navigation.panes.length === 2} class="workspace-panes" aria-label="Document workspace">
      {#each navigation.panes as pane (pane.id)}
        {@const paneNote = activeNote(pane)}
        {@const document = paneNote ? documents[paneNote.noteId] : null}
        {@const view = sessionView(paneNote ? sessions.get(paneNote.noteId) : undefined)}
        <article
          class:active-pane={navigation.activePaneId === pane.id}
          class="pane"
          data-pane={pane.id}
          role="region"
          aria-label={pane.id === 'primary' ? 'Primary pane' : 'Secondary pane'}
        >
          <div class="pane-bar">
            <div class="history-controls" aria-label={`${pane.id} navigation history`}>
              <button aria-label={`Back in ${pane.id} pane`} disabled={pane.historyIndex <= 0} onclick={() => moveHistory(pane.id, 'back')}>←</button>
              <button aria-label={`Forward in ${pane.id} pane`} disabled={pane.historyIndex >= pane.history.length - 1} onclick={() => moveHistory(pane.id, 'forward')}>→</button>
            </div>
            <div class="tabs-shell">
              <div class="tabs" role="tablist" aria-label={`${pane.id} pane tabs`}>
                {#each pane.tabs as tab (tab.noteId)}
                  <button class:active={pane.activeNoteId === tab.noteId} id={`tab-${pane.id}-${tab.noteId}`} role="tab" aria-controls={`tabpanel-${pane.id}`} aria-selected={pane.activeNoteId === tab.noteId} tabindex={pane.activeNoteId === tab.noteId ? 0 : -1} onkeydown={(event) => handleTabKey(event, pane.id, tab.noteId)} onclick={() => selectTab(tab, pane.id)}>{noteLabel(tab)}</button>
                {/each}
              </div>
              {#if paneNote}<button class="close-tab" aria-label={`Close ${noteLabel(paneNote)}`} onclick={() => closeTab(paneNote.noteId, pane.id)}>×</button>{/if}
            </div>
            <button class="split-control" onclick={navigation.panes.length === 1 ? split : closeSplit}>
              {navigation.panes.length === 1 ? 'Split' : 'Close split'}
            </button>
          </div>
          <div class="canvas" data-pane-content={pane.id} tabindex="0" onfocusin={() => void activatePane(pane.id)} id={`tabpanel-${pane.id}`} role="tabpanel" aria-labelledby={paneNote ? `tab-${pane.id}-${paneNote.noteId}` : undefined}>
            {#if paneNote}
              <p class="path">{paneNote.path}</p>
              <h1>{noteLabel(paneNote)}</h1>
              <div class="rule"></div>
              {#if document && view}
                <div class="editor-controls" role="toolbar" aria-label={`${noteLabel(paneNote)} editor mode`}>
                  {#each ['wysiwyg', 'source', 'split'] as mode}
                    <button class:active={view.mode === mode} aria-pressed={view.mode === mode} onclick={() => setMode(view.session, mode as EditorMode)}>{mode === 'wysiwyg' ? 'WYSIWYG' : mode[0].toUpperCase() + mode.slice(1)}</button>
                  {/each}
                </div>
                <div class:split-editors={view.mode === 'split'} class="editors">
                  {#if view.mode === 'wysiwyg' || view.mode === 'split'}
                    <WysiwygEditor session={view.session} draftRevision={view.draftRevision} label={`${noteLabel(paneNote)} WYSIWYG editor`} onSourceChange={refreshSession} />
                  {/if}
                  {#if view.mode === 'source' || view.mode === 'split'}
                    <SourceEditor source={view.source} status={view.status} selection={view.selection} ariaLabel={`${noteLabel(paneNote)} Markdown source`} onChange={(update) => editSource(view.session, update.source, update.selection)} />
                  {/if}
                </div>
                {#if view.dirty}
                  <div class="transaction-controls" aria-label="Transaction controls">
                    <button disabled={view.status === 'previewing' || view.status === 'applying'} onclick={() => previewSession(view.session)}>{view.status === 'previewing' ? 'Previewing...' : 'Preview'}</button>
                    <button disabled={!view.preview || view.preview.review_required || view.status === 'applying'} onclick={() => applySession(view.session, pane.id)}>{view.status === 'applying' ? 'Applying...' : 'Apply'}</button>
                    <span role="status">{view.status}{view.preview ? ` · ${view.preview.summary}` : ''}</span>
                  </div>
                  {#if view.status === 'review'}<p class="review" role="alert">This transaction requires review and cannot be applied.</p>{/if}
                  {#if view.status === 'error'}<p class="error" role="alert">{String(view.error)}</p>{/if}
                  {#if view.status === 'recovery-conflict'}<p class="error" role="alert">Recovery reloaded workspace state, but this unsaved draft was preserved. Review it against the current note before previewing again.{view.error ? ` ${String(view.error)}` : ''}</p>{/if}
                {/if}
                {#if view.status === 'reloaded'}<p role="status">Note reloaded after recovery.</p>{/if}
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
    {/if}

    <aside class="context-panel" aria-label="Note context" hidden={view === 'graph'}>
      {#if selected && contexts[selected.noteId]}
        {@const context = contexts[selected.noteId]}
        {@const selectedSession = sessionView(sessions.get(selected.noteId))}
        <section class="properties" aria-labelledby="properties-title">
          <h2 id="properties-title">Properties</h2>
          {#if selectedSession?.dirty}<p class="review" role="status">Save or discard the editor draft before editing properties.</p>{/if}
          <fieldset disabled={propertyBusy || selectedSession?.dirty}>
            {#each Object.entries(properties[selected.noteId] ?? {}) as [key, value] (key)}
              <div class="property-row">
                <label for={`property-${selected.noteId}-${key}`}>{key}</label>
                <input id={`property-${selected.noteId}-${key}`} bind:value={propertyValues[`${selected.noteId}:${key}`]} aria-label={`${key} JSON value`} />
                <button onclick={() => previewPropertySet(selected, key, propertyValues[`${selected.noteId}:${key}`])}>Preview</button>
                <button aria-label={`Remove ${key}`} onclick={() => previewPropertyEdit(selected, { action: 'remove', key })}>Remove</button>
              </div>
            {/each}
            <div class="property-row property-new">
              <input bind:value={propertyKey} aria-label="New property name" placeholder="property" />
              <input value="null" aria-label="New property JSON value" oninput={(event) => { propertyValues[`${selected.noteId}:new`] = event.currentTarget.value; }} />
              <button disabled={!propertyKey.trim()} onclick={() => previewPropertySet(selected, propertyKey.trim(), propertyValues[`${selected.noteId}:new`] ?? 'null')}>Preview add</button>
            </div>
          </fieldset>
          {#if propertyPreview}
            <div class="property-review">
              <p>{propertyPreview.transaction.summary}</p>
              <button disabled={propertyPreview.transaction.review_required || propertyBusy || selectedSession?.dirty} onclick={() => applyPropertyEdit(selected)}>Apply property</button>
            </div>
          {/if}
          {#if propertyError}<p class="error" role="alert">{propertyError}</p>{/if}
        </section>
        <section aria-labelledby="outline-title"><h2 id="outline-title">Outline</h2>
          {#if context.outline.length}<ol class="outline-list">{#each context.outline as heading}<li style={`--level: ${heading.level}`}><button title={`Line ${heading.line}`} onclick={() => selectedSession && void openOutlineHeading(selectedSession.session, heading.line)}>{heading.text}</button><small>line {heading.line}</small></li>{/each}</ol>
          {:else}<p class="context-empty">No headings</p>{/if}
        </section>
        <section aria-labelledby="backlinks-title"><h2 id="backlinks-title">Backlinks</h2>
          {#if context.backlinks.length}<ul class="backlink-list">{#each context.backlinks as backlink (backlink.noteId)}<li><button onclick={() => choose(backlink)}><span>{noteLabel(backlink)}</span><small>{backlink.path}</small></button></li>{/each}</ul>
          {:else}<p class="context-empty">No backlinks</p>{/if}
        </section>
      {:else if selected && paneBusy[navigation.activePaneId]}<p class="context-empty">Loading context...</p>
      {:else}<p class="context-empty">Select a note</p>{/if}
    </aside>

    {#if healthOpen}
      <section id="health-panel" class="health-panel" aria-labelledby="health-title">
        <div class="health-heading"><div><p>WORKSPACE INTEGRITY</p><h2 id="health-title">Health & recovery</h2></div><button aria-label="Close health and recovery" onclick={() => { healthOpen = false; }}>×</button></div>
        {#if healthBusy}<p role="status">Inspecting workspace state...</p>
        {:else if healthError}<p class="error" role="alert">{healthError}</p>
        {:else if health}
          <p class="health-summary" role="status">{health.status === 'healthy' ? 'Workspace is healthy.' : `${health.issues.length} issue(s) need attention.`}</p>
          <dl><div><dt>Derived index</dt><dd>{health.index.status} · {health.index.notes} notes</dd></div><div><dt>Transactions</dt><dd>{health.transactions.pending} pending · {health.transactions.reviewsPending} review</dd></div></dl>
          {#each health.issues as issue}<article class="health-issue"><strong>{issue.code}</strong><p>{issue.message}</p><small>{issue.action} · {issue.path}{issue.reviewRequired ? ' · review required' : issue.retryable ? ' · retry available' : ''}</small></article>{/each}
          <div class="health-actions"><button onclick={() => void inspectHealth()}>Rerun diagnostics</button><button disabled={healthBusy} onclick={() => void runRecovery()}>Run safe recovery</button></div>
          {#if recovery}<section class="recovery-results" aria-labelledby="recovery-results-title"><h3 id="recovery-results-title">Last recovery</h3><p>{recovery.repaired} repaired · {recovery.quarantined} quarantined · {recovery.abandoned} abandoned · {recovery.reviewRequired} review</p>{#each recovery.actions as action}<article><strong>{action.code}: {action.action}</strong><p>{action.path}: {action.message}</p>{#if action.quarantinePath}<small>Preserved at {action.quarantinePath}</small>{/if}</article>{:else}<p>Nothing required recovery.</p>{/each}</section>{/if}
        {/if}
      </section>
    {/if}
    <footer><span role="status" aria-live="polite">{workStatus}</span><span>SecondBrain · Phase 1</span></footer>
  {/if}
  {#if paletteOpen}
    <div class="palette-backdrop" role="presentation" onclick={(event) => { if (event.target === event.currentTarget) closePalette(); }}>
      <div bind:this={paletteDialog} class="command-palette" role="dialog" aria-modal="true" aria-labelledby="palette-title">
        <h2 id="palette-title">Command palette</h2>
        <input bind:this={paletteInput} role="combobox" aria-label="Find a command" aria-autocomplete="list" aria-controls="palette-options" aria-expanded="true" aria-activedescendant={paletteMatches[paletteIndex] ? `palette-option-${paletteMatches[paletteIndex].command.id}` : undefined} bind:value={paletteQuery} oninput={() => { paletteIndex = 0; }} autocomplete="off" />
        <ul id="palette-options" role="listbox" aria-label="Commands">{#each paletteMatches as match, index (match.command.id)}<li id={`palette-option-${match.command.id}`} class:active={index === paletteIndex} role="option" aria-selected={index === paletteIndex} tabindex="-1" onkeydown={(event) => { if (event.key === 'Enter' || event.key === ' ') executeCommand(match.command.id); }} onclick={() => executeCommand(match.command.id)}>{match.command.label}<kbd>{match.command.normalizedShortcut?.signature ?? ''}</kbd></li>{:else}<li class="context-empty">No matching commands</li>{/each}</ul>
      </div>
    </div>
  {/if}
</main>
