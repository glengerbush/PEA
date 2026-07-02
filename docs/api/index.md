---
aside: false
---

# API Overview

The engine serves a local HTTP API. All endpoints are prefixed with `/api/v1`
and require no authentication — the app is single-user and binds to
`127.0.0.1` only.

## API Services

- [**Archived Email Service**](./archived-email.md): Manages archived emails.
- [**Dashboard Service**](./dashboard.md): Provides data for the main dashboard.
- [**Ingestion Service**](./ingestion.md): Manages email import sources.
- [**Search Service**](./search.md): Full-text search over the archive.
- [**Storage Service**](./storage.md): Manages file storage and downloads.
- [**Upload Service**](./upload.md): Handles archive-file uploads.
- [**Jobs Service**](./jobs.md): Background job queues and their status.
- [**Users Service**](./users.md): Local user profile.
- [**Settings Service**](./settings.md): System settings, update checks, and the search-index rebuild.
