ALTER TABLE `attachments` ADD `extracted_text` text;--> statement-breakpoint
CREATE INDEX IF NOT EXISTS `archived_emails_source_sent_idx` ON `archived_emails` (`ingestion_source_id`,`sent_at`);--> statement-breakpoint
ALTER TABLE `archived_emails` DROP COLUMN `is_indexed`;--> statement-breakpoint
ALTER TABLE `archived_emails` DROP COLUMN `source_labels`;--> statement-breakpoint
ALTER TABLE `archived_emails` DROP COLUMN `path`;--> statement-breakpoint
ALTER TABLE `ingestion_sources` DROP COLUMN `sync_state`;--> statement-breakpoint
ALTER TABLE `users` DROP COLUMN `password`;--> statement-breakpoint
ALTER TABLE `users` DROP COLUMN `provider`;--> statement-breakpoint
ALTER TABLE `users` DROP COLUMN `provider_id`;--> statement-breakpoint
DROP TABLE `journaling_sources`;
