CREATE TABLE `ingestion_sources_new` (
	`id` text PRIMARY KEY NOT NULL,
	`name` text NOT NULL,
	`provider` text NOT NULL,
	`credentials` text,
	`status` text DEFAULT 'pending_auth' NOT NULL,
	`last_sync_started_at` integer,
	`last_sync_finished_at` integer,
	`last_sync_status_message` text,
	`merged_into_id` text,
	`created_at` integer DEFAULT (unixepoch() * 1000) NOT NULL,
	`updated_at` integer DEFAULT (unixepoch() * 1000) NOT NULL,
	FOREIGN KEY (`merged_into_id`) REFERENCES `ingestion_sources`(`id`) ON UPDATE no action ON DELETE set null
);--> statement-breakpoint
INSERT INTO `ingestion_sources_new` (`id`, `name`, `provider`, `credentials`, `status`, `last_sync_started_at`, `last_sync_finished_at`, `last_sync_status_message`, `merged_into_id`, `created_at`, `updated_at`) SELECT `id`, `name`, `provider`, `credentials`, `status`, `last_sync_started_at`, `last_sync_finished_at`, `last_sync_status_message`, `merged_into_id`, `created_at`, `updated_at` FROM `ingestion_sources`;--> statement-breakpoint
DROP TABLE `ingestion_sources`;--> statement-breakpoint
ALTER TABLE `ingestion_sources_new` RENAME TO `ingestion_sources`;--> statement-breakpoint
CREATE INDEX `idx_merged_into` ON `ingestion_sources` (`merged_into_id`);--> statement-breakpoint
DROP TABLE `users`;
