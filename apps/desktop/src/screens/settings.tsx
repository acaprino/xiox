// Settings, the fourth view: Appearance and About Xiox (the version, the Winapp2 rules and where they come from, the attribution Winapp2
// asks for). A setting is not a decision about the window, so it is not a dialog. The theme is the palette's (light, dark and the
// system's, the same three commands in every app), so there is no switch for it here; glass is the person's choice, in a window of its
// own only, and it applies at once.
import { useGlass, useWindow } from '@acaprino/imago';
import { useCommandPalette } from '@acaprino/sigrid/assemblies';
import { SettingRow } from '@acaprino/sigrid/components';
import { Panel } from '@acaprino/sigrid/layout';
import { Button, Column, Icon, Section, Switch, Text } from '@acaprino/sigrid/primitives';
import { DescriptionList } from '@acaprino/sigrid/components';
import type { AboutView } from '../types.ts';

export function Settings({ about }: { about: AboutView }) {
  const [glass, setGlass] = useGlass();
  const own = useWindow().own;
  const palette = useCommandPalette();
  return (
    <Panel title="Settings" subtitle="Appearance and About">
      <Column gap="xl" pad="lg" width="prose">
        <Section id="settings-appearance" title="Appearance" icon={<Icon name="appearance" />}>
          <SettingRow
            label="Theme"
            description="Light or dark with the switch of the title bar; the system's look, with light and dark, is in the palette of every app."
            control={<Button variant="outline" size="sm" onClick={() => palette.setOpen(true)}>Open the commands</Button>}
          />
          {/* A browser has no glass: the setting is in a window of its own. */}
          {own ? (
            <SettingRow
              label="Glass window"
              description="Windows' acrylic behind the page: a blurred, tinted copy of the desktop. It changes at once; the same choice is the command Glass of the palette."
              control={<Switch aria-label="Glass window" checked={glass} onCheckedChange={setGlass} />}
            />
          ) : null}
        </Section>
        <Section id="settings-about" title="About Xiox" icon={<Icon name="info" />}>
          <DescriptionList aria-label="About Xiox" items={[{ name: 'Version', value: about.version }, { name: 'Rules', value: about.winapp2 }]} />
          <Text as="p" variant="muted">{about.attribution}</Text>
        </Section>
      </Column>
    </Panel>
  );
}
