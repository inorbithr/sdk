// The initials on an avatar with no picture.

export function initials(name: string): string {
  return name
    .trim()
    .split(" ")
    .map((w) => w[0].toUpperCase())
    .slice(0, 2)
    .join("");
}
