CREATE TABLE `archived_emails` (
	`id` text PRIMARY KEY NOT NULL,
	`thread_id` text,
	`ingestion_source_id` text NOT NULL,
	`user_email` text NOT NULL,
	`message_id_header` text,
	`provider_message_id` text,
	`sent_at` integer NOT NULL,
	`subject` text,
	`sender_name` text,
	`sender_email` text NOT NULL,
	`recipients` text,
	`storage_path` text NOT NULL,
	`storage_hash_sha256` text NOT NULL,
	`size_bytes` integer NOT NULL,
	`is_indexed` integer DEFAULT false NOT NULL,
	`has_attachments` integer DEFAULT false NOT NULL,
	`archived_at` integer DEFAULT (unixepoch() * 1000) NOT NULL,
	`source_path` text,
	`source_labels` text,
	`duplicate_subject_hash` text,
	`duplicate_fuzzy_group_key` text,
	`duplicate_body_hash` text,
	`duplicate_recipient_fingerprint` text,
	`duplicate_attachment_fingerprint` text,
	`remote_content_status` text DEFAULT 'not_started' NOT NULL,
	`remote_content_asset_count` integer DEFAULT 0 NOT NULL,
	`remote_content_archived_at` integer,
	`path` text,
	`tags` text,
	FOREIGN KEY (`ingestion_source_id`) REFERENCES `ingestion_sources`(`id`) ON UPDATE no action ON DELETE cascade
);
--> statement-breakpoint
CREATE INDEX `thread_id_idx` ON `archived_emails` (`thread_id`);--> statement-breakpoint
CREATE INDEX `archived_emails_message_id_header_idx` ON `archived_emails` (`message_id_header`);--> statement-breakpoint
CREATE INDEX `archived_emails_storage_hash_idx` ON `archived_emails` (`storage_hash_sha256`);--> statement-breakpoint
CREATE INDEX `provider_msg_source_idx` ON `archived_emails` (`provider_message_id`,`ingestion_source_id`);--> statement-breakpoint
CREATE INDEX `archived_emails_source_path_idx` ON `archived_emails` (`source_path`);--> statement-breakpoint
CREATE INDEX `archived_emails_fuzzy_subject_sender_idx` ON `archived_emails` (`duplicate_subject_hash`,`sender_email`);--> statement-breakpoint
CREATE INDEX `archived_emails_fuzzy_group_key_idx` ON `archived_emails` (`duplicate_fuzzy_group_key`);--> statement-breakpoint
CREATE INDEX `archived_emails_fuzzy_body_idx` ON `archived_emails` (`duplicate_body_hash`);--> statement-breakpoint
CREATE INDEX `archived_emails_fuzzy_recipients_idx` ON `archived_emails` (`duplicate_recipient_fingerprint`);--> statement-breakpoint
CREATE INDEX `archived_emails_fuzzy_attachments_idx` ON `archived_emails` (`duplicate_attachment_fingerprint`);--> statement-breakpoint
CREATE INDEX `archived_emails_remote_content_status_idx` ON `archived_emails` (`remote_content_status`);--> statement-breakpoint
CREATE INDEX `archived_emails_sent_at_idx` ON `archived_emails` (`sent_at`);--> statement-breakpoint
CREATE TABLE `attachments` (
	`id` text PRIMARY KEY NOT NULL,
	`filename` text NOT NULL,
	`mime_type` text,
	`size_bytes` integer NOT NULL,
	`content_hash_sha256` text NOT NULL,
	`storage_path` text NOT NULL,
	`ingestion_source_id` text,
	FOREIGN KEY (`ingestion_source_id`) REFERENCES `ingestion_sources`(`id`) ON UPDATE no action ON DELETE cascade
);
--> statement-breakpoint
CREATE INDEX `source_hash_idx` ON `attachments` (`ingestion_source_id`,`content_hash_sha256`);--> statement-breakpoint
CREATE TABLE `email_attachments` (
	`id` text PRIMARY KEY NOT NULL,
	`email_id` text NOT NULL,
	`attachment_id` text NOT NULL,
	FOREIGN KEY (`email_id`) REFERENCES `archived_emails`(`id`) ON UPDATE no action ON DELETE cascade,
	FOREIGN KEY (`attachment_id`) REFERENCES `attachments`(`id`) ON UPDATE no action ON DELETE restrict
);
--> statement-breakpoint
CREATE INDEX `email_attachments_email_idx` ON `email_attachments` (`email_id`);--> statement-breakpoint
CREATE INDEX `email_attachments_attachment_idx` ON `email_attachments` (`attachment_id`);--> statement-breakpoint
CREATE TABLE `fuzzy_duplicate_group_emails` (
	`group_id` text NOT NULL,
	`email_id` text NOT NULL,
	`suggested_keeper` integer DEFAULT false NOT NULL,
	PRIMARY KEY(`group_id`, `email_id`),
	FOREIGN KEY (`group_id`) REFERENCES `fuzzy_duplicate_groups`(`id`) ON UPDATE no action ON DELETE cascade,
	FOREIGN KEY (`email_id`) REFERENCES `archived_emails`(`id`) ON UPDATE no action ON DELETE cascade
);
--> statement-breakpoint
CREATE INDEX `fuzzy_duplicate_group_emails_email_idx` ON `fuzzy_duplicate_group_emails` (`email_id`);--> statement-breakpoint
CREATE TABLE `fuzzy_duplicate_groups` (
	`id` text PRIMARY KEY NOT NULL,
	`group_key` text NOT NULL,
	`status` text DEFAULT 'pending' NOT NULL,
	`score` integer NOT NULL,
	`signals` text,
	`created_at` integer DEFAULT (unixepoch() * 1000) NOT NULL,
	`updated_at` integer DEFAULT (unixepoch() * 1000) NOT NULL
);
--> statement-breakpoint
CREATE UNIQUE INDEX `fuzzy_duplicate_groups_group_key_unique` ON `fuzzy_duplicate_groups` (`group_key`);--> statement-breakpoint
CREATE INDEX `fuzzy_duplicate_groups_status_idx` ON `fuzzy_duplicate_groups` (`status`);--> statement-breakpoint
CREATE INDEX `fuzzy_duplicate_groups_score_idx` ON `fuzzy_duplicate_groups` (`score`);--> statement-breakpoint
CREATE TABLE `remote_content_assets` (
	`id` text PRIMARY KEY NOT NULL,
	`email_id` text NOT NULL,
	`original_url` text NOT NULL,
	`final_url` text,
	`url_hash` text NOT NULL,
	`status` text DEFAULT 'pending' NOT NULL,
	`content_type` text,
	`size_bytes` integer,
	`content_hash_sha256` text,
	`storage_path` text,
	`failure_reason` text,
	`created_at` integer DEFAULT (unixepoch() * 1000) NOT NULL,
	`updated_at` integer DEFAULT (unixepoch() * 1000) NOT NULL,
	FOREIGN KEY (`email_id`) REFERENCES `archived_emails`(`id`) ON UPDATE no action ON DELETE cascade
);
--> statement-breakpoint
CREATE INDEX `remote_content_assets_email_idx` ON `remote_content_assets` (`email_id`);--> statement-breakpoint
CREATE INDEX `remote_content_assets_status_idx` ON `remote_content_assets` (`status`);--> statement-breakpoint
CREATE UNIQUE INDEX `remote_content_assets_email_url_hash_unique` ON `remote_content_assets` (`email_id`,`url_hash`);--> statement-breakpoint
CREATE TABLE `ingestion_sources` (
	`id` text PRIMARY KEY NOT NULL,
	`user_id` text,
	`name` text NOT NULL,
	`provider` text NOT NULL,
	`credentials` text,
	`status` text DEFAULT 'pending_auth' NOT NULL,
	`last_sync_started_at` integer,
	`last_sync_finished_at` integer,
	`last_sync_status_message` text,
	`sync_state` text,
	`merged_into_id` text,
	`created_at` integer DEFAULT (unixepoch() * 1000) NOT NULL,
	`updated_at` integer DEFAULT (unixepoch() * 1000) NOT NULL,
	FOREIGN KEY (`user_id`) REFERENCES `users`(`id`) ON UPDATE no action ON DELETE cascade,
	FOREIGN KEY (`merged_into_id`) REFERENCES `ingestion_sources`(`id`) ON UPDATE no action ON DELETE set null
);
--> statement-breakpoint
CREATE INDEX `idx_merged_into` ON `ingestion_sources` (`merged_into_id`);--> statement-breakpoint
CREATE TABLE `sessions` (
	`id` text PRIMARY KEY NOT NULL,
	`user_id` text NOT NULL,
	`expires_at` integer NOT NULL,
	FOREIGN KEY (`user_id`) REFERENCES `users`(`id`) ON UPDATE no action ON DELETE cascade
);
--> statement-breakpoint
CREATE TABLE `users` (
	`id` text PRIMARY KEY NOT NULL,
	`email` text NOT NULL,
	`first_name` text,
	`last_name` text,
	`password` text,
	`provider` text DEFAULT 'local',
	`provider_id` text,
	`created_at` integer DEFAULT (unixepoch() * 1000) NOT NULL,
	`updated_at` integer DEFAULT (unixepoch() * 1000) NOT NULL
);
--> statement-breakpoint
CREATE UNIQUE INDEX `users_email_unique` ON `users` (`email`);--> statement-breakpoint
CREATE TABLE `system_settings` (
	`id` integer PRIMARY KEY AUTOINCREMENT NOT NULL,
	`config` text NOT NULL
);
--> statement-breakpoint
CREATE TABLE `api_keys` (
	`id` text PRIMARY KEY NOT NULL,
	`name` text NOT NULL,
	`user_id` text NOT NULL,
	`key` text NOT NULL,
	`key_hash` text NOT NULL,
	`expires_at` integer NOT NULL,
	`created_at` integer DEFAULT (unixepoch() * 1000) NOT NULL,
	`updated_at` integer DEFAULT (unixepoch() * 1000) NOT NULL,
	FOREIGN KEY (`user_id`) REFERENCES `users`(`id`) ON UPDATE no action ON DELETE cascade
);
--> statement-breakpoint
CREATE TABLE `sync_sessions` (
	`id` text PRIMARY KEY NOT NULL,
	`ingestion_source_id` text NOT NULL,
	`is_initial_import` integer DEFAULT false NOT NULL,
	`total_mailboxes` integer DEFAULT 0 NOT NULL,
	`completed_mailboxes` integer DEFAULT 0 NOT NULL,
	`failed_mailboxes` integer DEFAULT 0 NOT NULL,
	`error_messages` text DEFAULT '[]' NOT NULL,
	`created_at` integer DEFAULT (unixepoch() * 1000) NOT NULL,
	`last_activity_at` integer DEFAULT (unixepoch() * 1000) NOT NULL,
	FOREIGN KEY (`ingestion_source_id`) REFERENCES `ingestion_sources`(`id`) ON UPDATE no action ON DELETE cascade
);
--> statement-breakpoint
CREATE TABLE `journaling_sources` (
	`id` text PRIMARY KEY NOT NULL,
	`name` text NOT NULL,
	`allowed_ips` text NOT NULL,
	`require_tls` integer DEFAULT true NOT NULL,
	`smtp_username` text,
	`smtp_password_hash` text,
	`status` text DEFAULT 'active' NOT NULL,
	`ingestion_source_id` text NOT NULL,
	`routing_address` text NOT NULL,
	`total_received` integer DEFAULT 0 NOT NULL,
	`last_received_at` integer,
	`created_at` integer DEFAULT (unixepoch() * 1000) NOT NULL,
	`updated_at` integer DEFAULT (unixepoch() * 1000) NOT NULL,
	FOREIGN KEY (`ingestion_source_id`) REFERENCES `ingestion_sources`(`id`) ON UPDATE no action ON DELETE cascade
);
--> statement-breakpoint
CREATE TABLE `contacts` (
	`id` text PRIMARY KEY NOT NULL,
	`email` text NOT NULL,
	`display_name` text NOT NULL,
	`source` text,
	`created_at` integer DEFAULT (unixepoch() * 1000) NOT NULL,
	`updated_at` integer DEFAULT (unixepoch() * 1000) NOT NULL
);
--> statement-breakpoint
CREATE UNIQUE INDEX `contacts_email_idx` ON `contacts` (`email`);--> statement-breakpoint
CREATE TABLE `jobs` (
	`id` text PRIMARY KEY NOT NULL,
	`queue` text NOT NULL,
	`name` text NOT NULL,
	`payload` text NOT NULL,
	`state` text DEFAULT 'pending' NOT NULL,
	`attempts` integer DEFAULT 0 NOT NULL,
	`max_attempts` integer DEFAULT 5 NOT NULL,
	`run_at` integer DEFAULT (unixepoch() * 1000) NOT NULL,
	`singleton_key` text,
	`error` text,
	`created_at` integer DEFAULT (unixepoch() * 1000) NOT NULL,
	`started_at` integer,
	`finished_at` integer
);
--> statement-breakpoint
CREATE INDEX `jobs_claim_idx` ON `jobs` (`queue`,`state`,`run_at`);--> statement-breakpoint
CREATE INDEX `jobs_singleton_idx` ON `jobs` (`queue`,`singleton_key`,`state`);--> statement-breakpoint
CREATE INDEX `jobs_created_idx` ON `jobs` (`created_at`);