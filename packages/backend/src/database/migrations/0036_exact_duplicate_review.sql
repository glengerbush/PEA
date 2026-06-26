ALTER TABLE "archived_emails" ADD COLUMN "duplicate_of_email_id" uuid;
--> statement-breakpoint
ALTER TABLE "archived_emails" ADD COLUMN "duplicate_review_status" text DEFAULT 'unique' NOT NULL;
--> statement-breakpoint
ALTER TABLE "archived_emails" ADD COLUMN "is_duplicate_hidden" boolean DEFAULT false NOT NULL;
--> statement-breakpoint
ALTER TABLE "archived_emails" ADD CONSTRAINT "archived_emails_duplicate_of_email_id_archived_emails_id_fk" FOREIGN KEY ("duplicate_of_email_id") REFERENCES "public"."archived_emails"("id") ON DELETE set null ON UPDATE no action;
--> statement-breakpoint
CREATE INDEX "archived_emails_message_id_header_idx" ON "archived_emails" USING btree ("message_id_header");
--> statement-breakpoint
CREATE INDEX "archived_emails_storage_hash_idx" ON "archived_emails" USING btree ("storage_hash_sha256");
--> statement-breakpoint
CREATE INDEX "archived_emails_duplicate_of_idx" ON "archived_emails" USING btree ("duplicate_of_email_id");
--> statement-breakpoint
CREATE INDEX "archived_emails_duplicate_hidden_idx" ON "archived_emails" USING btree ("is_duplicate_hidden");
