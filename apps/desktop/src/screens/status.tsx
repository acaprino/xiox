// The status row: what the section says (a badge and a sentence), the job that runs (its label, a bar, its percent and detail) and, at
// the end, who runs the program and how much memory there is. One live region: the shell reads it when the phrase changes.
import { Badge, Progress, Spacer, Text } from '@acaprino/sigrid/primitives';
import { ramText } from '../format.ts';
import type { AppInfo, SectionView } from '../types.ts';

/** The tone of the badge from what the view is: a job runs, a report is there (done, or cancelled), or nothing special. */
function toneOf(view: SectionView): 'info' | 'success' | 'warning' | undefined {
  if (view.job !== null) return 'info';
  if (view.report === null) return undefined;
  return view.report.cancelled ? 'warning' : 'success';
}

export function Status({ app, view, settings }: { app: AppInfo | null; view: SectionView | undefined; settings: boolean }) {
  // Settings has no work of its own: the section's sentence and job stay with the section.
  if (settings) view = undefined;
  const percent = view?.job ? Math.round(Math.min(1, Math.max(0, view.job.fraction)) * 100) : null;
  return (
    <>
      <Badge tone={view === undefined ? undefined : toneOf(view)}>{settings ? 'Ready' : (view?.status.badge ?? 'Starting')}</Badge>
      {view?.job ? (
        <>
          <Text truncate>{view.job.label}</Text>
          <Progress inline size="sm" aria-label={view.job.label} value={percent} showValue />
          {view.job.detail ? <Text variant="muted" truncate>{view.job.detail}</Text> : null}
        </>
      ) : view?.status.text ? <Text variant="soft" truncate>{view.status.text}</Text> : null}
      <Spacer />
      {app === null ? null : <Text variant="muted">{app.admin ? 'administrator' : 'standard user'} · {ramText(app.ramTotalBytes)}</Text>}
    </>
  );
}
