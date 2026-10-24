// A paged list. allListMonitors yields every item and follows next_page_token for you;
// the second half reads one page by hand with page_size and page_token.
// Run with INORBIT_ORG_ID and a credential with connections:read.
import { Public } from "@inorbithr/sdk";

const api = Public.fromEnv();
const org = process.env.INORBIT_ORG_ID ?? "";

// Every item, page after page. The loop ends at the last page or throws the first error.
let n = 0;
for await (const m of api.connections.allListMonitors(org, { page_size: 2 })) {
  n++;
  console.log(`${m.monitor_id} ${m.name}`);
}
console.log(`${n} monitors`);

// One page by hand: the next page token comes back with the page.
const { value: page } = await api.connections.listMonitors(org, { page_size: 1 });
console.log(
  `one page: ${page.monitors?.length ?? 0} monitor, more after "${page.next_page_token ?? ""}"`,
);
