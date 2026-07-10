DELETE FROM `email_attachments` WHERE `rowid` NOT IN (SELECT MIN(`rowid`) FROM `email_attachments` GROUP BY `email_id`, `attachment_id`);--> statement-breakpoint
CREATE UNIQUE INDEX `email_attachments_email_attachment_unique` ON `email_attachments` (`email_id`,`attachment_id`);
