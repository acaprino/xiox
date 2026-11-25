// The window of Xiox: Imago's `AppWindow` with Xiox's views, and in its regions what each view says. The controller (src/controller.ts)
// holds what the page remembers and asks Rust; this file puts it in the shell's regions and tells the window what the shell cannot know:
// the section in its title and the job on its taskbar button.
import { useEffect, useMemo, useRef } from 'react';
import { AppWindow, useNotify, useWindow, type AppView } from '@acaprino/imago';
import { Empty, Note } from '@acaprino/sigrid/components';
import { Button, Progress, Text } from '@acaprino/sigrid/primitives';
import { Controller, type ViewId } from '../controller.ts';
import { footerEntries } from '../commands.ts';
import { actionIntent } from '../intents.ts';
import { disabledReason } from '../policy.ts';
import { shownRows } from '../state.ts';
import { useXioxCommands } from './commands.tsx';
import { Confirm } from './confirm.tsx';
import { ControllerContext, useController } from './hooks.ts';
import { SectionInspector } from './inspector.tsx';
import { SectionScreen } from './section.tsx';
import { Settings } from './settings.tsx';
import { SectionSidebar } from './sidebar.tsx';
import { Status } from './status.tsx';

const VIEWS: AppView<ViewId>[] = [
  { id: 'cleanup', label: 'Cleanup', icon: 'delete', command: 'Go to Cleanup' },
  { id: 'registry', label: 'Registry', icon: 'key', command: 'Go to Registry' },
  { id: 'developers', label: 'Developers', icon: 'code', command: 'Go to Developers' },
  { id: 'settings', label: 'Settings', icon: 'settings', command: 'Go to Settings' },
];

const TITLES: Record<ViewId, string> = { cleanup: 'Cleanup', registry: 'Registry', developers: 'Developers', settings: 'Settings' };

function Window() {
  const c = useController();
  const notify = useNotify();
  const window_ = useWindow();
  const field = useRef<HTMLInputElement>(null);
  c.fail = (text) => notify(text, { kind: 'error', id: 'xiox.error' });
  useXioxCommands(c);
  useEffect(() => { void c.load(); }, [c]);

  // The title of the window says the view (Alt+Tab and screen readers tell Cleanup from Registry), and the taskbar button the running job.
  const open = c.open;
  const job = Object.values(c.ui.views).find((v) => v?.job)?.job ?? null;
  const percent = job === null ? null : Math.min(100, Math.max(0, Math.round(job.fraction * 100)));
  useEffect(() => { window_.setTitle(`${TITLES[open]} - Xiox`); }, [window_, open]);
  useEffect(() => { window_.setProgress(percent); }, [window_, percent]);

  // A native dialog closing gives the window back, but WebView2 does not always put the focus back in the page: with nothing focused the
  // list takes it, so that the keys work without a click.
  useEffect(() => {
    const back = (): void => {
      if (document.activeElement === null || document.activeElement === document.body) document.querySelector<HTMLElement>('[data-slot="table-row"][tabindex="0"]')?.focus();
    };
    window.addEventListener('focus', back);
    return () => window.removeEventListener('focus', back);
  }, []);

  const view = c.view;
  const settings = c.ui.settings;
  const searchable = !settings && view !== undefined && view.search !== null && view.report === null;
  const shown = useShown(c);
  const row = view === undefined ? null : c.detailRow(view, shown);

  let screen;
  if (settings && c.about !== null) screen = <Settings about={c.about} />;
  else if (view !== undefined) screen = <SectionScreen c={c} app={c.app} view={view} field={field} />;
  else if (c.failure !== null) {
    screen = (
      <Empty fill title="Xiox could not start" description={<Note tone="error" title={c.failure} actions={<Button variant="outline" onClick={() => { void c.load(); }}>Retry</Button>} />} />
    );
  } else screen = <Empty fill title="Opening Xiox" description="Reading what is on this computer." action={<Progress inline aria-label="Opening Xiox" value={null} />} />;

  return (
    <AppWindow
      views={VIEWS}
      view={open}
      onView={(next) => c.choose(next)}
      search={searchable ? () => field.current?.focus() : null}
      status={<Status app={c.app} view={view} settings={settings} />}
      sidebar={(state) => <SectionSidebar c={c} state={state} />}
      inspector={!settings && view !== undefined && view.report === null ? <SectionInspector c={c} view={view} row={row} /> : undefined}
      actions={settings || view === undefined ? undefined : (
        <>
          {view.actions.map((action) => {
            const reason = disabledReason(view, action);
            return (
              <Button
                key={action.id}
                variant={action.primary ? 'default' : 'outline'}
                disabled={!action.enabled}
                title={reason ?? undefined}
                aria-describedby={reason === null ? undefined : `why-${action.id}`}
                onClick={() => { void c.send(view.section, actionIntent(view.section, action.id)); }}
              >{action.label}</Button>
            );
          })}
          {view.actions.map((action) => disabledReason(view, action) === null ? null : <Text srOnly key={action.id} id={`why-${action.id}`}>{disabledReason(view, action)}</Text>)}
        </>
      )}
      footer={<Text variant="muted" truncate>{settings ? '' : (view?.footer ?? '')}</Text>}
      hints={view === undefined || settings ? [] : footerEntries(view)}
    >
      {screen}
      {view === undefined ? null : <Confirm c={c} view={view} />}
    </AppWindow>
  );
}

/** The rows on screen, as the list draws them. */
function useShown(c: Controller) {
  const view = c.view;
  return useMemo(() => (view === undefined ? [] : shownRows(c.ui, view)), [c, view, c.layout]);
}

export function Xiox() {
  const controller = useMemo(() => new Controller(), []);
  return (
    <ControllerContext value={controller}>
      <Window />
    </ControllerContext>
  );
}
