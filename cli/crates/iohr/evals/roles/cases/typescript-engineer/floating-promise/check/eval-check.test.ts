import assert from "node:assert/strict";
import { test } from "node:test";

import { signup } from "./signup.ts";

test("201 means the account is saved", async () => {
  let saved = false;
  const store = {
    save: () => new Promise<void>((done) => setTimeout(() => { saved = true; done(); }, 20)),
  };
  const reply = await signup(store, { email: "ana@example.com" });
  assert.equal(reply.status, 201);
  assert.ok(saved, "signup answered 201 before the store finished: the save is not awaited");
});

test("a failed save is not reported as created", async () => {
  const unhandled: unknown[] = [];
  const record = (e: unknown) => unhandled.push(e);
  process.on("unhandledRejection", record);
  const store = { save: () => Promise.reject(new Error("store down")) };
  let status = 0;
  try {
    status = (await signup(store, { email: "ana@example.com" })).status;
  } catch {
    status = 500;
  }
  await new Promise((r) => setImmediate(r));
  process.off("unhandledRejection", record);
  assert.notEqual(status, 201);
  assert.equal(unhandled.length, 0, "the store's rejection is unhandled");
});
