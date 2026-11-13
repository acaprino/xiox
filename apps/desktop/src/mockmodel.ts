// The scripted stand-in for the Rust core: serves the fixture views and (Task 2) their transitions.
// No DOM and no Vite here, so `node --experimental-strip-types --test` can load it.

import type { AboutView, AnyIntent, AppInfo, SectionId, SectionView } from './types.ts';

export interface Fixture {
  app: AppInfo;
  about: AboutView;
  views: Record<SectionId, Record<string, SectionView>>;
}

export class MockModel {
  private readonly fixture: Fixture;
  private readonly state: Record<SectionId, string>;
  private readonly revs: Record<SectionId, number>;

  constructor(fixture: Fixture) {
    this.fixture = fixture;
    const first = (s: SectionId): string => Object.keys(fixture.views[s])[0] as string;
    this.state = { cleanup: first('cleanup'), registry: first('registry'), services: first('services'), developers: first('developers') };
    this.revs = { cleanup: 1, registry: 1, services: 1, developers: 1 };
  }

  /** Opens a section: the view it was left in. */
  show(s: SectionId): SectionView {
    return this.view(s);
  }

  /** The section's current view, stamped with its current revision. */
  view(s: SectionId): SectionView {
    const view = this.fixture.views[s][this.state[s]];
    if (!view) throw new Error(`no state ${this.state[s]} for ${s}`);
    return { ...structuredClone(view), rev: this.revs[s] };
  }

  /** Task 1: the current view again with a new revision (Task 2 adds the transition table). */
  intent(s: SectionId, _intent: AnyIntent): SectionView {
    this.revs[s] += 1;
    return this.view(s);
  }

  /** What a worker would push on its own: nothing yet. */
  tick(): SectionView[] {
    return [];
  }

  /** Jumps a section to one of its fixture states (the smoke script's lever). */
  go(s: SectionId, state: string): SectionView {
    if (!this.fixture.views[s][state]) throw new Error(`no state ${state} for ${s}`);
    this.state[s] = state;
    this.revs[s] += 1;
    return this.view(s);
  }
}
