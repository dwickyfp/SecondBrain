import { describe, expect, it } from 'vitest';
import {
  createCommandRegistry,
  isImeComposing,
  isTypingTarget,
  matchPalette,
  normalizeShortcut,
  shouldSuppressShortcut,
  shortcutMatches,
  type CommandDefinition
} from './commands';

type Context = { hasNote: boolean; canGoBack: boolean };

const commands = [
  { id: 'note.open', label: 'Open Note', shortcut: 'Mod+O' },
  { id: 'note.close', label: 'Close Note', shortcut: 'Mod+W', enabled: (context: Context) => context.hasNote },
  { id: 'navigation.back', label: 'Navigate Back', enabled: (context: Context) => context.canGoBack },
  { id: 'palette.open', label: 'Show Command Palette', shortcut: 'Mod+Shift+P' },
  { id: 'workspace.settings', label: 'Workspace Settings', enabled: false }
] as const satisfies readonly CommandDefinition<string, Context>[];

describe('command registry', () => {
  it('preserves literal command IDs and evaluates enablement against current context', () => {
    const registry = createCommandRegistry(commands, 'macos');

    expect(registry.get('note.open')?.label).toBe('Open Note');
    expect(registry.isEnabled('note.open', { hasNote: false, canGoBack: false })).toBe(true);
    expect(registry.isEnabled('note.close', { hasNote: false, canGoBack: true })).toBe(false);
    expect(registry.isEnabled('note.close', { hasNote: true, canGoBack: false })).toBe(true);
    expect(registry.isEnabled('workspace.settings', { hasNote: true, canGoBack: true })).toBe(false);
  });

  it('rejects duplicate IDs before exposing a partial registry', () => {
    expect(() => createCommandRegistry([
      { id: 'note.open', label: 'Open Note' },
      { id: 'note.open', label: 'Open Another Note' }
    ], 'linux')).toThrow('Duplicate command ID: note.open');
  });

  it('rejects empty IDs and labels', () => {
    expect(() => createCommandRegistry([{ id: '', label: 'Empty' }], 'linux'))
      .toThrow('Command ID must not be empty');
    expect(() => createCommandRegistry([{ id: 'empty', label: '  ' }], 'linux'))
      .toThrow('Command label must not be empty: empty');
  });

  it('rejects shortcuts that collide after platform normalization', () => {
    const colliding = [
      { id: 'palette.open', label: 'Palette', shortcut: 'Mod+Shift+P' },
      { id: 'search.open', label: 'Search', shortcut: 'Cmd+Shift+p' }
    ] as const;

    expect(() => createCommandRegistry(colliding, 'macos'))
      .toThrow('Shortcut collision: search.open and palette.open both use Shift+Meta+p');
    expect(() => createCommandRegistry(colliding, 'windows')).not.toThrow();
  });
});

describe('platform shortcuts', () => {
  it('maps Mod to Meta on macOS and Control elsewhere', () => {
    expect(normalizeShortcut('Mod+Shift+P', 'macos')).toEqual({
      key: 'p', altKey: false, ctrlKey: false, metaKey: true, shiftKey: true,
      signature: 'Shift+Meta+p'
    });
    expect(normalizeShortcut('Mod+Shift+P', 'windows')).toEqual({
      key: 'p', altKey: false, ctrlKey: true, metaKey: false, shiftKey: true,
      signature: 'Control+Shift+p'
    });
    expect(normalizeShortcut('Mod+Shift+P', 'linux').signature).toBe('Control+Shift+p');
  });

  it('accepts structured shortcuts and common key and modifier aliases', () => {
    expect(normalizeShortcut({ key: 'Escape', modifiers: ['Alt'] }, 'linux').signature).toBe('Alt+escape');
    expect(normalizeShortcut('Option+Return', 'macos').signature).toBe('Alt+enter');
    expect(normalizeShortcut('Ctrl+Space', 'windows').signature).toBe('Control+ ');
    expect(() => normalizeShortcut('Hyper+P', 'linux')).toThrow('Unknown shortcut modifier: Hyper');
  });

  it('requires an exact key and modifier match', () => {
    const shortcut = normalizeShortcut('Mod+Shift+P', 'macos');
    expect(shortcutMatches({
      key: 'P', altKey: false, ctrlKey: false, metaKey: true, shiftKey: true
    }, shortcut)).toBe(true);
    expect(shortcutMatches({
      key: 'p', altKey: true, ctrlKey: false, metaKey: true, shiftKey: true
    }, shortcut)).toBe(false);

    const registry = createCommandRegistry(commands, 'macos');
    expect(registry.commandForShortcut({
      key: 'o', altKey: false, ctrlKey: false, metaKey: true, shiftKey: false
    })?.id).toBe('note.open');
  });
});

describe('command palette matching', () => {
  const registry = createCommandRegistry(commands, 'linux');

  it('ranks exact, prefix, word, term, and fuzzy matches deterministically', () => {
    expect(registry.match('open note').map(({ command }) => command.id)).toEqual(['note.open']);
    expect(registry.match('open').map(({ command }) => command.id)).toEqual([
      'note.open',
      'palette.open',
      'workspace.settings'
    ]);
    expect(registry.match('command').map(({ command }) => command.id)).toEqual(['palette.open']);
    expect(registry.match('workspace settings').map(({ command }) => command.id))
      .toEqual(['workspace.settings']);
    expect(registry.match('nvg bk').map(({ command }) => command.id)).toEqual(['navigation.back']);
  });

  it('normalizes case, punctuation, whitespace, and diacritics', () => {
    const accented = createCommandRegistry([
      { id: 'resume.open', label: 'Open Résumé' },
      { id: 'note.open', label: 'Open Note' }
    ], 'linux');

    expect(accented.match('  RÉSUMÉ  ').map(({ command }) => command.id)).toEqual(['resume.open']);
    expect(registry.match('NAVIGATION.BACK')[0]?.command.id).toBe('navigation.back');
  });

  it('uses normalized label then ID as stable tie breakers and includes disabled commands', () => {
    const tied = createCommandRegistry([
      { id: 'z', label: 'Beta', enabled: false },
      { id: 'b', label: 'Alpha' },
      { id: 'a', label: 'Alpha' }
    ], 'linux');

    expect(tied.match('').map(({ command }) => command.id)).toEqual(['a', 'b', 'z']);
    expect(matchPalette(tied.commands, '').map(({ command }) => command.id)).toEqual(['a', 'b', 'z']);
  });
});

describe('typing and IME suppression', () => {
  it('recognizes direct and nested editable targets but not contenteditable=false', () => {
    const input = document.createElement('input');
    const editor = document.createElement('div');
    editor.setAttribute('contenteditable', 'true');
    const child = document.createElement('span');
    editor.append(child);
    const nonEditable = document.createElement('div');
    nonEditable.setAttribute('contenteditable', 'false');

    expect(isTypingTarget(input)).toBe(true);
    expect(isTypingTarget(child)).toBe(true);
    expect(isTypingTarget(nonEditable)).toBe(false);
    expect(isTypingTarget(document.body)).toBe(false);
    expect(isTypingTarget(null)).toBe(false);
  });

  it('detects modern and compatibility IME composition signals', () => {
    expect(isImeComposing({ isComposing: true })).toBe(true);
    expect(isImeComposing({ key: 'Process' })).toBe(true);
    expect(isImeComposing({ keyCode: 229 })).toBe(true);
    expect(isImeComposing({ key: 'p', keyCode: 80 })).toBe(false);
  });

  it('suppresses all shortcuts during composition and bare shortcuts while typing', () => {
    const textarea = document.createElement('textarea');
    expect(shouldSuppressShortcut({ target: document.body, isComposing: true, metaKey: true })).toBe(true);
    expect(shouldSuppressShortcut({ target: textarea, key: 'p' })).toBe(true);
    expect(shouldSuppressShortcut({ target: textarea, key: 'p', metaKey: true })).toBe(false);
    expect(shouldSuppressShortcut({ target: document.body, key: 'p' })).toBe(false);
  });
});
