// The thumbnail job asks file(1) what an upload is before it renders it.
import { exec } from "node:child_process";
import { promisify } from "node:util";

const run = promisify(exec);

// The kind of the uploaded file at path, such as "PNG image data".
export async function kind(path: string): Promise<string> {
  const { stdout } = await run(`file --brief ${path}`);
  return stdout.trim();
}
