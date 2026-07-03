# API Overview

All endpoints are prefixed with `/api/v1` (also served at `/v1`) and require
no authentication — the app is single-user. Inside the desktop app the API is
served in-process (over the `pea://` scheme, no network socket). To call it
over HTTP, run the standalone engine:
`pea-engine --data-dir ~/.local/share/pea --port 47200` (binds to
`127.0.0.1` only).

The machine-readable spec is [openapi.json](./openapi.json) — import it into
Bruno/Insomnia/Postman or point Swagger UI at it for a browsable reference.

```bash
# examples against a running standalone engine
curl 'http://127.0.0.1:47200/api/v1/dashboard/stats'
curl 'http://127.0.0.1:47200/api/v1/archived-emails/?q=invoice&limit=5'
```

## Archived Emails

| Method | Path | Summary |
| --- | --- | --- |
| `GET` | `/v1/archived-emails/` | List / search archived emails (FTS5 full-text search with filters) |
| `POST` | `/v1/archived-emails/bulk/delete` | Delete a set of emails (per-email results reported) |
| `POST` | `/v1/archived-emails/bulk/tags` | Add / remove tags on a set of emails |
| `GET` | `/v1/archived-emails/facets` | Facet values (senders, tags) for filter dropdowns |
| `GET` | `/v1/archived-emails/ingestion-source/{ingestionSourceId}` | List archived emails for an ingestion source |
| `DELETE` | `/v1/archived-emails/{id}` | Delete an archived email |
| `GET` | `/v1/archived-emails/{id}` | Get a single archived email |
| `GET` | `/v1/archived-emails/{id}/preview` | Sanitized HTML preview of an email (cid images inlined, remote assets rewritten to archived copies) |

## Search

| Method | Path | Summary |
| --- | --- | --- |
| `GET` | `/v1/search` | Search archived emails |

## Duplicates

| Method | Path | Summary |
| --- | --- | --- |
| `GET` | `/v1/archived-emails/duplicates/exact` | List exact-duplicate groups (message-id / content hash / attachment set / headers) |
| `POST` | `/v1/archived-emails/duplicates/exact/approve` | Approve exact-duplicate groups: keep the keeper, delete the copies |
| `GET` | `/v1/archived-emails/duplicates/fuzzy` | List pending fuzzy-duplicate groups |
| `POST` | `/v1/archived-emails/duplicates/fuzzy/approve` | Approve fuzzy groups: keep keepers, delete duplicates |
| `POST` | `/v1/archived-emails/duplicates/fuzzy/ignore` | Mark fuzzy groups as not-duplicates |
| `POST` | `/v1/archived-emails/duplicates/fuzzy/scan` | Queue a fuzzy-duplicate scan batch |

## Remote Content

| Method | Path | Summary |
| --- | --- | --- |
| `GET` | `/v1/archived-emails/{id}/remote-assets` | List an email's archived remote-content assets |
| `GET` | `/v1/archived-emails/{id}/remote-assets/{assetId}` | Serve an archived remote asset (image) from storage |
| `POST` | `/v1/archived-emails/{id}/remote-content/archive` | Queue remote-content archiving for one email |

## Ingestion

| Method | Path | Summary |
| --- | --- | --- |
| `GET` | `/v1/ingestion-sources` | List ingestion sources |
| `POST` | `/v1/ingestion-sources` | Create an ingestion source |
| `DELETE` | `/v1/ingestion-sources/{id}` | Delete an ingestion source |
| `GET` | `/v1/ingestion-sources/{id}` | Get an ingestion source |
| `PUT` | `/v1/ingestion-sources/{id}` | Update an ingestion source |
| `POST` | `/v1/ingestion-sources/{id}/import` | Trigger initial import |
| `POST` | `/v1/ingestion-sources/{id}/pause` | Pause an ingestion source |
| `POST` | `/v1/ingestion-sources/{id}/sync` | Force sync |
| `POST` | `/v1/ingestion-sources/{id}/unmerge` | Unmerge a child ingestion source |

## Upload

| Method | Path | Summary |
| --- | --- | --- |
| `POST` | `/v1/upload` | Upload a file |

## Contacts

| Method | Path | Summary |
| --- | --- | --- |
| `POST` | `/v1/contacts/import` | Import contacts from CSV or vCard content |
| `GET` | `/v1/contacts/map` | Email → display-name map used for sender name resolution |

## Dashboard

| Method | Path | Summary |
| --- | --- | --- |
| `GET` | `/v1/dashboard/indexed-insights` | Get indexed email insights |
| `GET` | `/v1/dashboard/ingestion-history` | Get ingestion history |
| `GET` | `/v1/dashboard/ingestion-sources` | Get ingestion source summaries |
| `GET` | `/v1/dashboard/recent-syncs` | Get recent sync activity |
| `GET` | `/v1/dashboard/remote-content-issues` | Emails whose remote content failed or partially archived |
| `GET` | `/v1/dashboard/stats` | Get dashboard stats |

## Jobs

| Method | Path | Summary |
| --- | --- | --- |
| `GET` | `/v1/jobs/queues` | List all queues |
| `GET` | `/v1/jobs/queues/{queueName}` | Get jobs in a queue |

## Storage

| Method | Path | Summary |
| --- | --- | --- |
| `GET` | `/v1/storage/download` | Download a stored file |

## Users

| Method | Path | Summary |
| --- | --- | --- |
| `GET` | `/v1/users/profile` | Get current user profile |
| `PATCH` | `/v1/users/profile` | Update current user profile |

## Settings

| Method | Path | Summary |
| --- | --- | --- |
| `POST` | `/v1/settings/search/rebuild` | Wipe and rebuild the full-text index from the archive |
| `GET` | `/v1/settings/system` | Get system settings |
| `PUT` | `/v1/settings/system` | Update system settings |
| `GET` | `/v1/settings/updates/check` | Legacy commit-based update check against the GitHub repo |

## System

| Method | Path | Summary |
| --- | --- | --- |
| `GET` | `/healthz` | Liveness check |

