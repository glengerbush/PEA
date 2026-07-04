ALTER TABLE `remote_content_assets` DROP COLUMN `content_hash_sha256`;--> statement-breakpoint
ALTER TABLE `remote_content_assets` DROP COLUMN `created_at`;--> statement-breakpoint
ALTER TABLE `remote_content_assets` DROP COLUMN `updated_at`;--> statement-breakpoint
ALTER TABLE `contacts` DROP COLUMN `source`;--> statement-breakpoint
ALTER TABLE `contacts` DROP COLUMN `created_at`;--> statement-breakpoint
ALTER TABLE `contacts` DROP COLUMN `updated_at`;--> statement-breakpoint
ALTER TABLE `users` DROP COLUMN `updated_at`;
