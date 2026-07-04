ALTER TABLE `ingestion_sources` RENAME COLUMN `last_sync_started_at` TO `last_import_started_at`;--> statement-breakpoint
ALTER TABLE `ingestion_sources` RENAME COLUMN `last_sync_finished_at` TO `last_import_finished_at`;--> statement-breakpoint
ALTER TABLE `ingestion_sources` RENAME COLUMN `last_sync_status_message` TO `last_import_status_message`;--> statement-breakpoint
ALTER TABLE `sync_sessions` RENAME TO `import_sessions`;--> statement-breakpoint
UPDATE `ingestion_sources` SET `status` = 'pending' WHERE `status` = 'pending_auth';--> statement-breakpoint
UPDATE `ingestion_sources` SET `status` = 'ready' WHERE `status` = 'auth_success';--> statement-breakpoint
UPDATE `ingestion_sources` SET `status` = 'importing' WHERE `status` = 'syncing';