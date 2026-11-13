// Number formatting for the page. Same thresholds as `xiox_model::fmt_size` / `fmt_duration`,
// with an English decimal point.

export function formatBytes(n: number): string {
  if (n >= 2 ** 30) return `${(n / 2 ** 30).toFixed(2)} GB`;
  if (n >= 2 ** 20) return `${(n / 2 ** 20).toFixed(1)} MB`;
  if (n >= 1024) return `${(n / 1024).toFixed(0)} KB`;
  return `${n} B`;
}

export function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms} ms`;
  const s = Math.floor(ms / 1000);
  if (s < 60) return `${s} s`;
  if (s < 3600) return `${Math.floor(s / 60)} min ${String(s % 60).padStart(2, '0')} s`;
  return `${Math.floor(s / 3600)} h ${String(Math.floor((s % 3600) / 60)).padStart(2, '0')} min`;
}

export function ramText(bytes: number): string {
  return `RAM ${Math.round(bytes / 2 ** 30)} GB`;
}
