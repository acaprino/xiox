// The list of the section: Sigrid's `DataTable` as a tree with a choice. The page draws the rows in the order of `state.ts` (`shownRows`: the
// order the user chose, children under their parent, nothing under a folded category) and sends what the user does as intents; it never
// flips a check itself. A category is a group row that stays in sight under the headings, a sub-heading with a box is a row of its own, a
// folder of the projects folds, and a project opens its folders to clean. The keys of the table are Sigrid's (arrows, Home, End, Left and
// Right to fold); the page takes only Space (tick) and Enter (the primary action).
import { useEffect, useMemo, useRef } from 'react';
import { useMedia } from '@acaprino/sigrid';
import { DataTable, type DataColumn, type DataSelection, type DataSort } from '@acaprino/sigrid/table';
import { Badge, Cell, Cluster, Column, Spacer, Text } from '@acaprino/sigrid/primitives';
import { listKey } from '../commands.ts';
import type { Controller } from '../controller.ts';
import { formatBytes } from '../format.ts';
import { bulkIntent } from '../intents.ts';
import { activeSort, headerBytes, rowDepths, rowParents } from '../rows.ts';
import { bulkCheck, bulkLocked, currentFocus, foldedGroups } from '../state.ts';
import type { ColumnView, RowView, SectionView } from '../types.ts';

const sizeText = (row: RowView): string => (row.bytes !== null ? formatBytes(row.bytes) : (row.sizeText ?? '—'));

/** The level as a badge: recommended is a success, caution a warning, optional the plain badge. */
function Level({ level }: { level: NonNullable<RowView['level']> }) {
  const tone = level === 'recommended' ? 'success' : level === 'caution' ? 'warning' : undefined;
  return <Badge tone={tone} variant={tone === undefined ? 'secondary' : undefined}>{level.charAt(0).toUpperCase() + level.slice(1)}</Badge>;
}

/** The name of a row, and under it what it is: the path or the reason, or, for a blocked row, why it cannot be chosen. */
function Name({ row, withLevel }: { row: RowView; withLevel: boolean }) {
  const blocked = row.muted && !row.enabled && row.why !== null;
  const line = blocked ? row.why : row.sub;
  return (
    <Column gap="xs">
      <Text variant={row.kind === 'header' ? 'strong' : 'body'} truncate>{row.name}</Text>
      {line ? <Text variant="muted" tone={blocked ? 'warning' : undefined} truncate>{line}</Text> : null}
      {withLevel && row.level && row.kind !== 'group' ? <Cluster><Level level={row.level} /></Cluster> : null}
    </Column>
  );
}

/** The category's line: its name and what is ticked in it, then how many files it holds and how much they add up to. */
function Group({ row, note, narrow }: { row: RowView; note: string; narrow: boolean }) {
  return (
    <Cell grow>
      <Cluster wrap={false} gap="md">
        <Text variant="strong">{row.name}</Text>
        {note && !narrow ? <Text variant="muted" truncate>{note}</Text> : null}
        <Spacer />
        {row.cells[0] && !narrow ? <Text variant="muted">{row.cells[0]}</Text> : null}
        {row.bytes !== null ? <Text>{formatBytes(row.bytes)}</Text> : null}
      </Cluster>
    </Cell>
  );
}

/**
 * The columns the view says: a name, cells, a level, a size. A press on a heading asks for that order (the page puts the rows in it).
 * Where the list is narrow it keeps two (the name, with the level under it, and the size): a name needs room to be read.
 */
function columnsOf(view: SectionView, narrow: boolean): DataColumn<RowView>[] {
  let cell = 0;
  const kept = narrow ? view.columns.filter((column) => column.kind === 'name' || column.kind === 'size') : view.columns;
  return kept.map((column: ColumnView): DataColumn<RowView> => {
    const common = { id: column.title, header: column.title, label: column.title, sortValue: () => null };
    switch (column.kind) {
      case 'name': return { ...common, grow: true, cell: (row) => <Name row={row} withLevel={narrow} /> };
      case 'cell': {
        const at = cell++;
        return { ...common, align: 'end', cell: (row) => <Text>{row.cells[at] ?? ''}</Text> };
      }
      case 'level': return { ...common, cell: (row) => (row.level && row.kind !== 'group' ? <Level level={row.level} /> : null) };
      case 'size': return { ...common, align: 'end', cell: (row) => <Text>{row.kind === 'header' ? bytesOfHeader(view, row) : sizeText(row)}</Text> };
    }
  });
}

/** What the rows under a sub-heading add up to, in its own line. */
function bytesOfHeader(view: SectionView, row: RowView): string {
  const bytes = headerBytes(view.rows, row.id);
  return bytes === null ? '' : formatBytes(bytes);
}

/** The category the list is at: the last one whose line has reached the top, for the menu beside it. */
function categoryAt(box: HTMLElement): string | null {
  const head = box.querySelector('thead')?.getBoundingClientRect().bottom ?? box.getBoundingClientRect().top;
  let at: string | null = null;
  for (const group of box.querySelectorAll<HTMLElement>('tr[data-group]')) {
    if (group.getBoundingClientRect().top > head + 8) break;
    at = group.dataset['rowId'] ?? null;
  }
  return at;
}

/** Where a category really starts in the box, whatever a sticky line is doing: under the last row before it that does not stick. */
function categoryTop(box: HTMLElement, group: HTMLElement): number {
  let height = 0;
  let before: Element | null = group.previousElementSibling;
  while (before instanceof HTMLElement && before.hasAttribute('data-group')) {
    height += before.offsetHeight;
    before = before.previousElementSibling;
  }
  const edge = before instanceof HTMLElement ? before.getBoundingClientRect().bottom : (box.querySelector('thead')?.getBoundingClientRect().bottom ?? 0);
  return edge + height - box.getBoundingClientRect().top + box.scrollTop;
}

/** After a click on the menu the marked entry is the one clicked, however far the scroll gets (the last categories cannot reach the top). */
const PIN_MS = 900;

export function List({ c, view, rows }: { c: Controller; view: SectionView; rows: readonly RowView[] }) {
  const ui = c.ui;
  const parents = useMemo(() => rowParents(view.rows), [view]);
  const folded = foldedGroups(ui, view);
  const depths = useMemo(() => rowDepths(view.rows), [view]);
  const narrow = !useMedia('(width >= 620px)');
  const columns = useMemo(() => columnsOf(view, narrow), [view, narrow]);
  const focus = currentFocus(ui, view);
  const byId = useMemo(() => new Map(view.rows.map((r) => [r.id, r])), [view]);
  const busy = view.job !== null;
  const check = bulkCheck(view);
  const active = activeSort(view.columns, ui.sort[view.section] ?? null);
  const sort: DataSort = active === null ? null : { column: active.column, descending: active.desc };
  const noteOf = (row: RowView): string => [view.nav.find((n) => n.id === row.id)?.note, row.why].filter(Boolean).join(' · ');

  const selection: DataSelection<RowView> = {
    isSelected: (row) => (row.check === 'none' ? null : row.check === 'on' ? true : row.check === 'mixed' ? 'mixed' : false),
    isDisabled: (row) => !row.enabled || busy,
    onSelectedChange: (row) => c.toggleRow(row.id, view.section),
    label: (row) => (row.kind === 'group' || row.kind === 'header' ? `${row.name}: tick or clear every item that can be ticked` : row.name),
    ...(view.bulk && bulkIntent(view.section, true) !== null
      ? {
        selectAll: check === 'on' ? true : check === 'mixed' ? ('mixed' as const) : false,
        onSelectAll: (checked: boolean) => { void c.send(view.section, bulkIntent(view.section, checked)); },
        selectAllDisabled: check === 'none' || bulkLocked(view),
      }
      : {}),
  };

  const open = (row: RowView): boolean | undefined => {
    if (row.kind === 'group') return !folded.has(row.id);
    if (!parents.has(row.id)) return undefined;
    return row.kind === 'header' ? !folded.has(row.id) : ui.expanded.has(row.id);
  };

  // The details follow the row the focus is on, by keyboard too: the box tells which row took it.
  const element = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const here = element.current;
    if (here === null) return;
    const follow = (event: FocusEvent): void => {
      const id = event.target instanceof Element ? event.target.closest<HTMLElement>('tr[data-row-id]')?.dataset['rowId'] : undefined;
      const row = id === undefined ? undefined : byId.get(id);
      if (row !== undefined && row.kind !== 'group' && !(row.kind === 'header' && row.check === 'none' && !parents.has(row.id))) c.focusRow(row.id);
    };
    here.addEventListener('focusin', follow);
    return () => here.removeEventListener('focusin', follow);
  }, [c, byId, parents]);

  // The menu marks the category the list is at, and a click on it scrolls the list there.
  const pinned = useRef(0);
  useEffect(() => {
    const here = element.current;
    if (here === null) return;
    let frame = 0;
    const update = (): void => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        if (performance.now() < pinned.current) return;
        const id = categoryAt(here);
        if (id !== null) c.setSpy(id);
      });
    };
    const release = (): void => { pinned.current = 0; };
    here.addEventListener('scroll', update, { passive: true });
    here.addEventListener('wheel', release, { passive: true });
    here.addEventListener('touchmove', release, { passive: true });
    return () => {
      cancelAnimationFrame(frame);
      here.removeEventListener('scroll', update);
      here.removeEventListener('wheel', release);
      here.removeEventListener('touchmove', release);
    };
  }, [c, view.section]);
  useEffect(() => {
    const id = c.takeScroll();
    const here = element.current;
    const group = id === null ? null : here?.querySelector<HTMLElement>(`tr[data-group][data-row-id="${CSS.escape(id)}"]`);
    if (!here || !group) return;
    pinned.current = performance.now() + PIN_MS;
    const head = here.querySelector('thead')?.getBoundingClientRect().height ?? 0;
    const reduced = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
    here.scrollTo({ top: Math.max(0, categoryTop(here, group) - head), behavior: reduced ? 'auto' : 'smooth' });
  });

  return (
    <DataTable
      fill
      aria-label={view.title}
      containerRef={element}
      columns={columns}
      rows={rows}
      rowId={(row) => row.id}
      sort={sort}
      onSortChange={(next) => c.sortBy(next?.column ?? active?.column ?? '')}
      current={focus}
      rowLevel={(row) => (depths.get(row.id) ?? 0) + 1}
      rowOpen={open}
      onRowOpenChange={(row, next) => c.setOpen(row, next)}
      isGroupRow={(row) => row.kind === 'group'}
      groupCell={(row) => <Group row={row} note={noteOf(row)} narrow={narrow} />}
      onRowPress={(row) => {
        const state = open(row);
        if (row.kind === 'group' || (row.kind === 'header' && state !== undefined)) c.setOpen(row, !state);
        else c.focusRow(row.id);
      }}
      selection={selection}
      onRowKeyDown={(row, event) => {
        const act = listKey(event);
        if (act?.kind === 'primary') {
          event.preventDefault();
          c.primary();
        } else if (act?.kind === 'toggle') {
          event.preventDefault();
          if (row.check !== 'none' && row.enabled && !busy) c.toggleRow(row.id, view.section);
        }
      }}
    />
  );
}
