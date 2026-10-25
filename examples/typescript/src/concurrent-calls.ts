// Concurrent use: one client shared by several calls in flight. Callers that need a token
// at the same moment share one exchange.
import { Public } from "@inorbithr/sdk";

const api = Public.fromEnv();
const answers = await Promise.all([api.me(), api.me(), api.me()]);
console.log(`${answers.length} calls at once: ${answers.map((a) => a.value.subject).join(", ")}`);
