// Reads users from the accounts service.

export interface User {
  id: string;
  name: string;
}

// fetch has no deadline of its own: a silent service would hold the caller for minutes.
const TIMEOUT_MS = 5_000;

export async function fetchUser(base: string, id: string): Promise<User> {
  const res = await fetch(`${base}/users/${encodeURIComponent(id)}`, {
    signal: AbortSignal.timeout(TIMEOUT_MS),
  });
  if (!res.ok) {
    throw new Error(`accounts: status ${res.status}`);
  }
  return (await res.json()) as User;
}
