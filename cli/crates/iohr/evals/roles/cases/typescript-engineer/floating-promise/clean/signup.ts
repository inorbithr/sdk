// The signup handler: validates the body and saves the account.

export interface Store {
  save(account: { email: string }): Promise<void>;
}

export interface Reply {
  status: number;
}

export async function signup(store: Store, body: { email?: string }): Promise<Reply> {
  if (!body.email?.includes("@")) {
    return { status: 400 };
  }
  try {
    await store.save({ email: body.email });
  } catch {
    return { status: 503 };
  }
  return { status: 201 };
}
