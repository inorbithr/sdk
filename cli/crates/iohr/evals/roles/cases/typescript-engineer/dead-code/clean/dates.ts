// Dates as the order pages show them.

export function formatDay(d: Date): string {
  return d.toISOString().slice(0, 10);
}

// The day and the UTC time to the minute, such as 2026-11-19 14:05.
export function formatMinute(d: Date): string {
  return d.toISOString().slice(0, 16).replace("T", " ");
}
