// Reads users from the accounts service.

export interface User {
  id: string;
  name: string;
}

export async function fetchUser(base: string, id: string): Promise<User> {
  const res = await fetch(`${base}/users/${encodeURIComponent(id)}`);
  if (!res.ok) {
    throw new Error(`accounts: status ${res.status}`);
  }
  return (await res.json()) as User;
}
