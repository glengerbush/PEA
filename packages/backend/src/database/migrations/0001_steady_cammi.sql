ALTER TABLE "custodians" DISABLE ROW LEVEL SECURITY;--> statement-breakpoint
DROP TABLE "custodians" CASCADE;--> statement-breakpoint
ALTER TABLE "email_attachments" DROP CONSTRAINT "email_attachments_email_id_attachment_id_pk";--> statement-breakpoint
ALTER TABLE "email_attachments" ADD COLUMN "id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL;--> statement-breakpoint
CREATE INDEX "email_attachments_email_idx" ON "email_attachments" USING btree ("email_id");--> statement-breakpoint
CREATE INDEX "email_attachments_attachment_idx" ON "email_attachments" USING btree ("attachment_id");