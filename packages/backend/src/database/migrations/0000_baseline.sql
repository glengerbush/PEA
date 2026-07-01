CREATE TYPE "public"."ingestion_provider" AS ENUM('google_workspace', 'microsoft_365', 'generic_imap', 'pst_import', 'eml_import', 'mbox_import', 'smtp_journaling');--> statement-breakpoint
CREATE TYPE "public"."ingestion_status" AS ENUM('active', 'paused', 'error', 'pending_auth', 'syncing', 'importing', 'auth_success', 'imported', 'partially_active');--> statement-breakpoint
CREATE TYPE "public"."journaling_source_status" AS ENUM('active', 'paused');--> statement-breakpoint
CREATE TABLE "archived_emails" (
	"id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
	"thread_id" text,
	"ingestion_source_id" uuid NOT NULL,
	"user_email" text NOT NULL,
	"message_id_header" text,
	"provider_message_id" text,
	"sent_at" timestamp with time zone NOT NULL,
	"subject" text,
	"sender_name" text,
	"sender_email" text NOT NULL,
	"recipients" jsonb,
	"storage_path" text NOT NULL,
	"storage_hash_sha256" text NOT NULL,
	"size_bytes" bigint NOT NULL,
	"is_indexed" boolean DEFAULT false NOT NULL,
	"has_attachments" boolean DEFAULT false NOT NULL,
	"archived_at" timestamp with time zone DEFAULT now() NOT NULL,
	"source_path" text,
	"source_labels" jsonb,
	"duplicate_subject_hash" text,
	"duplicate_fuzzy_group_key" text,
	"duplicate_body_hash" text,
	"duplicate_recipient_fingerprint" text,
	"duplicate_attachment_fingerprint" text,
	"remote_content_status" text DEFAULT 'not_started' NOT NULL,
	"remote_content_asset_count" bigint DEFAULT 0 NOT NULL,
	"remote_content_archived_at" timestamp with time zone,
	"path" text,
	"tags" jsonb
);
--> statement-breakpoint
CREATE TABLE "attachments" (
	"id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
	"filename" text NOT NULL,
	"mime_type" text,
	"size_bytes" bigint NOT NULL,
	"content_hash_sha256" text NOT NULL,
	"storage_path" text NOT NULL,
	"ingestion_source_id" uuid
);
--> statement-breakpoint
CREATE TABLE "email_attachments" (
	"email_id" uuid NOT NULL,
	"attachment_id" uuid NOT NULL,
	CONSTRAINT "email_attachments_email_id_attachment_id_pk" PRIMARY KEY("email_id","attachment_id")
);
--> statement-breakpoint
CREATE TABLE "fuzzy_duplicate_group_emails" (
	"group_id" uuid NOT NULL,
	"email_id" uuid NOT NULL,
	"suggested_keeper" boolean DEFAULT false NOT NULL,
	CONSTRAINT "fuzzy_duplicate_group_emails_group_id_email_id_pk" PRIMARY KEY("group_id","email_id")
);
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
CREATE TABLE "custodians" (
	"id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
	"email" text NOT NULL,
	"display_name" text,
	"source_type" "ingestion_provider" NOT NULL,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL,
	CONSTRAINT "custodians_email_unique" UNIQUE("email")
);
--> statement-breakpoint
CREATE TABLE "ingestion_sources" (
	"id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
	"user_id" uuid,
	"name" text NOT NULL,
	"provider" "ingestion_provider" NOT NULL,
	"credentials" text,
	"status" "ingestion_status" DEFAULT 'pending_auth' NOT NULL,
	"last_sync_started_at" timestamp with time zone,
	"last_sync_finished_at" timestamp with time zone,
	"last_sync_status_message" text,
	"sync_state" jsonb,
	"merged_into_id" uuid,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
CREATE TABLE "sessions" (
	"id" text PRIMARY KEY NOT NULL,
	"user_id" uuid NOT NULL,
	"expires_at" timestamp with time zone NOT NULL
);
--> statement-breakpoint
CREATE TABLE "users" (
	"id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
	"email" text NOT NULL,
	"first_name" text,
	"last_name" text,
	"password" text,
	"provider" text DEFAULT 'local',
	"provider_id" text,
	"created_at" timestamp DEFAULT now() NOT NULL,
	"updated_at" timestamp DEFAULT now() NOT NULL,
	CONSTRAINT "users_email_unique" UNIQUE("email")
);
--> statement-breakpoint
CREATE TABLE "system_settings" (
	"id" serial PRIMARY KEY NOT NULL,
	"config" jsonb NOT NULL
);
--> statement-breakpoint
CREATE TABLE "api_keys" (
	"id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
	"name" text NOT NULL,
	"user_id" uuid NOT NULL,
	"key" text NOT NULL,
	"key_hash" text NOT NULL,
	"expires_at" timestamp with time zone NOT NULL,
	"created_at" timestamp DEFAULT now() NOT NULL,
	"updated_at" timestamp DEFAULT now() NOT NULL
);
--> statement-breakpoint
CREATE TABLE "sync_sessions" (
	"id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
	"ingestion_source_id" uuid NOT NULL,
	"is_initial_import" boolean DEFAULT false NOT NULL,
	"total_mailboxes" integer DEFAULT 0 NOT NULL,
	"completed_mailboxes" integer DEFAULT 0 NOT NULL,
	"failed_mailboxes" integer DEFAULT 0 NOT NULL,
	"error_messages" text[] DEFAULT '{}' NOT NULL,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"last_activity_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
CREATE TABLE "journaling_sources" (
	"id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
	"name" text NOT NULL,
	"allowed_ips" jsonb NOT NULL,
	"require_tls" boolean DEFAULT true NOT NULL,
	"smtp_username" text,
	"smtp_password_hash" text,
	"status" "journaling_source_status" DEFAULT 'active' NOT NULL,
	"ingestion_source_id" uuid NOT NULL,
	"routing_address" text NOT NULL,
	"total_received" integer DEFAULT 0 NOT NULL,
	"last_received_at" timestamp with time zone,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
CREATE TABLE "contacts" (
	"id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
	"email" text NOT NULL,
	"display_name" text NOT NULL,
	"source" text,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
ALTER TABLE "archived_emails" ADD CONSTRAINT "archived_emails_ingestion_source_id_ingestion_sources_id_fk" FOREIGN KEY ("ingestion_source_id") REFERENCES "public"."ingestion_sources"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "attachments" ADD CONSTRAINT "attachments_ingestion_source_id_ingestion_sources_id_fk" FOREIGN KEY ("ingestion_source_id") REFERENCES "public"."ingestion_sources"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "email_attachments" ADD CONSTRAINT "email_attachments_email_id_archived_emails_id_fk" FOREIGN KEY ("email_id") REFERENCES "public"."archived_emails"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "email_attachments" ADD CONSTRAINT "email_attachments_attachment_id_attachments_id_fk" FOREIGN KEY ("attachment_id") REFERENCES "public"."attachments"("id") ON DELETE restrict ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "fuzzy_duplicate_group_emails" ADD CONSTRAINT "fuzzy_duplicate_group_emails_group_id_fuzzy_duplicate_groups_id_fk" FOREIGN KEY ("group_id") REFERENCES "public"."fuzzy_duplicate_groups"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "fuzzy_duplicate_group_emails" ADD CONSTRAINT "fuzzy_duplicate_group_emails_email_id_archived_emails_id_fk" FOREIGN KEY ("email_id") REFERENCES "public"."archived_emails"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "remote_content_assets" ADD CONSTRAINT "remote_content_assets_email_id_archived_emails_id_fk" FOREIGN KEY ("email_id") REFERENCES "public"."archived_emails"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "ingestion_sources" ADD CONSTRAINT "ingestion_sources_user_id_users_id_fk" FOREIGN KEY ("user_id") REFERENCES "public"."users"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "ingestion_sources" ADD CONSTRAINT "ingestion_sources_merged_into_id_ingestion_sources_id_fk" FOREIGN KEY ("merged_into_id") REFERENCES "public"."ingestion_sources"("id") ON DELETE set null ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "sessions" ADD CONSTRAINT "sessions_user_id_users_id_fk" FOREIGN KEY ("user_id") REFERENCES "public"."users"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "api_keys" ADD CONSTRAINT "api_keys_user_id_users_id_fk" FOREIGN KEY ("user_id") REFERENCES "public"."users"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "sync_sessions" ADD CONSTRAINT "sync_sessions_ingestion_source_id_ingestion_sources_id_fk" FOREIGN KEY ("ingestion_source_id") REFERENCES "public"."ingestion_sources"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "journaling_sources" ADD CONSTRAINT "journaling_sources_ingestion_source_id_ingestion_sources_id_fk" FOREIGN KEY ("ingestion_source_id") REFERENCES "public"."ingestion_sources"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
CREATE INDEX "thread_id_idx" ON "archived_emails" USING btree ("thread_id");--> statement-breakpoint
CREATE INDEX "archived_emails_message_id_header_idx" ON "archived_emails" USING btree ("message_id_header");--> statement-breakpoint
CREATE INDEX "archived_emails_storage_hash_idx" ON "archived_emails" USING btree ("storage_hash_sha256");--> statement-breakpoint
CREATE INDEX "provider_msg_source_idx" ON "archived_emails" USING btree ("provider_message_id","ingestion_source_id");--> statement-breakpoint
CREATE INDEX "archived_emails_source_path_idx" ON "archived_emails" USING btree ("source_path");--> statement-breakpoint
CREATE INDEX "archived_emails_fuzzy_subject_sender_idx" ON "archived_emails" USING btree ("duplicate_subject_hash","sender_email");--> statement-breakpoint
CREATE INDEX "archived_emails_fuzzy_group_key_idx" ON "archived_emails" USING btree ("duplicate_fuzzy_group_key");--> statement-breakpoint
CREATE INDEX "archived_emails_fuzzy_body_idx" ON "archived_emails" USING btree ("duplicate_body_hash");--> statement-breakpoint
CREATE INDEX "archived_emails_fuzzy_recipients_idx" ON "archived_emails" USING btree ("duplicate_recipient_fingerprint");--> statement-breakpoint
CREATE INDEX "archived_emails_fuzzy_attachments_idx" ON "archived_emails" USING btree ("duplicate_attachment_fingerprint");--> statement-breakpoint
CREATE INDEX "archived_emails_remote_content_status_idx" ON "archived_emails" USING btree ("remote_content_status");--> statement-breakpoint
CREATE INDEX "source_hash_idx" ON "attachments" USING btree ("ingestion_source_id","content_hash_sha256");--> statement-breakpoint
CREATE INDEX "fuzzy_duplicate_group_emails_email_idx" ON "fuzzy_duplicate_group_emails" USING btree ("email_id");--> statement-breakpoint
CREATE INDEX "fuzzy_duplicate_groups_status_idx" ON "fuzzy_duplicate_groups" USING btree ("status");--> statement-breakpoint
CREATE INDEX "fuzzy_duplicate_groups_score_idx" ON "fuzzy_duplicate_groups" USING btree ("score");--> statement-breakpoint
CREATE INDEX "remote_content_assets_email_idx" ON "remote_content_assets" USING btree ("email_id");--> statement-breakpoint
CREATE INDEX "remote_content_assets_status_idx" ON "remote_content_assets" USING btree ("status");--> statement-breakpoint
CREATE INDEX "idx_merged_into" ON "ingestion_sources" USING btree ("merged_into_id");--> statement-breakpoint
CREATE UNIQUE INDEX "contacts_email_idx" ON "contacts" USING btree ("email");