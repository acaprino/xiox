// The confirmation: Sigrid's `Dialog`, drawn from `view.dialog` and open while the view has one. The focus starts on Cancel, for every
// confirmation (the permanent ones included: a second Enter never approves). A close the person asks for (Escape, the close button, a
// press outside) is `closeDialog`; Rust answers with the view without the dialog. The page holds Enter and Space for a moment after the
// dialog opens, and while they are held down (`dialogKeySwallowed`): the Enter that opened it must not also confirm it.
import { useEffect, useRef, type KeyboardEvent } from 'react';
import { Checkbox, Button, Table, TableBody, TableCell, TableRow, Text, ScrollArea, TextList, Column } from '@acaprino/sigrid/primitives';
import { Dialog, Note } from '@acaprino/sigrid/components';
import { dialogKeySwallowed } from '../commands.ts';
import type { Controller } from '../controller.ts';
import { buttonIntent } from '../intents.ts';
import type { ButtonView, DialogView, SectionView } from '../types.ts';

/** The buttons in the order of a confirmation: Cancel first, the others as Rust wrote them, the confirming one last. */
const ordered = (buttons: ButtonView[]): ButtonView[] => [
  ...buttons.filter((b) => b.id === 'cancel'),
  ...buttons.filter((b) => b.id !== 'cancel' && !b.primary),
  ...buttons.filter((b) => b.id !== 'cancel' && b.primary),
];

function Body({ c, view, dialog }: { c: Controller; view: SectionView; dialog: DialogView }) {
  return (
    <Column gap="md">
      {dialog.text.map((line) => <Text as="p" key={line}>{line}</Text>)}
      {dialog.error === null ? null : <Note tone="error" title={dialog.error} />}
      {dialog.summary.map((line) => <Text as="p" variant="strong" key={line}>{line}</Text>)}
      {dialog.rows.length === 0 ? null : (
        <ScrollArea aria-label="Items" bounded>
          <Table aria-label="Items">
            <TableBody>
              {dialog.rows.map(([label, size]) => (
                <TableRow key={label}>
                  <TableCell><Text truncate>{label}</Text></TableCell>
                  <TableCell align="end">{size}</TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </ScrollArea>
      )}
      {dialog.warnings.length === 0 && dialog.warningsTitle === null ? null : (
        <Note tone="warning" title={dialog.warningsTitle ?? undefined} items={dialog.warnings} />
      )}
      {dialog.cautions.length === 0 && dialog.cautionTitle === null ? null : (
        <Note tone="warning" title={dialog.cautionTitle ?? undefined} items={dialog.cautions.map(([label, note]) => <span key={label}><Text variant="strong">{label}</Text>: {note}</span>)} />
      )}
      {dialog.ack === null ? null : (
        <Checkbox label="I have read the warnings" checked={dialog.ack} onCheckedChange={(on) => { void c.send(view.section, { kind: 'setAck', on: on === true }); }} />
      )}
    </Column>
  );
}

export function Confirm({ c, view }: { c: Controller; view: SectionView }) {
  const dialog = view.dialog;
  const openedAt = useRef(0);
  const id = dialog === null ? null : `${view.section}:${dialog.id}`;
  useEffect(() => { if (id !== null) openedAt.current = performance.now(); }, [id]);
  // Enter and Space on a button become a click in the browser's own handler (Space on key-up): held back here while they are held or too soon.
  const guard = (event: KeyboardEvent): void => {
    if (dialogKeySwallowed(event, performance.now() - openedAt.current)) { event.preventDefault(); event.stopPropagation(); }
  };
  const last = useRef<DialogView | null>(null);
  if (dialog !== null) last.current = dialog;
  // While it closes the dialog keeps what it said, so that it does not empty before it goes.
  const shown = dialog ?? last.current;
  if (shown === null) return null;
  const permanent = shown.permanent;
  return (
    <Dialog
      open={dialog !== null}
      onOpenChange={(open) => { if (!open) void c.send(view.section, { kind: 'closeDialog' }); }}
      title={shown.title}
      width="lg"
      footer={ordered(shown.buttons).map((b) => (
        <Button
          key={b.id}
          variant={b.primary ? (permanent ? 'destructive' : 'default') : 'outline'}
          disabled={!b.enabled}
          autoFocus={b.id === 'cancel'}
          onKeyDown={guard}
          onKeyUp={(event) => { if (event.key === ' ') guard(event); }}
          onClick={() => { void c.send(view.section, buttonIntent(view.section, b.id)); }}
        >{b.label}</Button>
      ))}
    >
      <Body c={c} view={view} dialog={shown} />
    </Dialog>
  );
}
