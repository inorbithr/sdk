---
name: sync-spec
description: Refresh spec/ from the live public API contract, regenerate models and summarise what changed for the SDKs. Use when the platform's API changed or before adding an endpoint.
disable-model-invocation: true
---

# Sync the contract

Current source:
!`cat spec/SOURCE`

1. Run `mise run spec:sync`. It fetches `https://api.inorbit.hr/openapi.json`, keeps the
   public operations and applies the normalisation rules in `spec/README.md`.
2. If a normalisation rule no longer matches anything, the platform fixed the quirk:
   delete the rule from the tool and from the `spec/README.md` table.
3. Run `mise run spec:lint`. Breaking changes against `main` are reported by oasdiff;
   list every one in your summary. A breaking change in `/v1` is a platform bug to
   report, not something to absorb quietly.
4. Run `mise run gen`, then `mise run check`.
5. Summarise for the user: operations added, changed or removed; schema fields added;
   breaking changes; which SDK methods and conformance cases need work. Suggest
   `/add-endpoint` for each new operation.
6. Commit as `spec: sync from <api version> (<date>)` with the generated changes in the
   same commit, so `main` never holds a spec that disagrees with the models.
