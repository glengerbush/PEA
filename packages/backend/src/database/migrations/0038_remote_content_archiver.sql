ALTER TABLE "archived_emails" ADD COLUMN "remote_content_status" text DEFAULT 'not_started' NOT NULL;
--> statement-breakpoint
ALTER TABLE "archived_emails" ADD COLUMN "remote_content_asset_count" bigint DEFAULT 0 NOT NULL;
--> statement-breakpoint
ALTER TABLE "archived_emails" ADD COLUMN "remote_content_archived_at" timestamp with time zone;
--> statement-breakpoint
CREATE TABLE "remote_content_assets" (
	"id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
	"email_id" uuid NOT NULL,
	"original_url" text NOT NULL,
	"final_url" text,
	"url_hash" text NOT NULL,
	"status" text DEFAULT 'pending' NOT NULL,
	"content_type" text,
	"size_bytes" bigint,
	"content_hash_sha256" text,
	"storage_path" text,
	"failure_reason" text,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL,
	CONSTRAINT "remote_content_assets_email_url_hash_unique" UNIQUE("email_id","url_hash")
);
--> statement-breakpoint
ALTER TABLE "remote_content_assets" ADD CONSTRAINT "remote_content_assets_email_id_archived_emails_id_fk" FOREIGN KEY ("email_id") REFERENCES "public"."archived_emails"("id") ON DELETE cascade ON UPDATE no action;
--> statement-breakpoint
CREATE INDEX "archived_emails_remote_content_status_idx" ON "archived_emails" USING btree ("remote_content_status");
--> statement-breakpoint
CREATE INDEX "remote_content_assets_email_idx" ON "remote_content_assets" USING btree ("email_id");
--> statement-breakpoint
CREATE INDEX "remote_content_assets_status_idx" ON "remote_content_assets" USING btree ("status");
