// The view contract (Rust -> TypeScript). `crates/xiox-core/src/view.rs` mirrors it field for field.
// The page renders whatever SectionView it receives and sends intents; it decides nothing.

export type SectionId = 'cleanup' | 'registry' | 'developers';
export type Tone = 'info' | 'ok' | 'warn' | 'error';
export type LevelView = 'recommended' | 'optional' | 'caution';
export type Check = 'on' | 'off' | 'mixed' | 'none';
/** `group` is a category header of a list that shows its categories one after another: it carries the category's own check box, count and size. `header` is a plain sub-heading. */
export type RowKind = 'item' | 'header' | 'child' | 'group';
export type ColumnKind = 'name' | 'cell' | 'level' | 'size';

export interface AppInfo { version: string; admin: boolean; ramTotalBytes: number; profileMismatch: string | null; fake: boolean }
export interface AboutView { version: string; winapp2: string; attribution: string }
export interface JobView { label: string; fraction: number; detail: string | null }
export interface StatusView { badge: string; text: string }
export interface NavItem { id: string; name: string; note: string | null; bytes: number | null; active: boolean }
export interface ChoiceItem { id: string; label: string }
export type OptionView =
  | { kind: 'toggle'; id: string; label: string; on: boolean; enabled: boolean; hint: string | null }
  | { kind: 'number'; id: string; label: string; value: number; min: number; max: number; enabled: boolean }
  | { kind: 'choice'; id: string; label: string; value: string; choices: ChoiceItem[]; enabled: boolean }
  | { kind: 'list'; id: string; label: string; items: string[]; selected: number | null; enabled: boolean; addLabel: string; removeLabel: string; empty: string }
  | { kind: 'button'; id: string; label: string; enabled: boolean; hint: string | null }
  | { kind: 'text'; text: string; tone: Tone; hint: string | null };
export interface ColumnView { title: string; kind: ColumnKind }
export interface RowView {
  id: string; kind: RowKind; parent: string | null; check: Check; enabled: boolean;
  name: string; sub: string | null; level: LevelView | null; cells: string[];
  bytes: number | null; sizeText: string | null; muted: boolean; why: string | null;
  detail: [string, string][]; lines: string[];
}
export interface NoticeView { tone: Tone; text: string; more: string[] }
export interface ActionView { id: string; label: string; key: string | null; enabled: boolean; primary: boolean }
export interface ButtonView { id: string; label: string; enabled: boolean; primary: boolean }
export interface DialogView {
  id: string; title: string; text: string[]; summary: string[]; permanent: boolean; rows: [string, string][];
  warningsTitle: string | null; warnings: string[]; cautionTitle: string | null; cautions: [string, string][];
  ack: boolean | null; error: string | null; buttons: ButtonView[];
}
export interface ReportLine { item: string; why: string; path: string }
export interface ReportItem { name: string; freedBytes: number; removed: number; left: number; note: string | null; lines: string[] }
export interface SavedView { text: string; ok: boolean }
export interface ReportView {
  title: string; freedBytes: number; recycledBytes: number; files: number; dirs: number; elapsedMs: number;
  cancelled: boolean; notes: NoticeView[]; byCategory: [string, number][]; skipped: ReportLine[]; failed: ReportLine[];
  items: ReportItem[]; canSave: boolean; saved: SavedView | null;
}
export interface EmptyView { title: string; text: string }
export interface SearchView { placeholder: string; value: string }
export interface SectionView {
  section: SectionId; rev: number; job: JobView | null; status: StatusView;
  nav: NavItem[]; options: OptionView[]; title: string; subtitle: string;
  columns: ColumnView[]; rows: RowView[]; bulk: boolean; empty: EmptyView | null;
  notices: NoticeView[]; actions: ActionView[]; search: SearchView | null;
  dialog: DialogView | null; report: ReportView | null; footer: string | null;
}
export interface CmdError { kind: 'refused' | 'failed'; message: string }

export type CleanupIntent =
  | { kind: 'toggle'; row: string } | { kind: 'selectAll'; on: boolean } | { kind: 'setOnlyOld'; on: boolean }
  | { kind: 'search'; text: string } | { kind: 'analyze' } | { kind: 'openConfirm' } | { kind: 'confirm' }
  | { kind: 'closeDialog' } | { kind: 'cancel' } | { kind: 'openKeepList' };
export type RegistryIntent =
  | { kind: 'toggleCategory'; index: number } | { kind: 'toggle'; row: string } | { kind: 'selectAll'; on: boolean }
  | { kind: 'scan' } | { kind: 'openFix' } | { kind: 'fix'; backup: boolean } | { kind: 'closeDialog' };
export type DevIntent =
  | { kind: 'select'; category: 'projects' | 'worktrees' | 'sessions' } | { kind: 'toggle'; row: string } | { kind: 'selectAll'; on: boolean }
  | { kind: 'setInactiveDays'; days: number } | { kind: 'setRemoveMode'; mode: 'recycle' | 'permanent' } | { kind: 'setScanRoots'; on: boolean }
  | { kind: 'selectRoot'; index: number | null } | { kind: 'removeRoot' } | { kind: 'analyze' } | { kind: 'openConfirm' }
  | { kind: 'setAck'; on: boolean } | { kind: 'confirm' } | { kind: 'permanent' } | { kind: 'closeDialog' } | { kind: 'cancel' };
export type AnyIntent = CleanupIntent | RegistryIntent | DevIntent;
