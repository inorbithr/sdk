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
  store.save({ email: body.email });
  return { status: 201 };
}
