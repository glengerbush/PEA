ALTER TABLE `archived_emails` ADD `deleted_at` integer;--> statement-breakpoint
CREATE INDEX `archived_emails_deleted_at_idx` ON `archived_emails` (`deleted_at`);
