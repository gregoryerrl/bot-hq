CREATE TABLE `authorized_devices` (
	`id` integer PRIMARY KEY AUTOINCREMENT NOT NULL,
	`device_name` text NOT NULL,
	`device_fingerprint` text NOT NULL,
	`token_hash` text NOT NULL,
	`authorized_at` integer NOT NULL,
	`last_seen_at` integer,
	`is_revoked` integer DEFAULT false NOT NULL
);
--> statement-breakpoint
CREATE UNIQUE INDEX `authorized_devices_device_fingerprint_unique` ON `authorized_devices` (`device_fingerprint`);--> statement-breakpoint
CREATE TABLE `logs` (
	`id` integer PRIMARY KEY AUTOINCREMENT NOT NULL,
	`workspace_id` integer,
	`task_id` integer,
	`type` text NOT NULL,
	`message` text NOT NULL,
	`details` text,
	`created_at` integer NOT NULL,
	FOREIGN KEY (`workspace_id`) REFERENCES `workspaces`(`id`) ON UPDATE no action ON DELETE no action,
	FOREIGN KEY (`task_id`) REFERENCES `tasks`(`id`) ON UPDATE no action ON DELETE no action
);
--> statement-breakpoint
CREATE INDEX `logs_type_idx` ON `logs` (`type`);--> statement-breakpoint
CREATE INDEX `logs_task_idx` ON `logs` (`task_id`);--> statement-breakpoint
CREATE INDEX `logs_created_idx` ON `logs` (`created_at`);--> statement-breakpoint
CREATE INDEX `logs_stream_idx` ON `logs` (`id`,`type`);--> statement-breakpoint
CREATE TABLE `pending_devices` (
	`id` integer PRIMARY KEY AUTOINCREMENT NOT NULL,
	`pairing_code` text NOT NULL,
	`device_info` text NOT NULL,
	`created_at` integer NOT NULL,
	`expires_at` integer NOT NULL
);
--> statement-breakpoint
CREATE UNIQUE INDEX `pending_devices_pairing_code_unique` ON `pending_devices` (`pairing_code`);--> statement-breakpoint
CREATE TABLE `plugin_store` (
	`id` integer PRIMARY KEY AUTOINCREMENT NOT NULL,
	`plugin_id` integer NOT NULL,
	`key` text NOT NULL,
	`value` text,
	`created_at` integer NOT NULL,
	`updated_at` integer NOT NULL,
	FOREIGN KEY (`plugin_id`) REFERENCES `plugins`(`id`) ON UPDATE no action ON DELETE cascade
);
--> statement-breakpoint
CREATE INDEX `plugin_store_plugin_idx` ON `plugin_store` (`plugin_id`);--> statement-breakpoint
CREATE INDEX `plugin_store_key_idx` ON `plugin_store` (`plugin_id`,`key`);--> statement-breakpoint
CREATE TABLE `plugin_task_data` (
	`id` integer PRIMARY KEY AUTOINCREMENT NOT NULL,
	`plugin_id` integer NOT NULL,
	`task_id` integer NOT NULL,
	`data` text DEFAULT '{}' NOT NULL,
	`created_at` integer NOT NULL,
	`updated_at` integer NOT NULL,
	FOREIGN KEY (`plugin_id`) REFERENCES `plugins`(`id`) ON UPDATE no action ON DELETE cascade,
	FOREIGN KEY (`task_id`) REFERENCES `tasks`(`id`) ON UPDATE no action ON DELETE cascade
);
--> statement-breakpoint
CREATE INDEX `plugin_task_plugin_idx` ON `plugin_task_data` (`plugin_id`);--> statement-breakpoint
CREATE INDEX `plugin_task_task_idx` ON `plugin_task_data` (`task_id`);--> statement-breakpoint
CREATE TABLE `plugin_workspace_data` (
	`id` integer PRIMARY KEY AUTOINCREMENT NOT NULL,
	`plugin_id` integer NOT NULL,
	`workspace_id` integer NOT NULL,
	`data` text DEFAULT '{}' NOT NULL,
	`created_at` integer NOT NULL,
	`updated_at` integer NOT NULL,
	FOREIGN KEY (`plugin_id`) REFERENCES `plugins`(`id`) ON UPDATE no action ON DELETE cascade,
	FOREIGN KEY (`workspace_id`) REFERENCES `workspaces`(`id`) ON UPDATE no action ON DELETE cascade
);
--> statement-breakpoint
CREATE INDEX `plugin_workspace_plugin_idx` ON `plugin_workspace_data` (`plugin_id`);--> statement-breakpoint
CREATE INDEX `plugin_workspace_workspace_idx` ON `plugin_workspace_data` (`workspace_id`);--> statement-breakpoint
CREATE TABLE `plugins` (
	`id` integer PRIMARY KEY AUTOINCREMENT NOT NULL,
	`name` text NOT NULL,
	`version` text NOT NULL,
	`enabled` integer DEFAULT true NOT NULL,
	`manifest` text NOT NULL,
	`settings` text DEFAULT '{}' NOT NULL,
	`credentials` text,
	`installed_at` integer NOT NULL,
	`updated_at` integer NOT NULL
);
--> statement-breakpoint
CREATE UNIQUE INDEX `plugins_name_unique` ON `plugins` (`name`);--> statement-breakpoint
CREATE TABLE `settings` (
	`key` text PRIMARY KEY NOT NULL,
	`value` text NOT NULL,
	`updated_at` integer NOT NULL
);
--> statement-breakpoint
CREATE TABLE `tasks` (
	`id` integer PRIMARY KEY AUTOINCREMENT NOT NULL,
	`workspace_id` integer NOT NULL,
	`source_plugin_id` integer,
	`source_ref` text,
	`title` text NOT NULL,
	`description` text,
	`state` text DEFAULT 'new' NOT NULL,
	`priority` integer DEFAULT 0,
	`agent_plan` text,
	`branch_name` text,
	`completion_criteria` text,
	`iteration_count` integer DEFAULT 0,
	`max_iterations` integer,
	`feedback` text,
	`assigned_at` integer,
	`updated_at` integer NOT NULL,
	FOREIGN KEY (`workspace_id`) REFERENCES `workspaces`(`id`) ON UPDATE no action ON DELETE no action
);
--> statement-breakpoint
CREATE INDEX `tasks_workspace_idx` ON `tasks` (`workspace_id`);--> statement-breakpoint
CREATE INDEX `tasks_state_idx` ON `tasks` (`state`);--> statement-breakpoint
CREATE TABLE `workspaces` (
	`id` integer PRIMARY KEY AUTOINCREMENT NOT NULL,
	`name` text NOT NULL,
	`repo_path` text NOT NULL,
	`linked_dirs` text,
	`build_command` text,
	`agent_config` text,
	`created_at` integer NOT NULL
);
--> statement-breakpoint
CREATE UNIQUE INDEX `workspaces_name_unique` ON `workspaces` (`name`);