// Which intent a click sends: exactly the id tables of the view contract (plan, «The view contract»).
// A null means the id is not one the section sends; the page then sends nothing. Whether the intent
// is allowed right now is Rust's decision, never decided here.

import type { AnyIntent, OptionView, SectionId } from './types.ts';

/** Top bar action ids. */
export function actionIntent(s: SectionId, id: string): AnyIntent | null {
  switch (id) {
    case 'analyze': return s === 'cleanup' || s === 'developers' ? { kind: 'analyze' } : null;
    case 'scan': return s === 'registry' ? { kind: 'scan' } : null;
    case 'clean': return s === 'cleanup' || s === 'developers' ? { kind: 'openConfirm' } : null;
    case 'fix': return s === 'registry' ? { kind: 'openFix' } : null;
    case 'cancel': return s === 'cleanup' || s === 'developers' ? { kind: 'cancel' } : null;
    default: return null;
  }
}

/** Dialog button ids. */
export function buttonIntent(s: SectionId, id: string): AnyIntent | null {
  switch (id) {
    case 'cancel': return { kind: 'closeDialog' };
    case 'continue': return s === 'cleanup' ? { kind: 'confirm' } : null;
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
  if (o.kind === 'text') return null;
  if (s === 'cleanup') {
    if (o.id === 'onlyOld' && o.kind === 'toggle' && typeof value === 'boolean') return { kind: 'setOnlyOld', on: value };
    if (o.id === 'keepList' && o.kind === 'button') return { kind: 'openKeepList' };
  } else if (s === 'registry') {
    const m = /^cat:(\d+)$/.exec(o.id);
    if (m && o.kind === 'toggle') return { kind: 'toggleCategory', index: Number(m[1]) };
  } else {
    if (o.id === 'inactiveDays' && o.kind === 'number' && typeof value === 'number' && Number.isFinite(value)) return { kind: 'setInactiveDays', days: value };
    if (o.id === 'removeMode' && o.kind === 'choice' && (value === 'recycle' || value === 'permanent')) return { kind: 'setRemoveMode', mode: value };
    if (o.id === 'layout' && o.kind === 'choice' && (value === 'tree' || value === 'flat')) return { kind: 'setLayout', layout: value };
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

/**
 * A category of bar 2 that is a list of its own: Developers projects / worktrees / sessions. Cleanup shows all its categories in
 * one list and its bar 2 only scrolls to one (the page's own business, no intent).
 */
export function selectIntent(s: SectionId, nav: string): AnyIntent | null {
  if (s === 'developers' && (nav === 'projects' || nav === 'worktrees' || nav === 'sessions')) return { kind: 'select', category: nav };
  return null;
}

export function searchIntent(s: SectionId, text: string): AnyIntent | null {
  return s === 'cleanup' ? { kind: 'search', text } : null;
}

/** The check box at the top of the list: every section with ticks has one (Rust ticks what can be ticked, and never a «caution» item). */
export function bulkIntent(_s: SectionId, on: boolean): AnyIntent | null {
  return { kind: 'selectAll', on };
}
