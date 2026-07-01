# Open Archiver(a fork)

[![Docker Compose](https://img.shields.io/badge/Docker%20Compose-2496ED?style=for-the-badge&logo=docker&logoColor=white)](https://www.docker.com)
[![PostgreSQL](https://img.shields.io/badge/PostgreSQL-4169E1?style=for-the-badge&logo=postgresql&logoColor=white)](https://www.postgresql.org/)
[![Meilisearch](https://img.shields.io/badge/Meilisearch-FF5A5F?style=for-the-badge&logo=meilisearch&logoColor=white)](https://www.meilisearch.com/)
[![TypeScript](https://img.shields.io/badge/TypeScript-3178C6?style=for-the-badge&logo=typescript&logoColor=white)](https://www.typescriptlang.org/)
[![Redis](https://img.shields.io/badge/Redis-DC382D?style=for-the-badge&logo=redis&logoColor=white)](https://redis.io)
[![SvelteKit](https://img.shields.io/badge/SvelteKit-FF3E00?style=for-the-badge&logo=svelte&logoColor=white)](https://svelte.dev/)

**A secure, sovereign, and open-source platform for email archiving.**

This fork focuses on a local only, on device archiver, in order to get emails out of bad email clients or off of email servers, into an app that can handle searching and organizing your emails, completely offline.

The changes that support that goal:

- **Runs entirely on your machine.** No accounts and no login — you're the sole owner with full access. It stays on `127.0.0.1` and starts with one command.
- **Import from files, not live mailboxes.** Bring your mail in once from `.mbox` or `.eml` files instead of connecting to email servers, keeping the original folder structure.
- **Your mailbox is the home screen.** Browse and search your whole archive in one place; the dashboard is still there, just not the center.
- **Fast search and filtering.** Search by field, tag, source, or attachment, then sort and page through results — every view is a URL you can bookmark.
- **Organize with tags.** Add or remove tags on any email and filter by them.
- **Clean up duplicates.** Exact copies are grouped for one-click removal; near-duplicates are surfaced for review.
- **Emails render correctly offline.** Remote images are saved at import time and shown in a safe preview, so archived mail looks right without going back online.
- **Easier on the eyes:** Uses [Everforest](https://github.com/sainnhe/everforest)


## Tech Stack

Open Archiver is built on a modern, scalable, and maintainable technology stack:

- **Frontend**: SvelteKit with Svelte 5
- **Backend**: Node.js with Express.js & TypeScript
- **Job Queue**: BullMQ on Redis for robust, asynchronous processing. (We use Valkey as the Redis service in the Docker Compose deployment mode, but you can use Redis as well.)
- **Search Engine**: Meilisearch for blazingly fast and resource-efficient search
- **Database**: PostgreSQL for email metadata and application state
- **Deployment**: Docker Compose deployment

## Deployment

### Prerequisites

- [Docker](https://docs.docker.com/get-docker/) and [Docker Compose](https://docs.docker.com/compose/install/). Docker must already be installed and running before `npm run local:up`; Node does not install Docker for you. Check with `docker --version` and `docker compose version`.
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

    If the launcher says Docker was not found, Docker is not installed or the `docker` command is not available in this terminal. If it says Docker Compose is not available, install Docker Compose v2 or Docker Desktop. After installing Docker Desktop, open/start it before running `npm run local:up` again; the generated `.env` will be reused.

    The containers run detached in the background, so you do not need to keep the terminal open after the command finishes. Stop them later with `npm run local:down` or `docker compose down`.

    For broader attachment text extraction, use:

    ```bash
    npm run local:up:tika
    ```

    For reference, [.env.example](.env.example) lists the variables the local Docker install can use, including optional ones you normally won't need. You normally do not need to copy it manually.

3.  **Access the application:**
    Once the services are running, you can access the Open Archiver web interface by navigating to `http://127.0.0.1:3000` in your web browser.

### Updating A Running Local Install

The app image is built from your local source, so update by rebuilding. `update-local.sh` runs the whole update end to end:

```bash
./update-local.sh
```

It backs up the database to `backups/` as a safety net, pulls the latest code (`git pull --ff-only`), rebuilds the image, recreates the app container (schema migrations run automatically on start), and waits for the app to return healthy.

Your data is safe regardless — recreating the container preserves the named Docker volumes that hold your database, search index, queue data, and archived files. To watch the app restart afterward, run:

```bash
docker compose logs -f open-archiver
```

## Importing Your Email

This fork does not connect to live mailboxes or run continuous ingestion. Instead, you import your existing mail once from static files through the web interface. Two formats are supported:

- **[Mbox import](docs/user-guides/email-providers/mbox.md)** — a single `.mbox` file, or a folder of them (nested directories are scanned recursively).
- **[EML import](docs/user-guides/email-providers/eml.md)** — a zip archive of `.eml` files; the folder structure inside the zip is preserved.
