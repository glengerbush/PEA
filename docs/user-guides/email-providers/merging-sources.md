# Merging Imports

Merged import groups let you combine multiple imports so their emails appear unified in browsing, search, and thread views. This is useful when a single archive was exported as several files — for example one `.mbox` per folder, or several EML zips — and you want them to show up as one mailbox.

> This fork only supports one-time file imports (Mbox and EML). Merging live connections, provider migration, and compliance-driven retention are not part of this fork.

## Concepts

| Term             | Definition                                                                                                                              |
| ---------------- | -------------------------------------------------------------------------------------------------------------------------------------- |
| **Root source**  | An import where no merge parent is set. Shown as the primary row in the Imports table. All emails in the group are owned by it. |
| **Child source** | An import merged into a root. Its emails are stored under the root source, not under the child.                                          |
| **Group**        | A root source and all its children. Every email in the group is stored under and owned by the root.                                     |

The hierarchy is **flat** — only one level of nesting is supported. If you merge into a child, the relationship is redirected to the root.

## Root Ownership — How Storage and Data Work

> **Every email from a child import is written to the root source's storage folder and assigned the root source's ID in the database.**

In practical terms:

- The storage path for every email belongs to the root: `openarchiver/{root-name}-{root-id}/emails/...`
- Every `archived_emails` row created by a child import has `ingestionSourceId` set to the **root's ID**, not the child's.
- Attachments are stored under the root's folder and scoped to the root's ID.

Browsing the root source therefore shows every email in the group without any extra configuration.

## How to Merge a New Import Into an Existing One

Merging can only be configured **at creation time**.

1. Go to the **Imports** page.
2. Click **Import Archive** to open the import form.
3. Fill in the import details as usual.
4. Expand the **Advanced Options** section at the bottom of the form. This is only visible when at least one import already exists.
5. Check **Merge into existing import** and select the target root source.
6. Click **Submit**.

The new import runs normally. Once complete, its emails appear alongside the root's — all stored under the root.

## How Emails Appear When Merged

Because every email in the group is physically owned by the root, browsing the root source shows all of them — there is nothing to aggregate. The same applies to search (filtering by the root's source ID returns the whole group) and to threads (a reply imported from a different file still lands in the correct thread).

## Deduplication Across the Group

Duplicate detection covers the **entire merge group**. If the same email (matched by its RFC `Message-ID` header) already exists anywhere in the group, it is skipped and not stored again.

## Editing Sources in a Group

Each source in a group can be edited independently. Expand the group row in the Imports table by clicking the chevron, then use the **⋮** actions menu on the specific source you want to edit.

## Unmerging a Child Source

To detach a child from its group and make it standalone:

1. Expand the group row by clicking the chevron next to the root source name.
2. Open the **⋮** actions menu on the child source.
3. Click **Unmerge**.

The child becomes an independent root source. No email data is moved or deleted.

> **Note:** Because all emails from the child were stored under the root source's ID, unmerging does not transfer them. They remain owned by the root.

## Deleting Sources in a Group

- **Deleting a root source** also deletes all its children and every email, attachment, storage file, and search index entry owned by the root. Because all group emails are stored under the root, this removes the entire group's archive.
- **Deleting a child source** removes only the child's configuration. Emails it imported are stored under the root and are **not** deleted.

A warning is shown in the delete confirmation dialog when a root source has children.

## Known Limitations

- **Merging existing standalone imports is not supported.** You can only merge into a group at creation time. To merge two existing imports, delete one and recreate it with the merge target selected.
- **Historical data from a child remains with the root after unmerging.** Emails a child imported while merged stay owned by the root and are not migrated back.
