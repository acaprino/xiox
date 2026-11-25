// The report that replaces the list when a clean has finished (or was cancelled): what was freed, what was skipped or failed, and each
// item. It is drawn from the ReportView alone; the page adds the headings and the number formatting.
import { Disclosure, DescriptionList, Note } from '@acaprino/sigrid/components';
import { Button, Table, TableBody, TableCell, TableHead, TableHeader, TableRow, Text, TextList, Column, Section } from '@acaprino/sigrid/primitives';
import { Panel } from '@acaprino/sigrid/layout';
import type { Controller } from '../controller.ts';
import { formatBytes, formatDuration } from '../format.ts';
import type { ReportLine, ReportView, SectionId } from '../types.ts';
import { Notice, noteTone } from './notices.tsx';

const count = (n: number): string => n.toLocaleString('en-US');

/** `Skipped (n)` / `Failed (n)`: the item and why; the full path, when there is one, is a cell of its own. */
function Lines({ title, lines }: { title: string; lines: ReportLine[] }) {
  if (lines.length === 0) return null;
  return (
    <Section title={`${title} (${lines.length})`} level={3}>
      <Table aria-label={title}>
        <TableBody>
          {lines.map((line, at) => (
            <TableRow key={at}>
              <TableCell><Text truncate>{line.item}</Text></TableCell>
              <TableCell><Text truncate>{line.why}</Text></TableCell>
              <TableCell><Text mono truncate variant="muted">{line.path}</Text></TableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>
    </Section>
  );
}

export function Report({ c, section, report }: { c: Controller; section: SectionId; report: ReportView }) {
  return (
    <Panel
      title={report.title}
      actions={
        <>
          {report.canSave ? <Button variant="outline" size="sm" onClick={() => { void c.saveReport(section); }}>Save as text…</Button> : null}
          <Button size="sm" onClick={() => { void c.send(section, { kind: 'closeDialog' }); }}>Close</Button>
        </>
      }
    >
      <Column gap="lg" pad="lg">
        <DescriptionList
          aria-label="Result"
          items={[
            { name: 'Space freed', value: <Text variant="strong">{formatBytes(report.freedBytes)}</Text> },
            report.recycledBytes > 0 && { name: 'Sent to the Recycle Bin', value: formatBytes(report.recycledBytes) },
            { name: 'Files deleted', value: count(report.files) },
            { name: 'Folders deleted', value: count(report.dirs) },
            { name: 'Duration', value: formatDuration(report.elapsedMs) },
          ]}
        />
        {report.cancelled ? <Note tone="warning" title="Cancelled">The clean stopped before the end.</Note> : null}
        {report.notes.map((note, at) => <Notice key={at} notice={note} />)}
        {report.saved === null ? null : <Note tone={noteTone(report.saved.ok ? 'ok' : 'warn')} title={report.saved.text} />}
        {report.byCategory.length === 0 ? null : (
          <Section title="By category" level={3}>
            <Table aria-label="By category">
              <TableBody>
                {report.byCategory.map(([name, bytes]) => (
                  <TableRow key={name}>
                    <TableCell><Text truncate>{name}</Text></TableCell>
                    <TableCell align="end">{formatBytes(bytes)}</TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </Section>
        )}
        <Lines title="Skipped" lines={report.skipped} />
        <Lines title="Failed" lines={report.failed} />
        {report.items.length === 0 ? null : (
          <Section title={`Items (${report.items.length})`} level={3}>
            <Table aria-label="Items">
              <TableHeader>
                <tr>
                  <TableHead>Item</TableHead>
                  <TableHead align="end">Freed</TableHead>
                  <TableHead align="end">Removed</TableHead>
                  <TableHead align="end">Left</TableHead>
                  <TableHead>Note</TableHead>
                </tr>
              </TableHeader>
              <TableBody>
                {report.items.map((item, at) => (
                  <TableRow key={at}>
                    <TableCell>
                      {item.lines.length === 0 ? <Text truncate>{item.name}</Text> : (
                        <Disclosure label={item.name}>
                          <TextList items={item.lines.map((line) => <Text mono key={line}>{line}</Text>)} />
                        </Disclosure>
                      )}
                    </TableCell>
                    <TableCell align="end">{formatBytes(item.freedBytes)}</TableCell>
                    <TableCell align="end">{count(item.removed)}</TableCell>
                    <TableCell align="end">{count(item.left)}</TableCell>
                    <TableCell><Text truncate variant="muted">{item.note ?? ''}</Text></TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </Section>
        )}
      </Column>
    </Panel>
  );
}
