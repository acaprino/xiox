// Which intent a click sends: exactly the id tables of the view contract (plan, «The view contract»).
// A null means the id is not one the section sends; the page then sends nothing. Whether the intent
// is allowed right now is Rust's decision, never decided here.

import type { AnyIntent, OptionView, SectionId } from './types.ts';

/** Top bar action ids. */
export function actionIntent(s: SectionId, id: string): AnyIntent | null {
  switch (id) {
    case 'analyze': return s === 'cleanup' || s === 'developers' ? { kind: 'analyze' } : null;
    case 'scan': return s === 'registry' ? { kind: 'scan' } : null;
    case 'refresh': return s === 'services' ? { kind: 'refresh' } : null;
    case 'clean': return s === 'cleanup' || s === 'developers' ? { kind: 'openConfirm' } : null;
    case 'fix': return s === 'registry' ? { kind: 'openFix' } : null;
    case 'disable':
    case 'manual':
    case 'restore': return s === 'services' ? { kind: 'openConfirm', action: id } : null;
    case 'cancel': return s === 'cleanup' || s === 'developers' ? { kind: 'cancel' } : null;
    default: return null;
  }
}

/** Dialog button ids. */
export function buttonIntent(s: SectionId, id: string): AnyIntent | null {
  switch (id) {
    case 'cancel': return { kind: 'closeDialog' };
    case 'continue': return s === 'cleanup' || s === 'services' ? { kind: 'confirm' } : null;
    case 'clean': return s === 'developers' ? { kind: 'confirm' } : null;
    case 'permanent': return s === 'developers' ? { kind: 'permanent' } : null;
    case 'backup': return s === 'registry' ? { kind: 'fix', backup: true } : null;
    case 'nobackup': return s === 'registry' ? { kind: 'fix', backup: false } : null;
    default: return null;
  }
}

/**
 * Bar 2 option ids. `value` is what the control now holds: a boolean for a toggle, a number for a
 * number box or the selected index of a list, the chosen id for a choice. A list also takes the word
 * `remove` (its remove button); adding a folder is the `developers_add_root` command, not an intent.
 */
export function optionIntent(s: SectionId, o: OptionView, value: boolean | number | string | null): AnyIntent | null {
  if (o.kind === 'text' || o.kind === 'meter') return null;
  if (s === 'cleanup') {
    if (o.id === 'onlyOld' && o.kind === 'toggle' && typeof value === 'boolean') return { kind: 'setOnlyOld', on: value };
    if (o.id === 'keepList' && o.kind === 'button') return { kind: 'openKeepList' };
  } else if (s === 'registry') {
    const m = /^cat:(\d+)$/.exec(o.id);
    if (m && o.kind === 'toggle') return { kind: 'toggleCategory', index: Number(m[1]) };
  } else if (s === 'services') {
    if (o.id === 'filter' && o.kind === 'choice' && (value === 'recommended' || value === 'all')) return { kind: 'setAll', all: value === 'all' };
  } else {
    if (o.id === 'inactiveDays' && o.kind === 'number' && typeof value === 'number' && Number.isFinite(value)) return { kind: 'setInactiveDays', days: value };
    if (o.id === 'removeMode' && o.kind === 'choice' && (value === 'recycle' || value === 'permanent')) return { kind: 'setRemoveMode', mode: value };
    if (o.id === 'scanRoots' && o.kind === 'toggle' && typeof value === 'boolean') return { kind: 'setScanRoots', on: value };
    if (o.id === 'roots' && o.kind === 'list') {
      if (value === 'remove') return { kind: 'removeRoot' };
      if (value === null || typeof value === 'number') return { kind: 'selectRoot', index: value };
    }
  }
  return null;
}

export function toggleIntent(_s: SectionId, row: string): AnyIntent {
  return { kind: 'toggle', row };
}

/** A category of bar 2: Cleanup groups, Developers projects / worktrees / sessions. */
export function selectIntent(s: SectionId, nav: string): AnyIntent | null {
  if (s === 'cleanup') return { kind: 'select', category: nav };
  if (s === 'developers' && (nav === 'projects' || nav === 'worktrees' || nav === 'sessions')) return { kind: 'select', category: nav };
  return null;
}

export function searchIntent(s: SectionId, text: string): AnyIntent | null {
  return s === 'cleanup' || s === 'services' ? { kind: 'search', text } : null;
}

/** The `All` / `None` buttons of the list header. */
export function bulkIntent(s: SectionId, on: boolean): AnyIntent | null {
  return s === 'registry' || s === 'developers' ? { kind: 'selectAll', on } : null;
}
