// Auth with a token provider of your own: anything with a token() method, here a stand-in
// for a secret store. The SDK asks it before every attempt and calls invalidate() when
// the API refused the last token. Never log the token.
import { Client, Public, type Token, type TokenProvider } from "@inorbithr/sdk";

const vault: TokenProvider = {
  async token(): Promise<Token> {
    return { access: "custom-token-1" }; // set expiresAt when you know it
  },
};

const api = new Public(new Client({ tokenProvider: vault, baseUrl: process.env.INORBIT_BASE_URL }));
const { value: me } = await api.me();
console.log(`${me.subject} signed in with a custom provider`);
