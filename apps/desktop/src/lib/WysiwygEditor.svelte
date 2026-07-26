<script module lang="ts">
  import type { JSONContent } from '@tiptap/core';

  const escapeHtml = (value: string) => value
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;');

  function inlineHtml(source: string): string {
    const token = /(`[^`\n]+`|\[[^\]\n]+\]\((?:https?:\/\/|mailto:)[^\s)]+\)|\*\*[^*\n]+\*\*|__[^_\n]+__|~~[^~\n]+~~|\*[^*\n]+\*|_[^_\n]+_)/g;
    let html = '';
    let offset = 0;
    for (const match of source.matchAll(token)) {
      html += escapeHtml(source.slice(offset, match.index));
      const value = match[0];
      if (value.startsWith('`')) html += `<code>${escapeHtml(value.slice(1, -1))}</code>`;
      else if (value.startsWith('[')) {
        const split = value.indexOf('](');
        html += `<a href="${escapeHtml(value.slice(split + 2, -1))}">${escapeHtml(value.slice(1, split))}</a>`;
      } else if (value.startsWith('**') || value.startsWith('__')) {
        html += `<strong>${escapeHtml(value.slice(2, -2))}</strong>`;
      } else if (value.startsWith('~~')) html += `<s>${escapeHtml(value.slice(2, -2))}</s>`;
      else html += `<em>${escapeHtml(value.slice(1, -1))}</em>`;
      offset = match.index! + value.length;
    }
    return html + escapeHtml(source.slice(offset));
  }

  /**
   * The lossless boundary used by WysiwygEditor. False means the source must be
   * displayed verbatim rather than passed through TipTap's HTML document model.
   */
  export function isWysiwygSupportedSource(source: string): boolean {
    if (/\r|\0|^---(?:\n|$)|<[^\n>]+>|!\[|^\s*\[[^\]]+\]:|\[\^|^\$\$|^%%|\[\[|\\|&(?:#\d+|\w+);| {2}$/m.test(source)) return false;
    if (/^(?: {1,}|\t)|^\s*[-*+] \[[ xX]\]|^(?:___+|---+|\*\*\*+)\s*$|^.+\n(?:===+|---+)\s*$/m.test(source)) return false;

    let fenced = false;
    for (const line of source.split('\n')) {
      if (line === '```') {
        fenced = !fenced;
        continue;
      }
      if (fenced) continue;
      if (/^```/.test(line)) return false;
      if (/^#{1,6}(?:$|[^ ])|^#{7,}|^>[^ ]|^\d+[.)](?:$|[^ ])|^[-*+](?:$|[^ ])/.test(line)) return false;
      if (/^\s+(?:[-*+] |\d+\. )/.test(line)) return false;
    }
    if (fenced) return false;

    const withoutTokens = source.replace(/`[^`\n]+`|\[[^\]\n]+\]\((?:https?:\/\/|mailto:)[^\s)]+\)|\*\*[^*\n]+\*\*|__[^_\n]+__|~~[^~\n]+~~|\*[^*\n]+\*|_[^_\n]+_/g, '');
    return !/[`*_~]|\[[^\n]*\]\(|\]\(/.test(withoutTokens);
  }

  export function supportedMarkdownToHtml(source: string): string {
    const lines = source.split('\n');
    const html: string[] = [];
    for (let index = 0; index < lines.length;) {
      const line = lines[index];
      if (!line) { index += 1; continue; }
      if (line === '```') {
        const body: string[] = [];
        index += 1;
        while (lines[index] !== '```') body.push(lines[index++] ?? '');
        html.push(`<pre><code>${escapeHtml(body.join('\n'))}</code></pre>`);
        index += 1;
        continue;
      }
      const heading = /^(#{1,6}) (.*)$/.exec(line);
      if (heading) {
        html.push(`<h${heading[1].length}>${inlineHtml(heading[2])}</h${heading[1].length}>`);
        index += 1;
        continue;
      }
      const quote: string[] = [];
      while (lines[index]?.startsWith('> ')) quote.push(`<p>${inlineHtml(lines[index++].slice(2))}</p>`);
      if (quote.length) { html.push(`<blockquote>${quote.join('')}</blockquote>`); continue; }
      const list = /^(?:([-*+]) |(\d+)\. )(.*)$/.exec(line);
      if (list) {
        const ordered = Boolean(list[2]);
        const items: string[] = [];
        const pattern = ordered ? /^\d+\. (.*)$/ : /^[-*+] (.*)$/;
        while (index < lines.length) {
          const item = pattern.exec(lines[index]);
          if (!item) break;
          items.push(`<li><p>${inlineHtml(item[1])}</p></li>`);
          index += 1;
        }
        const tag = ordered ? 'ol' : 'ul';
        html.push(`<${tag}>${items.join('')}</${tag}>`);
        continue;
      }
      const paragraph: string[] = [line];
      index += 1;
      while (lines[index] && !/^(?:#{1,6} |> |```|[-*+] |\d+\. )/.test(lines[index])) paragraph.push(lines[index++]);
      html.push(`<p>${inlineHtml(paragraph.join(' '))}</p>`);
    }
    return html.join('');
  }

  function textMarkdown(node: JSONContent): string {
    if (node.type === 'hardBreak') return '\n';
    let value = node.text ?? (node.content ?? []).map(textMarkdown).join('');
    for (const mark of node.marks ?? []) {
      if (mark.type === 'code') value = `\`${value}\``;
      else if (mark.type === 'bold') value = `**${value}**`;
      else if (mark.type === 'italic') value = `*${value}*`;
      else if (mark.type === 'strike') value = `~~${value}~~`;
      else if (mark.type === 'link') value = `[${value}](${String(mark.attrs?.href ?? '')})`;
    }
    return value;
  }

  export function tiptapJsonToMarkdown(document: JSONContent): string {
    const block = (node: JSONContent): string => {
      const content = (node.content ?? []).map(textMarkdown).join('');
      if (node.type === 'heading') return `${'#'.repeat(Number(node.attrs?.level ?? 1))} ${content}`;
      if (node.type === 'paragraph') return content;
      if (node.type === 'codeBlock') return `\`\`\`\n${content}\n\`\`\``;
      if (node.type === 'blockquote') return (node.content ?? []).map(block).join('\n').split('\n').map((line) => `> ${line}`).join('\n');
      if (node.type === 'bulletList') return (node.content ?? []).map((item) => `- ${block(item)}`).join('\n');
      if (node.type === 'orderedList') return (node.content ?? []).map((item, index) => `${index + 1}. ${block(item)}`).join('\n');
      if (node.type === 'listItem') return (node.content ?? []).map(block).join(' ');
      return content;
    };
    const markdown = (document.content ?? []).map(block).join('\n\n');
    return markdown ? `${markdown}\n` : '';
  }
</script>

<script lang="ts">
  import { Editor } from '@tiptap/core';
  import StarterKit from '@tiptap/starter-kit';
  import { onMount } from 'svelte';
  import type { NoteSession, ProjectionUpdate } from './note-session';

  /**
   * Projection API: mutate only `session.draftSource` through updateFromWysiwyg.
   * Incrementing `draftRevision` asks this component to project an external
   * session update. Unsupported source is shown verbatim and is never editable.
   */
  interface Props {
    session: NoteSession;
    draftRevision?: number;
    label?: string;
    onSourceChange?: (update: ProjectionUpdate) => void;
  }

  let { session, draftRevision = 0, label = 'WYSIWYG editor', onSourceChange }: Props = $props();
  let host: HTMLDivElement;
  let editor: Editor | null = null;
  let mounted = $state(false);
  let rawSource = $state<string | null>(null);

  function destroyEditor() {
    editor?.destroy();
    editor = null;
    if (host) host.replaceChildren();
  }

  function project(source: string) {
    if (!mounted) return;
    if (!isWysiwygSupportedSource(source)) {
      destroyEditor();
      rawSource = source;
      return;
    }
    rawSource = null;
    const content = supportedMarkdownToHtml(source);
    if (editor) {
      editor.commands.setContent(content, { emitUpdate: false });
      return;
    }
    editor = new Editor({
      element: host,
      extensions: [StarterKit.configure({ link: { openOnClick: false } })],
      content,
      editorProps: { attributes: { 'aria-label': label, role: 'textbox' } },
      onUpdate: ({ editor: updatedEditor }) => {
        const update = session.updateFromWysiwyg(tiptapJsonToMarkdown(updatedEditor.getJSON()));
        if (update) onSourceChange?.(update);
      }
    });
  }

  onMount(() => {
    mounted = true;
    project(session.draftSource);
    return destroyEditor;
  });

  $effect(() => {
    draftRevision;
    if (mounted) project(session.draftSource);
  });
</script>

<div class="wysiwyg-editor">
  {#if rawSource !== null}
    <aside class="raw-preserved" role="note" aria-label="Raw Markdown preserved">
      <strong>WYSIWYG unavailable for this Markdown</strong>
      <span>This source contains syntax that cannot be converted losslessly. It is preserved read-only.</span>
      <pre>{rawSource}</pre>
    </aside>
  {:else}
    <div class="toolbar" role="toolbar" aria-label="Formatting">
      <button type="button" onclick={() => editor?.chain().focus().toggleBold().run()}>Bold</button>
      <button type="button" onclick={() => editor?.chain().focus().toggleItalic().run()}>Italic</button>
      <button type="button" onclick={() => editor?.chain().focus().toggleCode().run()}>Code</button>
      <button type="button" onclick={() => editor?.chain().focus().toggleHeading({ level: 2 }).run()}>Heading</button>
      <button type="button" onclick={() => editor?.chain().focus().toggleBulletList().run()}>Bullets</button>
      <button type="button" onclick={() => editor?.chain().focus().toggleOrderedList().run()}>Numbered</button>
      <button type="button" onclick={() => editor?.chain().focus().toggleBlockquote().run()}>Quote</button>
    </div>
  {/if}
  <div class:hidden={rawSource !== null} bind:this={host}></div>
</div>

<style>
  .hidden { display: none; }
  .toolbar { display: flex; flex-wrap: wrap; gap: 0.35rem; margin-bottom: 0.5rem; }
  .raw-preserved { border: 1px solid currentColor; padding: 0.75rem; }
  .raw-preserved span { display: block; margin-top: 0.25rem; }
  .raw-preserved pre { overflow: auto; margin-bottom: 0; white-space: pre-wrap; }
  :global(.wysiwyg-editor .tiptap) { min-height: 8rem; outline: none; }
</style>
