import { sqliteTable, text, integer, index, real } from "drizzle-orm/sqlite-core";

// Projects (replaces workspaces)
export const projects = sqliteTable("projects", {
  id: integer("id").primaryKey({ autoIncrement: true }),
  name: text("name").notNull().unique(),
  description: text("description"),
  repoPath: text("repo_path"),
  status: text("status", { enum: ["active", "archived"] }).notNull().default("active"),
  notes: text("notes"),
  createdAt: integer("created_at", { mode: "timestamp" }).notNull().$defaultFn(() => new Date()),
  updatedAt: integer("updated_at", { mode: "timestamp" }).notNull().$defaultFn(() => new Date()),
});

// Tasks
export const tasks = sqliteTable("tasks", {
  id: integer("id").primaryKey({ autoIncrement: true }),
  projectId: integer("project_id").notNull().references(() => projects.id, { onDelete: "cascade" }),
  parentTaskId: integer("parent_task_id"),  // self-reference for subtasks (no .references() to avoid circular ref)
  title: text("title").notNull(),
  description: text("description"),
  state: text("state", { enum: ["todo", "in_progress", "done", "blocked"] }).notNull().default("todo"),
  priority: integer("priority").default(0),
  tags: text("tags"),  // JSON array
  dueDate: integer("due_date", { mode: "timestamp" }),
  order: integer("order").default(0),
  createdAt: integer("created_at", { mode: "timestamp" }).notNull().$defaultFn(() => new Date()),
  updatedAt: integer("updated_at", { mode: "timestamp" }).notNull().$defaultFn(() => new Date()),
}, (table) => [
  index("tasks_project_idx").on(table.projectId),
  index("tasks_state_idx").on(table.state),
  index("tasks_parent_idx").on(table.parentTaskId),
  index("tasks_due_idx").on(table.dueDate),
]);

// Task Notes
export const taskNotes = sqliteTable("task_notes", {
  id: integer("id").primaryKey({ autoIncrement: true }),
  taskId: integer("task_id").notNull().references(() => tasks.id, { onDelete: "cascade" }),
  content: text("content").notNull(),
  createdAt: integer("created_at", { mode: "timestamp" }).notNull().$defaultFn(() => new Date()),
}, (table) => [
  index("task_notes_task_idx").on(table.taskId),
]);

// Task Dependencies
export const taskDependencies = sqliteTable("task_dependencies", {
  id: integer("id").primaryKey({ autoIncrement: true }),
  taskId: integer("task_id").notNull().references(() => tasks.id, { onDelete: "cascade" }),
  dependsOnTaskId: integer("depends_on_task_id").notNull().references(() => tasks.id, { onDelete: "cascade" }),
  createdAt: integer("created_at", { mode: "timestamp" }).notNull().$defaultFn(() => new Date()),
}, (table) => [
  index("task_deps_task_idx").on(table.taskId),
  index("task_deps_depends_idx").on(table.dependsOnTaskId),
]);

// Diagrams (simplified — no more flowData blob)
export const diagrams = sqliteTable("diagrams", {
  id: integer("id").primaryKey({ autoIncrement: true }),
  projectId: integer("project_id").notNull().references(() => projects.id, { onDelete: "cascade" }),
  title: text("title").notNull(),
  template: text("template"),
  createdAt: integer("created_at", { mode: "timestamp" }).notNull().$defaultFn(() => new Date()),
  updatedAt: integer("updated_at", { mode: "timestamp" }).notNull().$defaultFn(() => new Date()),
}, (table) => [
  index("diagrams_project_idx").on(table.projectId),
]);

// Diagram Groups
export const diagramGroups = sqliteTable("diagram_groups", {
  id: integer("id").primaryKey({ autoIncrement: true }),
  diagramId: integer("diagram_id").notNull().references(() => diagrams.id, { onDelete: "cascade" }),
  label: text("label").notNull(),
  color: text("color").notNull().default("#6b7280"),
  createdAt: integer("created_at", { mode: "timestamp" }).notNull().$defaultFn(() => new Date()),
}, (table) => [
  index("diagram_groups_diagram_idx").on(table.diagramId),
]);

// Diagram Nodes
export const diagramNodes = sqliteTable("diagram_nodes", {
  id: integer("id").primaryKey({ autoIncrement: true }),
  diagramId: integer("diagram_id").notNull().references(() => diagrams.id, { onDelete: "cascade" }),
  groupId: integer("group_id").references(() => diagramGroups.id, { onDelete: "set null" }),
  nodeType: text("node_type").notNull().default("default"),
  label: text("label").notNull(),
  description: text("description"),
  metadata: text("metadata"),  // JSON
  positionX: real("position_x").notNull().default(0),
  positionY: real("position_y").notNull().default(0),
  createdAt: integer("created_at", { mode: "timestamp" }).notNull().$defaultFn(() => new Date()),
}, (table) => [
  index("diagram_nodes_diagram_idx").on(table.diagramId),
  index("diagram_nodes_group_idx").on(table.groupId),
]);

// Diagram Edges
export const diagramEdges = sqliteTable("diagram_edges", {
  id: integer("id").primaryKey({ autoIncrement: true }),
  diagramId: integer("diagram_id").notNull().references(() => diagrams.id, { onDelete: "cascade" }),
  sourceNodeId: integer("source_node_id").notNull().references(() => diagramNodes.id, { onDelete: "cascade" }),
  targetNodeId: integer("target_node_id").notNull().references(() => diagramNodes.id, { onDelete: "cascade" }),
  label: text("label"),
  createdAt: integer("created_at", { mode: "timestamp" }).notNull().$defaultFn(() => new Date()),
}, (table) => [
  index("diagram_edges_diagram_idx").on(table.diagramId),
]);

// Type exports
export type Project = typeof projects.$inferSelect;
export type NewProject = typeof projects.$inferInsert;
export type Task = typeof tasks.$inferSelect;
export type NewTask = typeof tasks.$inferInsert;
export type TaskNote = typeof taskNotes.$inferSelect;
export type NewTaskNote = typeof taskNotes.$inferInsert;
export type TaskDependency = typeof taskDependencies.$inferSelect;
export type NewTaskDependency = typeof taskDependencies.$inferInsert;
export type Diagram = typeof diagrams.$inferSelect;
export type NewDiagram = typeof diagrams.$inferInsert;
export type DiagramGroup = typeof diagramGroups.$inferSelect;
export type NewDiagramGroup = typeof diagramGroups.$inferInsert;
export type DiagramNode = typeof diagramNodes.$inferSelect;
export type NewDiagramNode = typeof diagramNodes.$inferInsert;
export type DiagramEdge = typeof diagramEdges.$inferSelect;
export type NewDiagramEdge = typeof diagramEdges.$inferInsert;
