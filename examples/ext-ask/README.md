# inorbit/ask (prototype of `iohr ask`)

Prints an AI answer to one question in the terminal. A dev extension: it runs with
`iohr ext dev`, needs no rebuild of iohr, and gets its token from the extension token
channel (scopes `mcp:generate`, `mcp:read`; `coordinator:*` for the `say` backend).

```sh
ASK_QUESTION="why is the sky blue?" iohr ext dev examples/ext-ask            # production, your profile
ASK_QUESTION="..." iohr ext dev examples/ext-ask --env local                  # the local cluster
ASK_AGENT=site ASK_QUESTION="what is InOrbit?" iohr ext dev examples/ext-ask  # a named llm agent
iohr ext dev --test examples/ext-ask
```

Every answer is framed by an AI label (EU AI Act Art. 50). The question goes only to the
API as a message body; the extension writes nothing and logs neither question nor answer
(error lines carry an HTTP status only). `iohr ext dev` prefixes each output line with the time.

Backends (`ASK_BACKEND`): `mcp` (default; MCP tool `llm_generate`, whole answer in one
result), `generate` (REST SSE, streams; refused for the extension's scoped key today),
`say` (coordinator thread `Say` + `WatchThread`; the coordinator refuses keys today).
