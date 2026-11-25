// Xiox on the shell: its identity, its screens, and nothing else. The frame, the palette, the notices, the look and the window are
// Imago's (docs/design-file.md of @acaprino/imago); the parts of every screen are Sigrid's.
import '@acaprino/sigrid/styles.css';
import { defineApp, startApp } from '@acaprino/imago';
import { Xiox } from './screens/xiox.tsx';

const XIOX = defineApp({
  id: 'xiox',
  name: 'Xiox',
  lang: 'en',
  // design-check-ignore-next-line color-value: the one colour an app gives, Xiox's hue (the table of the design file)
  accent: '#F2B966',
  storage: { scheme: 'xiox.theme', zoom: 'xiox.zoom', sidebar: 'xiox.sidebar' },
});

startApp(XIOX, document.getElementById('app')!, <Xiox />);
