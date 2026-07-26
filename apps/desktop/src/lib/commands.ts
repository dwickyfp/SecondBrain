export type CommandPlatform = 'macos' | 'windows' | 'linux';

export type ShortcutModifier = 'Alt' | 'Control' | 'Meta' | 'Mod' | 'Shift';

export interface Shortcut {
  key: string;
  modifiers?: readonly ShortcutModifier[];
}

export type ShortcutInput = string | Shortcut;

export interface NormalizedShortcut {
  key: string;
  altKey: boolean;
  ctrlKey: boolean;
  metaKey: boolean;
  shiftKey: boolean;
  signature: string;
}

export interface CommandDefinition<Id extends string = string, Context = undefined> {
  id: Id;
  label: string;
  shortcut?: ShortcutInput;
  enabled?: boolean | ((context: Context) => boolean);
}

export interface RegisteredCommand<Id extends string = string, Context = undefined>
  extends CommandDefinition<Id, Context> {
  normalizedShortcut?: NormalizedShortcut;
}

export interface PaletteMatch<Id extends string = string, Context = undefined> {
  command: RegisteredCommand<Id, Context>;
  score: number;
}

export interface ShortcutEvent {
  key: string;
  altKey: boolean;
  ctrlKey: boolean;
  metaKey: boolean;
  shiftKey: boolean;
}

export interface ShortcutSuppressionEvent extends Partial<ShortcutEvent> {
  target: EventTarget | null;
  isComposing?: boolean;
  keyCode?: number;
}

const modifierAliases: Readonly<Record<string, ShortcutModifier>> = {
  alt: 'Alt',
  option: 'Alt',
  control: 'Control',
  ctrl: 'Control',
  command: 'Meta',
  cmd: 'Meta',
  meta: 'Meta',
  mod: 'Mod',
  shift: 'Shift'
};

const keyAliases: Readonly<Record<string, string>> = {
  esc: 'escape',
  return: 'enter',
  space: ' ',
  spacebar: ' '
};

function normalizeKey(key: string): string {
  const trimmed = key.trim();
  if (!trimmed) throw new Error('Shortcut key must not be empty');
  return keyAliases[trimmed.toLowerCase()] ?? trimmed.toLowerCase();
}

function parseShortcut(shortcut: ShortcutInput): Shortcut {
  if (typeof shortcut !== 'string') return shortcut;

  const parts = shortcut.split('+').map((part) => part.trim()).filter(Boolean);
  if (parts.length === 0) throw new Error('Shortcut must not be empty');
  const key = parts.at(-1)!;
  const modifiers = parts.slice(0, -1).map((part) => {
    const modifier = modifierAliases[part.toLowerCase()];
    if (!modifier) throw new Error(`Unknown shortcut modifier: ${part}`);
    return modifier;
  });
  return { key, modifiers };
}

export function normalizeShortcut(
  shortcut: ShortcutInput,
  platform: CommandPlatform
): NormalizedShortcut {
  const parsed = parseShortcut(shortcut);
  const modifiers = new Set(parsed.modifiers ?? []);
  const primaryModifier = platform === 'macos' ? 'Meta' : 'Control';
  if (modifiers.delete('Mod')) modifiers.add(primaryModifier);

  const normalized = {
    key: normalizeKey(parsed.key),
    altKey: modifiers.has('Alt'),
    ctrlKey: modifiers.has('Control'),
    metaKey: modifiers.has('Meta'),
    shiftKey: modifiers.has('Shift')
  };
  const activeModifiers = [
    normalized.ctrlKey ? 'Control' : '',
    normalized.altKey ? 'Alt' : '',
    normalized.shiftKey ? 'Shift' : '',
    normalized.metaKey ? 'Meta' : ''
  ].filter(Boolean);

  return {
    ...normalized,
    signature: [...activeModifiers, normalized.key].join('+')
  };
}

export function shortcutMatches(event: ShortcutEvent, shortcut: NormalizedShortcut): boolean {
  return normalizeKey(event.key) === shortcut.key
    && event.altKey === shortcut.altKey
    && event.ctrlKey === shortcut.ctrlKey
    && event.metaKey === shortcut.metaKey
    && event.shiftKey === shortcut.shiftKey;
}

export function isTypingTarget(target: EventTarget | null): boolean {
  if (typeof Element === 'undefined' || !(target instanceof Element)) return false;
  if (target.closest('input, textarea, select')) return true;
  let current: Element | null = target;
  while (current) {
    if (current.hasAttribute('contenteditable')) {
      return current.getAttribute('contenteditable') !== 'false';
    }
    current = current.parentElement;
  }
  return false;
}

export function isImeComposing(event: Pick<ShortcutSuppressionEvent, 'isComposing' | 'key' | 'keyCode'>): boolean {
  return event.isComposing === true || event.key === 'Process' || event.keyCode === 229;
}

export function shouldSuppressShortcut(event: ShortcutSuppressionEvent): boolean {
  if (isImeComposing(event)) return true;
  const hasCommandModifier = event.altKey === true || event.ctrlKey === true || event.metaKey === true;
  return !hasCommandModifier && isTypingTarget(event.target);
}

function normalizedSearchText(value: string): string {
  return value
    .normalize('NFKD')
    .replace(/[\u0300-\u036f]/g, '')
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, ' ')
    .trim();
}

function subsequenceScore(candidate: string, query: string): number | null {
  let candidateIndex = 0;
  let gapCount = 0;
  for (const character of query.replaceAll(' ', '')) {
    const foundAt = candidate.indexOf(character, candidateIndex);
    if (foundAt < 0) return null;
    gapCount += foundAt - candidateIndex;
    candidateIndex = foundAt + 1;
  }
  return 50 + gapCount;
}

function paletteScore(command: Pick<CommandDefinition, 'id' | 'label'>, query: string): number | null {
  if (!query) return 0;
  const label = normalizedSearchText(command.label);
  const id = normalizedSearchText(command.id);
  if (label === query) return 0;
  if (id === query) return 1;
  if (label.startsWith(query)) return 10;
  if (id.startsWith(query)) return 11;
  if (label.split(' ').some((word) => word.startsWith(query))) return 20;
  if (label.includes(query)) return 30;

  const terms = query.split(' ');
  if (terms.every((term) => label.includes(term) || id.includes(term))) return 40;
  return subsequenceScore(`${label} ${id}`, query);
}

function compareText(left: string, right: string): number {
  return left < right ? -1 : left > right ? 1 : 0;
}

export function matchPalette<Id extends string, Context>(
  commands: readonly RegisteredCommand<Id, Context>[],
  query: string
): PaletteMatch<Id, Context>[] {
  const normalizedQuery = normalizedSearchText(query);
  return commands
    .map((command) => ({ command, score: paletteScore(command, normalizedQuery) }))
    .filter((match): match is PaletteMatch<Id, Context> => match.score !== null)
    .sort((left, right) => left.score - right.score
      || compareText(normalizedSearchText(left.command.label), normalizedSearchText(right.command.label))
      || compareText(left.command.id, right.command.id));
}

export class CommandRegistry<Id extends string, Context = undefined> {
  readonly commands: readonly RegisteredCommand<Id, Context>[];
  readonly #byId: ReadonlyMap<Id, RegisteredCommand<Id, Context>>;

  constructor(definitions: readonly CommandDefinition<Id, Context>[], platform: CommandPlatform) {
    const byId = new Map<Id, RegisteredCommand<Id, Context>>();
    const byShortcut = new Map<string, Id>();
    const commands = definitions.map((definition) => {
      if (!definition.id.trim()) throw new Error('Command ID must not be empty');
      if (!definition.label.trim()) throw new Error(`Command label must not be empty: ${definition.id}`);
      if (byId.has(definition.id)) throw new Error(`Duplicate command ID: ${definition.id}`);

      const normalizedShortcut = definition.shortcut
        ? normalizeShortcut(definition.shortcut, platform)
        : undefined;
      if (normalizedShortcut) {
        const owner = byShortcut.get(normalizedShortcut.signature);
        if (owner) {
          throw new Error(
            `Shortcut collision: ${definition.id} and ${owner} both use ${normalizedShortcut.signature}`
          );
        }
        byShortcut.set(normalizedShortcut.signature, definition.id);
      }

      const command: RegisteredCommand<Id, Context> = Object.freeze({
        ...definition,
        normalizedShortcut
      });
      byId.set(command.id, command);
      return command;
    });

    this.commands = Object.freeze(commands);
    this.#byId = byId;
  }

  get(id: Id): RegisteredCommand<Id, Context> | undefined {
    return this.#byId.get(id);
  }

  isEnabled(id: Id, context: Context): boolean {
    const command = this.#byId.get(id);
    if (!command) return false;
    return typeof command.enabled === 'function'
      ? command.enabled(context)
      : command.enabled !== false;
  }

  match(query: string): PaletteMatch<Id, Context>[] {
    return matchPalette(this.commands, query);
  }

  commandForShortcut(event: ShortcutEvent): RegisteredCommand<Id, Context> | undefined {
    return this.commands.find((command) => command.normalizedShortcut
      && shortcutMatches(event, command.normalizedShortcut));
  }
}

export function createCommandRegistry<const Id extends string, Context = undefined>(
  definitions: readonly CommandDefinition<Id, Context>[],
  platform: CommandPlatform
): CommandRegistry<Id, Context> {
  return new CommandRegistry(definitions, platform);
}
