<script lang="ts">
  import { onMount } from 'svelte';
  import { markdown } from '@codemirror/lang-markdown';
  import { Compartment, EditorState, EditorSelection } from '@codemirror/state';
  import { EditorView } from '@codemirror/view';
  import {
    type NoteSessionStatus,
    type ProjectionOrigin,
    type TextSelection
  } from './note-session';

  export let source = '';
  export let onChange: (update: {
    origin: ProjectionOrigin;
    source: string;
    selection: TextSelection;
  }) => void = () => {};
  export let origin: ProjectionOrigin = 'source';
  export let ariaLabel = 'Markdown source';
  export let readonly = false;
  export let status: NoteSessionStatus | string = 'clean';
  export let selection: TextSelection | null = null;

  let host: HTMLDivElement;
  let view: EditorView | null = null;
  let applyingExternal = false;
  let mounted = false;
  const editable = new Compartment();
  const attributes = new Compartment();
  const lineSeparator = new Compartment();

  const separatorFor = (value: string) => value.includes('\r\n') ? '\r\n' : '\n';

  const clampSelection = (value: TextSelection, length: number): TextSelection => ({
    anchor: Math.max(0, Math.min(length, Math.trunc(Number.isFinite(value.anchor) ? value.anchor : 0))),
    head: Math.max(0, Math.min(length, Math.trunc(Number.isFinite(value.head) ? value.head : 0)))
  });

  function sourceOffsetToDoc(sourceValue: string, offset: number): number {
    const clamped = Math.max(0, Math.min(sourceValue.length, Math.trunc(Number.isFinite(offset) ? offset : 0)));
    if (separatorFor(sourceValue) !== '\r\n') return clamped;
    let carriageReturns = 0;
    for (let index = 0; index + 1 < clamped; index += 1) {
      if (sourceValue[index] === '\r' && sourceValue[index + 1] === '\n') carriageReturns += 1;
    }
    return clamped - carriageReturns;
  }

  function docOffsetToSource(state: EditorState, offset: number): number {
    return state.sliceDoc(0, offset).length;
  }

  function currentSelection(): TextSelection {
    if (!view) return { anchor: 0, head: 0 };
    const main = view.state.selection.main;
    return {
      anchor: docOffsetToSource(view.state, main.anchor),
      head: docOffsetToSource(view.state, main.head)
    };
  }

  function editorSource(state = view?.state): string {
    return state?.sliceDoc(0, state.doc.length) ?? '';
  }

  function syncExternalSource(nextSource: string, nextSelectionProp: TextSelection | null) {
    if (!view || !mounted || editorSource() === nextSource) return;
    const nextSelection = clampSelection(nextSelectionProp ?? currentSelection(), nextSource.length);
    applyingExternal = true;
    try {
      view.dispatch({
        changes: { from: 0, to: view.state.doc.length, insert: nextSource },
        selection: EditorSelection.single(
          sourceOffsetToDoc(nextSource, nextSelection.anchor),
          sourceOffsetToDoc(nextSource, nextSelection.head)
        ),
        effects: lineSeparator.reconfigure(EditorState.lineSeparator.of(separatorFor(nextSource)))
      });
    } finally {
      applyingExternal = false;
    }
  }

  function syncAttributes(nextReadonly: boolean, nextLabel: string) {
    if (!view) return;
    view.dispatch({
      effects: [
        editable.reconfigure(EditorView.editable.of(!nextReadonly)),
        attributes.reconfigure(EditorView.contentAttributes.of({
          'aria-label': nextLabel,
          'aria-readonly': String(nextReadonly),
          spellcheck: 'false'
        }))
      ]
    });
  }

  $: if (mounted) {
    syncExternalSource(source, selection);
    syncAttributes(readonly, ariaLabel);
  }

  onMount(() => {
    view = new EditorView({
      state: EditorState.create({
        doc: source,
        selection: EditorSelection.single(
          sourceOffsetToDoc(source, clampSelection(selection ?? { anchor: 0, head: 0 }, source.length).anchor),
          sourceOffsetToDoc(source, clampSelection(selection ?? { anchor: 0, head: 0 }, source.length).head)
        ),
        extensions: [
          markdown(),
          lineSeparator.of(EditorState.lineSeparator.of(separatorFor(source))),
          editable.of(EditorView.editable.of(!readonly)),
          attributes.of(EditorView.contentAttributes.of({
            'aria-label': ariaLabel,
            'aria-readonly': String(readonly),
            spellcheck: 'false'
          })),
          EditorView.updateListener.of((update) => {
            if (!update.docChanged || applyingExternal) return;
            const main = update.state.selection.main;
            onChange({
              origin,
              source: update.state.sliceDoc(0, update.state.doc.length),
              selection: {
                anchor: docOffsetToSource(update.state, main.anchor),
                head: docOffsetToSource(update.state, main.head)
              }
            });
          })
        ]
      }),
      parent: host
    });
    mounted = true;

    return () => {
      mounted = false;
      view?.destroy();
      view = null;
    };
  });
</script>

<div class="source-editor" data-status={status}>
  <div bind:this={host} class="source-editor__surface"></div>
  <p class="source-editor__status" role="status" aria-label="Editor status">{status}</p>
</div>
