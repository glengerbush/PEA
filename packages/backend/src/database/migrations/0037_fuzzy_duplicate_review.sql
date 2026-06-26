ALTER TABLE "archived_emails" ADD COLUMN "duplicate_subject_hash" text;
--> statement-breakpoint
ALTER TABLE "archived_emails" ADD COLUMN "duplicate_fuzzy_group_key" text;
--> statement-breakpoint
ALTER TABLE "archived_emails" ADD COLUMN "duplicate_body_hash" text;
--> statement-breakpoint
ALTER TABLE "archived_emails" ADD COLUMN "duplicate_recipient_fingerprint" text;
--> statement-breakpoint
ALTER TABLE "archived_emails" ADD COLUMN "duplicate_attachment_fingerprint" text;
--> statement-breakpoint
CREATE TABLE "fuzzy_duplicate_groups" (
	"id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
	"group_key" text NOT NULL,
	"status" text DEFAULT 'pending' NOT NULL,
	"score" integer NOT NULL,
	"signals" jsonb,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL,
	CONSTRAINT "fuzzy_duplicate_groups_group_key_unique" UNIQUE("group_key")
);
--> statement-breakpoint
CREATE TABLE "fuzzy_duplicate_group_emails" (
	"group_id" uuid NOT NULL,
	"email_id" uuid NOT NULL,
	"suggested_keeper" boolean DEFAULT false NOT NULL,
	CONSTRAINT "fuzzy_duplicate_group_emails_group_id_email_id_pk" PRIMARY KEY("group_id","email_id")
);
--> statement-breakpoint
ALTER TABLE "fuzzy_duplicate_group_emails" ADD CONSTRAINT "fuzzy_duplicate_group_emails_group_id_fuzzy_duplicate_groups_id_fk" FOREIGN KEY ("group_id") REFERENCES "public"."fuzzy_duplicate_groups"("id") ON DELETE cascade ON UPDATE no action;
--> statement-breakpoint
ALTER TABLE "fuzzy_duplicate_group_emails" ADD CONSTRAINT "fuzzy_duplicate_group_emails_email_id_archived_emails_id_fk" FOREIGN KEY ("email_id") REFERENCES "public"."archived_emails"("id") ON DELETE cascade ON UPDATE no action;
--> statement-breakpoint
CREATE INDEX "archived_emails_fuzzy_subject_sender_idx" ON "archived_emails" USING btree ("duplicate_subject_hash","sender_email");
--> statement-breakpoint
CREATE INDEX "archived_emails_fuzzy_group_key_idx" ON "archived_emails" USING btree ("is_duplicate_hidden","duplicate_fuzzy_group_key");
--> statement-breakpoint
CREATE INDEX "archived_emails_fuzzy_body_idx" ON "archived_emails" USING btree ("duplicate_body_hash");
--> statement-breakpoint
CREATE INDEX "archived_emails_fuzzy_recipients_idx" ON "archived_emails" USING btree ("duplicate_recipient_fingerprint");
--> statement-breakpoint
CREATE INDEX "archived_emails_fuzzy_attachments_idx" ON "archived_emails" USING btree ("duplicate_attachment_fingerprint");
--> statement-breakpoint
CREATE INDEX "fuzzy_duplicate_groups_status_idx" ON "fuzzy_duplicate_groups" USING btree ("status");
--> statement-breakpoint
CREATE INDEX "fuzzy_duplicate_groups_score_idx" ON "fuzzy_duplicate_groups" USING btree ("score");
--> statement-breakpoint
CREATE INDEX "fuzzy_duplicate_group_emails_email_idx" ON "fuzzy_duplicate_group_emails" USING btree ("email_id");
