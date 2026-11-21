// Dates as the order pages show them.

export function formatDay(d: Date): string {
  return d.toISOString().slice(0, 10);
}

function legacyFormat(d: Date): string {
  return `${d.getUTCDate()}/${d.getUTCMonth() + 1}/${d.getUTCFullYear()}`;
}
