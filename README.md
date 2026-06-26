# Open Archiver(a fork)

[![Docker Compose](https://img.shields.io/badge/Docker%20Compose-2496ED?style=for-the-badge&logo=docker&logoColor=white)](https://www.docker.com)
[![PostgreSQL](https://img.shields.io/badge/PostgreSQL-4169E1?style=for-the-badge&logo=postgresql&logoColor=white)](https://www.postgresql.org/)
[![Meilisearch](https://img.shields.io/badge/Meilisearch-FF5A5F?style=for-the-badge&logo=meilisearch&logoColor=white)](https://www.meilisearch.com/)
[![TypeScript](https://img.shields.io/badge/TypeScript-3178C6?style=for-the-badge&logo=typescript&logoColor=white)](https://www.typescriptlang.org/)
[![Redis](https://img.shields.io/badge/Redis-DC382D?style=for-the-badge&logo=redis&logoColor=white)](https://redis.io)
[![SvelteKit](https://img.shields.io/badge/SvelteKit-FF3E00?style=for-the-badge&logo=svelte&logoColor=white)](https://svelte.dev/)

**A secure, sovereign, and open-source platform for email archiving.**

This fork focuses on a local only, on device archiver, in order to get emails out of bad email clients or off of email servers, into an app that can handle searching and organizing your emails, completely offline.

The changes towards that goal:

- **Local-first focus:** setup is oriented around one-device use, local Docker services, generated secrets, loopback defaults, optional heavy services, and personal mode.
- **Unified archive/search view:** browsing and searching are merged around /dashboard/archived-emails, with advanced filters, field-specific search, sorting, pagination, tags, folders, source filters, attachment filters, and URL-backed controls.
- **Tags and folders:** added backend services and UI flows for creating folders, moving emails in bulk, and adding/removing tags with batched search-index updates.
- **Duplicate review:** added exact duplicate grouping/approval and fuzzy duplicate review, with fuzzy scans handled through background jobs and a new /dashboard/duplicates UI.
- **Remote content archiving:** added remote asset capture, sanitized preview rendering, remote-content worker/queue, safe image-only storage, and strong protections against private IPs, DNS rebinding, redirects, unsafe ports, oversized files, and unsafe content types.
- **IAM/multi-user complexity removed:** IAM routes, services, policies, permission middleware, role/user admin screens, and IAM docs were removed or hidden. The app now treats the local authenticated owner as having full local access.
- **Import provenance and folders:** imported emails now preserve source folder/label paths while also getting a mutable local folder path. New imports land under their own import tree, but messages can be moved later.
- **Attachment indicators:** email/search/thread rows use indexed hasAttachments metadata instead of per-row attachment lookups.
- **Performance tooling:** added scripts/perf-baseline.mjs and packages/backend/scripts/seed-perf-data.mjs, plus a synthetic benchmark fixture. Current synthetic baseline was clean on 2,500 messages.
- **Docs/install changes:** README, install docs, API docs, .env.example, Docker Compose, and local setup scripts were updated for the local-first direction.

## Tech Stack

Open Archiver is built on a modern, scalable, and maintainable technology stack:

- **Frontend**: SvelteKit with Svelte 5
- **Backend**: Node.js with Express.js & TypeScript
- **Job Queue**: BullMQ on Redis for robust, asynchronous processing. (We use Valkey as the Redis service in the Docker Compose deployment mode, but you can use Redis as well.)
- **Search Engine**: Meilisearch for blazingly fast and resource-efficient search
- **Database**: PostgreSQL for metadata, user management, and audit logs
- **Deployment**: Docker Compose deployment

## Deployment

### Prerequisites

- [Docker](https://docs.docker.com/get-docker/) and [Docker Compose](https://docs.docker.com/compose/install/)
- A server or local machine with at least 4GB of RAM (2GB of RAM if you use external Postgres, Redis (Valkey) and Meilisearch instances).
- Optional: Node.js 22 or newer for the `npm run local:up` one-liner. If Node is not installed, use the Docker-only one-liner below.

### Installation

1.  **Clone the repository:**

    ```bash
    git clone https://github.com/glengerbush/OpenArchiver.git
    cd OpenArchiver
    ```

2.  **Start the local app:**
    This generates `.env` automatically if it is missing, then starts the local Docker stack.
    Docker is the runtime; Node is only used as a small convenience wrapper for this command and does not install or run the app itself.

    ```bash
    npm run local:up
    ```

    If you do not have Node.js installed locally, use this Docker-only one-liner:

    ```bash
    docker run --rm -u "$(id -u):$(id -g)" -v "$PWD":/work -w /work node:22-alpine node scripts/setup-local-env.mjs --if-missing && docker compose up -d
    ```

    The generated `.env` exists so local service passwords, JWT signing, and encryption keys stay stable across restarts. It is not meant to defend against someone who already controls your laptop. The stack still binds the web UI to your laptop only (`127.0.0.1`), enables personal mode, and keeps heavier optional services like Apache Tika disabled by default.

    The containers run detached in the background, so you do not need to keep the terminal open after the command finishes. Stop them later with `npm run local:down` or `docker compose down`.

    For broader attachment text extraction, use:

    ```bash
    npm run local:up:tika
    ```

    For reference, [.env.example](.env.example) shows every variable used by the local Docker install. You normally do not need to copy it manually.

3.  **Access the application:**
    Once the services are running, you can access the Open Archiver web interface by navigating to `http://127.0.0.1:3000` in your web browser.

## Data Source Configuration

After deploying the application, you will need to configure one or more ingestion sources to begin archiving emails. Or you can import .mbox, .eml, or .pst. Follow detailed guides to connect to your email provider:

- [Connecting to Google Workspace](https://docs.openarchiver.com/user-guides/email-providers/google-workspace.html)
- [Connecting to Microsoft 365](https://docs.openarchiver.com/user-guides/email-providers/imap.html)
- [Connecting to a Generic IMAP Server](https://docs.openarchiver.com/user-guides/email-providers/imap.html)
