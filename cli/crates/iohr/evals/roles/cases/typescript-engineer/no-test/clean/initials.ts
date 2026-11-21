// The initials on an avatar with no picture.

export function initials(name: string): string {
  return name
    .trim()
    .split(/\s+/)
    .filter((w) => w.length > 0)
    .map((w) => w[0].toUpperCase())
    .slice(0, 2)
    .join("");
}
