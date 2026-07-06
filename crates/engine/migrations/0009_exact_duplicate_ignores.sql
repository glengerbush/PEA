CREATE TABLE `exact_duplicate_ignores` (
	`fingerprint` text PRIMARY KEY NOT NULL,
	`created_at` integer DEFAULT (unixepoch() * 1000) NOT NULL
);--> statement-breakpoint
DROP TABLE IF EXISTS `likely_duplicate_ignores`;
