// The content of a section: its panel (the title, what is chosen, Details and Options; under them the search and the layout of the list) and in it the
// list, the report that replaces it, or what stands in its place: nothing to show (with the action that starts the work), a wait, an error.
import { useEffect, useMemo, useRef, useState, type Ref } from 'react';
import { useMedia } from '@acaprino/sigrid';
import { Empty, SearchField } from '@acaprino/sigrid/components';
import { Panel } from '@acaprino/sigrid/layout';
import { Button, Cell, Icon, Progress } from '@acaprino/sigrid/primitives';
import type { Controller } from '../controller.ts';
import { actionIntent, optionIntent, searchIntent } from '../intents.ts';
import { emptyAction, flatToggle, layoutChoice, panelOptions } from '../policy.ts';
import { foldedGroups, searching, shownRows } from '../state.ts';
import type { AppInfo, SectionView } from '../types.ts';
import { useAfter } from './hooks.ts';
import { List } from './list.tsx';
import { Notice, ProfileNote } from './notices.tsx';
import { Report } from './report.tsx';

const SEARCH_DELAY_MS = 150;

/** The search box: the text being typed is the person's until the view agrees with it; the intent goes after a short pause. */
function Search({ c, view, field }: { c: Controller; view: SectionView; field: Ref<HTMLInputElement> }) {
  const search = view.search!;
  const [text, setText] = useState(search.value);
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined);
  // What Rust says wins when it changed and the box is not being typed in.
  useEffect(() => {
    if (document.activeElement !== (field as { current: HTMLInputElement | null }).current) setText(search.value);
  }, [search.value, field]);
  return (
    <SearchField
      ref={field}
      aria-label={search.placeholder}
      placeholder={search.placeholder}
      value={text}
      onValueChange={(next) => {
        setText(next);
        clearTimeout(timer.current);
        timer.current = setTimeout(() => { void c.send(view.section, searchIntent(view.section, next)); }, SEARCH_DELAY_MS);
      }}
    />
  );
}

export function SectionScreen({ c, app, view, field }: { c: Controller; app: AppInfo | null; view: SectionView; field: Ref<HTMLInputElement> }) {
  const ui = c.ui;
  const rows = useMemo(() => shownRows(ui, view), [ui, view, c.layout]);
  const blank = view.rows.length === 0 || view.empty !== null;
  const waiting = useAfter(blank && view.job !== null);
  const start = emptyAction(view);
  const groups = view.rows.filter((r) => r.kind === 'group');
  const anyOpen = groups.some((g) => !foldedGroups(ui, view).has(g.id));
  const hasOptions = panelOptions(view).length > 0;
  const layout = layoutChoice(view);
  // Beside Options in a wide window; under the header in a narrow one, whose row wraps (the header's does not, and would run off the page).
  const wide = useMedia('(width >= 860px)');
  const toggle = layout ? flatToggle(layout) : null;
  const flat = layout && toggle ? (
    <Button
      variant="ghost" size="sm" aria-pressed={toggle.pressed} disabled={!layout.enabled}
      title="Every project in one list, with its full path: the order by size runs over the whole list"
      onClick={() => { void c.send(view.section, optionIntent(view.section, layout, toggle.next)); }}
    ><Icon name="layout" />{toggle.label}</Button>
  ) : null;

  if (view.report !== null) return <Report c={c} section={view.section} report={view.report} />;

  const tools = (
    <>
      {app?.profileMismatch ? <ProfileNote text={app.profileMismatch} /> : null}
      {view.notices.map((notice, at) => <Notice key={`${at}:${notice.text}`} notice={notice} />)}
      {view.search ? <Cell grow><Search c={c} view={view} field={field} /></Cell> : null}
      {groups.length >= 2 ? (
        <Button variant="outline" size="sm" disabled={searching(view)} onClick={() => c.foldAll(anyOpen)}>{anyOpen ? 'Fold all' : 'Open all'}</Button>
      ) : null}
      {wide ? null : flat}
    </>
  );
  return (
    <Panel
      title={view.title}
      subtitle={view.subtitle}
      actions={
        <>
          {wide ? flat : null}
          <Button variant="ghost" size="sm" aria-pressed={ui.inspector === 'details'} title="The details of the selected row (i)" onClick={() => c.toggleInspector('details')}><Icon name="info" />Details</Button>
          <Button variant="ghost" size="sm" aria-pressed={ui.inspector === 'options'} disabled={!hasOptions} title={hasOptions ? 'The options of this section (o)' : 'This section has no options'} onClick={() => c.toggleInspector('options')}><Icon name="settings" />Options</Button>
        </>
      }
      tools={tools}
    >
      {blank ? (
        <Empty
          fill
          title={waiting ? view.job?.label : (view.empty?.title ?? 'Nothing to show')}
          description={waiting ? (view.job?.detail ?? undefined) : (view.empty?.text ?? 'Nothing to show.')}
          action={waiting
            ? <Progress inline aria-label={view.job?.label ?? 'Working'} value={view.job ? Math.round(view.job.fraction * 100) : null} />
            : start === null ? undefined : <Button variant="outline" onClick={() => { void c.send(view.section, actionIntent(view.section, start.id)); }}>{start.label}</Button>}
        />
      ) : <List c={c} view={view} rows={rows} />}
    </Panel>
  );
}
