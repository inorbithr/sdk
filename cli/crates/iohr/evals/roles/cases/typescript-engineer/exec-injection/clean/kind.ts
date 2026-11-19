// The thumbnail job asks file(1) what an upload is before it renders it.
import { execFile } from "node:child_process";
import { promisify } from "node:util";

const run = promisify(execFile);

// The kind of the uploaded file at path, such as "PNG image data". The path goes to file(1)
// as one argument, never through a shell: upload names are chosen by users.
export async function kind(path: string): Promise<string> {
  const { stdout } = await run("file", ["--brief", "--", path]);
  return stdout.trim();
}
