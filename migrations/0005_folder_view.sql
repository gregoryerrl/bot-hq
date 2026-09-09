-- Folder view + project registration. Two layered concepts:
--
-- 1. cl_path on projects: lets a registered project live at an arbitrary on-
--    disk location instead of the default <data_dir>/projects/<name>/. NULL
--    means use the default convention.
--
-- 2. cl_folders: per-folder descriptions (parallel to cl_index.file_path but
--    for directories). folder_path is relative to the project's CL root;
--    folder_path='' is the project root itself (project-level description).

ALTER TABLE projects ADD COLUMN cl_path TEXT;

CREATE TABLE cl_folders (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id  TEXT NOT NULL REFERENCES projects(name) ON DELETE CASCADE,
    folder_path TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    tags        TEXT,
    created_at  TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at  TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(project_id, folder_path)
);

CREATE INDEX idx_cl_folders_project ON cl_folders(project_id);
