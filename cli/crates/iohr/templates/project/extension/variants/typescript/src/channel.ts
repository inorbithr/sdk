// Tokens from iohr's extension channel (the SDK's ADR 0012; PRD 0084 slice 1).
//
// iohr passes IOHR_EXT_TOKEN_SOCKET (a 0600 Unix socket). The program sends
// `POST /token {"scopes": [...]}` and gets `{"access_token", "expires_at"}` for scopes its
// manifest declares, and a refusal for any other. It is the only credential this project
// has: never read IOHR_TOKEN or run `iohr auth token`.

import net from "node:net";

const MAX_ANSWER = 64 * 1024;

export class Refused extends Error {}

function exchange(path: string, scopes: string[]): Promise<Buffer> {
  return new Promise((resolve, reject) => {
    const body = JSON.stringify({ scopes });
    const chunks: Buffer[] = [];
    let size = 0;
    const conn = net.createConnection(path);
    conn.setTimeout(10_000, () => conn.destroy(new Error("timed out")));
    conn.on("connect", () =>
      conn.write(
        `POST /token HTTP/1.1\r\nhost: iohr\r\ncontent-type: application/json\r\n` +
          `content-length: ${Buffer.byteLength(body)}\r\nconnection: close\r\n\r\n${body}`,
      ),
    );
    conn.on("data", (chunk: Buffer) => {
      size += chunk.length;
      if (size > MAX_ANSWER) conn.destroy(new Error("the answer is too large"));
      else chunks.push(chunk);
    });
    conn.on("error", reject);
    conn.on("close", () => resolve(Buffer.concat(chunks)));
  });
}

/** An access token for `scopes`; throws Refused with iohr's reason. */
export async function token(scopes: string[]): Promise<string> {
  const path = (process.env.IOHR_EXT_TOKEN_SOCKET ?? "").trim();
  if (!path) throw new Refused("IOHR_EXT_TOKEN_SOCKET is not set: run this with iohr ext dev");
  let raw: Buffer;
  try {
    raw = await exchange(path, scopes);
  } catch (e) {
    throw new Refused(`cannot reach iohr's token channel: ${(e as Error).message}`);
  }
  const text = raw.toString("utf8");
  const split = text.indexOf("\r\n\r\n");
  const head = split < 0 ? text : text.slice(0, split);
  const status = head.split(" ")[1] ?? "";
  let answer: { access_token?: string; error?: string; message?: string } = {};
  try {
    answer = JSON.parse(split < 0 ? "{}" : text.slice(split + 4) || "{}");
  } catch {
    answer = {};
  }
  if (status !== "200") {
    throw new Refused(`${status || "no answer"} ${answer.error ?? ""}: ${answer.message ?? ""}`);
  }
  if (!answer.access_token) throw new Refused("iohr's answer holds no token");
  return answer.access_token;
}
