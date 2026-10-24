// Typed error handling. A refused call throws ApiError with the HTTP status, a stable
// code to branch on, the problem text and the request id to quote to support.
import { ApiError, Public } from "@inorbithr/sdk";

const api = Public.fromEnv();
try {
  await api.me();
} catch (e) {
  if (e instanceof ApiError && e.code === "forbidden") {
    console.log(`refused: status ${e.status}, code ${e.code}, request ${e.raw.requestId}`);
  } else {
    throw e;
  }
}
