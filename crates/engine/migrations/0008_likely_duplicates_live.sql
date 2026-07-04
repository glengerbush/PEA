DROP TABLE IF EXISTS `fuzzy_duplicate_group_emails`;--> statement-breakpoint
CREATE TABLE `likely_duplicate_ignores` (
	`group_key` text PRIMARY KEY NOT NULL,
	`created_at` integer DEFAULT (unixepoch() * 1000) NOT NULL
);--> statement-breakpoint
INSERT OR IGNORE INTO `likely_duplicate_ignores` (`group_key`, `created_at`) SELECT `group_key`, `created_at` FROM `fuzzy_duplicate_groups` WHERE `status` = 'ignored';--> statement-breakpoint
DROP TABLE `fuzzy_duplicate_groups`;--> statement-breakpoint
ALTER TABLE `archived_emails` RENAME COLUMN `duplicate_fuzzy_group_key` TO `duplicate_likely_group_key`;--> statement-breakpoint
DROP INDEX `archived_emails_fuzzy_group_key_idx`;--> statement-breakpoint
CREATE INDEX `archived_emails_likely_group_key_idx` ON `archived_emails` (`duplicate_likely_group_key`);--> statement-breakpoint
DROP INDEX `archived_emails_fuzzy_subject_sender_idx`;--> statement-breakpoint
CREATE INDEX `archived_emails_likely_subject_sender_idx` ON `archived_emails` (`duplicate_subject_hash`,`sender_email`);--> statement-breakpoint
DROP INDEX `archived_emails_fuzzy_body_idx`;--> statement-breakpoint
CREATE INDEX `archived_emails_likely_body_idx` ON `archived_emails` (`duplicate_body_hash`);