ALTER TABLE `import_sessions` ADD `total_bytes` integer DEFAULT 0 NOT NULL;--> statement-breakpoint
ALTER TABLE `import_sessions` ADD `processed_bytes` integer DEFAULT 0 NOT NULL;
