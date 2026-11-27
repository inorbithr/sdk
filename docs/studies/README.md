# Studies

Research that ends in a recommendation and builds nothing yet. Each study is
`NNNN-title.md`, starts with front matter (`title`, `status`, `date`, `summary`), and links the
ADR or pull request it leads to. Style follows [docs/style.md](../style.md).

| Study | Status | Summary |
|---|---|---|
| [0001 Roles upgrade plan](0001-roles-upgrade-plan.md) | proposed | One working method, deeper per-language roles, new sql, infra, compliance and scientist roles, and a role eval harness built first |
| [0002 Release concurrency](0002-release-concurrency.md) | accepted | One concurrency group per release job and tag instead of one per workflow run, so a held review blocks only its own release |
