// `node src/main.ts`: grade the stub solver on the dataset and check the pass bar (see suite.ts).

import { main } from "./suite.ts";

process.exitCode = await main(process.argv.slice(2));
