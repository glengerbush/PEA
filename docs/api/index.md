# API Overview

All endpoints are under `/api/v1` and require no authentication — the app is
single-user. Inside the desktop app the API is served in-process (over the
`pea://` scheme, no network socket). To call it over HTTP, run the standalone
engine: `pea-engine --data-dir ~/.local/share/pea --port 47200` (binds to
`127.0.0.1` only).

The machine-readable spec is [openapi.json](./openapi.json) — import it into
Bruno/Insomnia/Postman or point Swagger UI at it for a browsable reference.
Paths in the tables below are relative to the `/api/v1` base.

```bash
# examples against a running standalone engine
curl 'http://127.0.0.1:47200/api/v1/dashboard/stats'
curl 'http://127.0.0.1:47200/api/v1/archived-emails/?q=invoice&limit=5'
```

## Archived Emails

| Method | Path | Summary |
| --- | --- | --- |
| `GET` | `/archived-emails/` | List / search archived emails (FTS5 full-text search with filters) |
| `POST` | `/archived-emails/bulk/delete` | Delete a set of emails (per-email results reported) |
| `POST` | `/archived-emails/bulk/tags` | Add / remove tags on a set of emails |
| `GET` | `/archived-emails/facets` | Facet values (senders, tags) for filter dropdowns |
| `GET` | `/archived-emails/{id}` | Get a single archived email |
| `DELETE` | `/archived-emails/{id}` | Delete an archived email |
| `GET` | `/archived-emails/{id}/preview` | Sanitized HTML preview (cid images inlined, remote assets rewritten to archived copies) |
| `GET` | `/archived-emails/{id}/eml` | Download the reconstructed `.eml` (`message/rfc822`) |
| `GET` | `/archived-emails/{id}/raw` | Raw stored message bytes (`application/octet-stream`) |
| `GET` | `/archived-emails/{id}/attachments/archive` | Download all attachments as a zip |

## Attachments

| Method | Path | Summary |
| --- | --- | --- |
| `POST` | `/attachments/quicklook` | Open an attachment in the macOS Quick Look preview (desktop app only) |

## Duplicates

| Method | Path | Summary |
| --- | --- | --- |
| `GET` | `/archived-emails/duplicates/exact` | List exact-duplicate groups (message-id / content hash / attachment set / headers) |
| `POST` | `/archived-emails/duplicates/exact/approve` | Approve exact-duplicate groups: keep the keeper, delete the copies |
| `GET` | `/archived-emails/duplicates/likely` | List likely-duplicate groups (computed live) |
| `POST` | `/archived-emails/duplicates/likely/approve` | Approve likely groups: keep keepers, delete duplicates |
| `POST` | `/archived-emails/duplicates/likely/ignore` | Ignore likely groups (hide by group key) |

## Remote Content

| Method | Path | Summary |
| --- | --- | --- |
| `GET` | `/archived-emails/{id}/remote-assets` | List an email's archived remote-content assets |
| `GET` | `/archived-emails/{id}/remote-assets/{assetId}` | Serve an archived remote asset (image) from storage |
| `POST` | `/archived-emails/{id}/remote-content/archive` | Queue remote-content archiving for one email |

## Ingestion

| Method | Path | Summary |
| --- | --- | --- |
| `GET` | `/ingestion-sources` | List ingestion sources |
| `POST` | `/ingestion-sources` | Create an ingestion source |
| `PUT` | `/ingestion-sources/{id}` | Update an ingestion source |
| `DELETE` | `/ingestion-sources/{id}` | Delete an ingestion source (and its archived emails and stored files) |
| `POST` | `/ingestion-sources/{id}/pause` | Pause an ingestion source |
| `POST` | `/ingestion-sources/{id}/reimport` | Re-import |
| `POST` | `/ingestion-sources/{id}/unmerge` | Unmerge a child ingestion source |

## Contacts

| Method | Path | Summary |
| --- | --- | --- |
| `POST` | `/contacts/import` | Import contacts from CSV or vCard content |
| `GET` | `/contacts/map` | Email → display-name map used for sender name resolution |

## Dashboard

| Method | Path | Summary |
| --- | --- | --- |
| `GET` | `/dashboard/stats` | Get dashboard stats |
| `GET` | `/dashboard/ingestion-history` | Get ingestion history |
| `GET` | `/dashboard/ingestion-sources` | Get ingestion source summaries |
| `GET` | `/dashboard/indexed-insights` | Get indexed email insights |
| `GET` | `/dashboard/remote-content-issues` | Emails whose remote content failed or partially archived |

## Jobs

| Method | Path | Summary |
| --- | --- | --- |
| `GET` | `/jobs/queues` | List all queues |
| `GET` | `/jobs/queues/{queueName}` | Get jobs in a queue |

## Storage

| Method | Path | Summary |
| --- | --- | --- |
| `GET` | `/storage/download` | Download a stored file |

## Settings

| Method | Path | Summary |
| --- | --- | --- |
| `GET` | `/settings/system` | Get system settings |
| `PUT` | `/settings/system` | Update system settings |
