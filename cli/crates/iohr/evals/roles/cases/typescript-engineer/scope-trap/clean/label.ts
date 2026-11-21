// The label on a shipping box.

export function label(name: string, city: string): string {
  return `${name}, ${city.toUpperCase()}`;
}
