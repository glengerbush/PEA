CREATE TABLE "archive_folders" (
	"id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
	"parent_id" uuid,
	"name" text NOT NULL,
	"path" text NOT NULL,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL,
	CONSTRAINT "archive_folders_path_unique" UNIQUE("path")
);
--> statement-breakpoint
ALTER TABLE "archive_folders" ADD CONSTRAINT "archive_folders_parent_id_archive_folders_id_fk" FOREIGN KEY ("parent_id") REFERENCES "public"."archive_folders"("id") ON DELETE set null ON UPDATE no action;
--> statement-breakpoint
ALTER TABLE "archived_emails" ADD COLUMN "source_path" text;
--> statement-breakpoint
ALTER TABLE "archived_emails" ADD COLUMN "source_labels" jsonb;
--> statement-breakpoint
ALTER TABLE "archived_emails" ADD COLUMN "local_folder_id" uuid;
--> statement-breakpoint
ALTER TABLE "archived_emails" ADD COLUMN "local_folder_path" text;
--> statement-breakpoint
ALTER TABLE "archived_emails" ADD CONSTRAINT "archived_emails_local_folder_id_archive_folders_id_fk" FOREIGN KEY ("local_folder_id") REFERENCES "public"."archive_folders"("id") ON DELETE set null ON UPDATE no action;
--> statement-breakpoint
CREATE INDEX "archive_folders_parent_idx" ON "archive_folders" USING btree ("parent_id");
--> statement-breakpoint
CREATE INDEX "archived_emails_source_path_idx" ON "archived_emails" USING btree ("source_path");
--> statement-breakpoint
CREATE INDEX "archived_emails_local_folder_idx" ON "archived_emails" USING btree ("local_folder_id");
