// The sidebar of each view, in Sigrid's `Sidebar` that the shell gives its state: the categories of Cleanup (a menu that scrolls the list
// to one of them and marks the one the list is at), the kinds of issue Registry looks for (each a check), the lists of Developers (each
// asks Rust for its own) and the pages of Settings. Its head is the name of the list and its count; its rows have an icon (the initials
// of the category, as the rail shows them) and, at the end, how much each adds up to.
import type { SidebarState } from '@acaprino/imago';
import { Avatar, Checkbox, Count, Icon, Text } from '@acaprino/sigrid/primitives';
import { NavItem, Sidebar, SidebarGroup, SidebarRow } from '@acaprino/sigrid/layout';
import type { Controller } from '../controller.ts';
import { formatBytes } from '../format.ts';
import { optionIntent, selectIntent } from '../intents.ts';
import type { NavItem as Nav, OptionView, SectionView } from '../types.ts';

const TICK_WORDS: Record<string, string> = { on: 'all ticked', mixed: 'some ticked', off: 'none ticked', none: 'nothing to tick' };

/** The icon of a row: the initials of its name. */
const initials = (name: string) => <Avatar name={name} size="xs" decorative />;

const size = (item: Nav) => (item.bytes === null ? undefined : <Text variant="muted">{formatBytes(item.bytes)}</Text>);

/** The menu of a list of categories: one entry per `group` row, the entry of the category the list is at is marked. */
function Categories({ c, view, state }: { c: Controller; view: SectionView; state: SidebarState }) {
  const spy = c.ui.spy[view.section] ?? view.rows.find((r) => r.kind === 'group')?.id ?? null;
  const menu = view.rows.some((r) => r.kind === 'group');
  return (
    <SidebarGroup>
      {view.nav.map((item) => {
        const group = view.rows.find((r) => r.kind === 'group' && r.id === item.id);
        const ticked = TICK_WORDS[group?.check ?? 'none'];
        return (
          <NavItem
            key={item.id}
            icon={initials(item.name)}
            current={menu ? spy === item.id : item.active}
            description={item.note ?? undefined}
            end={size(item)}
            disabled={menu && group === undefined}
            title={menu ? (group === undefined ? 'Nothing in this category matches the search' : `Go to ${item.name}: ${ticked}`) : undefined}
            onClick={() => {
              state.onOpenChange(false);
              if (menu) c.goToGroup(item.id); else void c.send(view.section, selectIntent(view.section, item.id));
            }}
          >{item.name}</NavItem>
        );
      })}
    </SidebarGroup>
  );
}

/** The kinds of issue Registry looks for: a row whose check is beside it; the whole row ticks it. */
function Kinds({ c, view }: { c: Controller; view: SectionView }) {
  const kinds = view.options.filter((o): o is Extract<OptionView, { kind: 'toggle' }> => o.kind === 'toggle');
  return (
    <SidebarGroup>
      {kinds.map((o) => {
        const toggle = (): void => { void c.send(view.section, optionIntent(view.section, o, !o.on)); };
        return (
          <NavItem
            key={o.id}
            icon={<Icon name="search" />}
            disabled={!o.enabled}
            title={o.hint ?? undefined}
            onClick={toggle}
            actions={<Checkbox aria-label={o.label} checked={o.on} disabled={!o.enabled} onCheckedChange={toggle} />}
          >{o.label}</NavItem>
        );
      })}
    </SidebarGroup>
  );
}

const head = (title: string, count: number) => <SidebarRow>{title} <Count value={count} label="entries" /></SidebarRow>;

/** The pages of Settings: a click goes to the section of the page. */
function Pages({ state }: { state: SidebarState }) {
  const go = (id: string): void => {
    state.onOpenChange(false);
    document.getElementById(id)?.scrollIntoView({ block: 'start', behavior: window.matchMedia('(prefers-reduced-motion: reduce)').matches ? 'auto' : 'smooth' });
  };
  return (
    <SidebarGroup>
      <NavItem icon={<Icon name="appearance" />} current onClick={() => go('settings-appearance')}>Appearance</NavItem>
      <NavItem icon={<Icon name="info" />} onClick={() => go('settings-about')}>About</NavItem>
    </SidebarGroup>
  );
}

export function SectionSidebar({ c, state }: { c: Controller; state: SidebarState }) {
  const view = c.view;
  if (c.ui.settings) return <Sidebar aria-label="Settings" header={head('Settings', 2)} {...state}><Pages state={state} /></Sidebar>;
  if (view === undefined) return <Sidebar aria-label="Sections" {...state} />;
  // Registry's options are the kinds of issue the scan looks for: they are its categories, rows with their check boxes.
  const kinds = view.nav.length === 0 && view.section === 'registry';
  const title = view.nav.length > 0 ? 'Categories' : kinds ? 'Look for' : 'Options';
  const count = kinds ? view.options.filter((o) => o.kind === 'toggle').length : view.nav.length;
  return (
    <Sidebar aria-label={title} header={head(title, count)} {...state}>
      {view.nav.length > 0 ? <Categories c={c} view={view} state={state} /> : null}
      {kinds ? <Kinds c={c} view={view} /> : null}
    </Sidebar>
  );
}
