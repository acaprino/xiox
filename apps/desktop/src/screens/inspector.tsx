// The inspector (Sigrid's, not modal): what the content header's `Details` (`i`) and `Options` (`o`) open. Details are the facts of the
// row the selection is on (item, files, size, level, mode, why) and any free lines; Options are the section's options, a form that saves
// itself. The shell closes it with Escape and tells the page (`onOpenChange`).
import { DescriptionList, Note } from '@acaprino/sigrid/components';
import { Column, Text, TextList } from '@acaprino/sigrid/primitives';
import { Inspector } from '@acaprino/sigrid/layout';
import type { Controller } from '../controller.ts';
import type { RowView, SectionView } from '../types.ts';
import { OptionControl, panelOptions } from './options.tsx';

const NUMBERS = new Set(['files', 'size']);
const word = (key: string): string => key.charAt(0).toUpperCase() + key.slice(1);

/** What a row's detail says: its facts, then the reason (`why`) and any free lines; a blocked row says why it cannot be chosen. */
function Details({ row }: { row: RowView }) {
  const facts = row.detail.filter(([key]) => key !== 'why');
  const reasons = row.detail.filter(([key]) => key === 'why').map(([, value]) => value);
  if (reasons.length === 0 && row.why && !row.detail.some(([, value]) => value === row.why)) reasons.push(row.why);
  const blocked = row.muted && !row.enabled && row.why ? row.why : null;
  return (
    <Column gap="lg">
      {facts.length === 0 ? null : <DescriptionList aria-label="Facts" items={facts.map(([key, value]) => ({ name: word(key), value: NUMBERS.has(key) ? <Text>{value}</Text> : value, code: key === 'key' || key === 'data' || key === 'path' }))} />}
      {blocked === null ? null : <Note tone="warning" title="Blocked">{blocked}</Note>}
      {reasons.length === 0 ? null : <DescriptionList aria-label="Why" items={reasons.map((value) => ({ name: 'Why', value }))} />}
      {row.lines.length === 0 ? null : <TextList items={row.lines.map((line) => <Text mono key={line}>{line}</Text>)} />}
    </Column>
  );
}

export function SectionInspector({ c, view, row }: { c: Controller; view: SectionView; row: RowView | null }) {
  const kind = c.ui.inspector;
  const options = panelOptions(view);
  return (
    <Inspector title={kind === 'options' ? 'Options' : 'Details'} open={kind !== null} onOpenChange={(open) => { if (!open) c.closeInspector(); }}>
      {kind === 'details' ? (
        row === null
          ? <Text variant="muted">Select a row to see its details: paths, files in use, why.</Text>
          : <Details row={row} />
      ) : null}
      {kind === 'options' ? (
        options.length === 0
          ? <Text variant="muted">This section has no options.</Text>
          : <Column gap="lg">{options.map((option, at) => <OptionControl key={`${option.kind === 'text' ? 'text' : option.id}:${at}`} c={c} section={view.section} option={option} />)}</Column>
      ) : null}
    </Inspector>
  );
}
