// The notes of a section: what Rust says about the view (`notices`, with their extra lines behind a disclosure), and the warning that
// the cleaned profile is not the signed-in one.
import { Disclosure, Note } from '@acaprino/sigrid/components';
import { Text, TextList } from '@acaprino/sigrid/primitives';
import type { NoticeView } from '../types.ts';

type Tone = 'info' | 'success' | 'warning' | 'error';

/** Rust's tones are the page's four; `ok` is Sigrid's `success`. */
export const noteTone = (tone: NoticeView['tone']): Tone => (tone === 'ok' ? 'success' : tone === 'warn' ? 'warning' : tone);

/** A notice, in the panel it is about: Sigrid's note. The extra lines (the backup path of a Registry fix, say) are behind a disclosure. */
export function Notice({ notice }: { notice: NoticeView }) {
  const extra = notice.more.length;
  return (
    <Note tone={noteTone(notice.tone)} title={notice.text}>
      {extra === 0 ? null : (
        <Disclosure label={`Show ${extra} more ${extra === 1 ? 'line' : 'lines'}`}>
          <TextList items={notice.more.map((line) => <Text mono key={line}>{line}</Text>)} />
        </Disclosure>
      )}
    </Note>
  );
}

/** Whose profile is cleaned, and whose session it is: both halves matter, so the note says it whole. */
export function ProfileNote({ text }: { text: string }) {
  return <Note tone="warning" title="Another profile">{text}</Note>;
}
