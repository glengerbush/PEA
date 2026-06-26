# Local-First Archive Roadmap

This roadmap tracks only remaining work for the local-first searchable email archive.
Speed and effective search are the top priorities: any change that can slow import,
indexing, browsing, or search must call out the risk and include a mitigation before
implementation.

## Scope

This roadmap targets new local-first installs. Until the first local-only release ships,
there is no requirement to migrate existing archives, preserve old API behavior, or keep
prior search document shapes.

## Remaining Work

### 1. End-To-End Import Smoke

Validate the current local-first flow with small but realistic import fixtures.

- Run a small `.mbox` import.
- Run a one-time IMAP-style import fixture.
- Confirm imported folder/tag structure is preserved from the source.
- Confirm each new import lands under its own local folder tree by default.
- Confirm emails can still be moved to other local folders without changing source
  provenance.
- Confirm attachment indicators, source filters, folder filters, tag filters, and sort
  controls work on imported data.
- Confirm exact and fuzzy duplicate candidates appear from imported data.
- Confirm remote-content preview stays sanitized and local-only after import.
- Capture before/after timings for import, indexing, browse, search, duplicate review, and
  remote preview.

Performance risk:

- Real import fixtures can reveal indexing or folder-tree costs that synthetic data misses.

Mitigation:

- Keep indexing/search Meili-first, keep page sizes capped, batch DB/index writes, and record
  benchmark output for any regression.

### 2. Final UI And Browser Pass

Do a hands-on browser pass over the local archive workflows.

- Verify `/dashboard/archived-emails` as the primary browse/search/filter/sort view.
- Verify `/dashboard/search` delegates cleanly to the unified archive view.
- Verify advanced field search, URL-backed filters, and sorted pagination.
- Verify bulk folder moves and tag add/remove flows.
- Verify attachment icons in email rows and thread/conversation displays.
- Verify exact duplicate review and fuzzy duplicate review flows.
- Verify remote-content preview iframe behavior, blocked asset states, and safe image serving.
- Verify local install setup copy and personal-mode UI do not expose multi-user complexity.

Performance risk:

- UI changes can make large result sets feel slow even when API latency is acceptable.

Mitigation:

- Keep rows compact, avoid per-row follow-up fetches, use server-side pagination, and verify
  behavior with a seeded local archive.

## Parallel Work Rules

- Avoid parallel PRs changing the same schema, search settings, or result DTO.
- UI polish can proceed in parallel with fixture work if it does not change shared contracts.
- Every PR that touches ingestion, indexing, search settings, archive results, duplicate
  review, or remote preview must include a before/after benchmark note.

## Performance Reporting Template

Use this in PR descriptions for any performance-sensitive change:

```text
Performance impact:
- Area touched:
- Dataset/fixture:
- Baseline command:
- Before:
- After:
- Regression risk:
- Mitigation:
```
