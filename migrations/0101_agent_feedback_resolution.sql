-- How a feedback row was resolved, in words: the commit that fixed it, or the
-- decision that declined it and whose it was. Written by the `resolve_feedback`
-- MCP tool from a session on the bot-hq project, and shown by the Feedback
-- panel under the row. A row the panel's status buttons moved carries none;
-- reopening a row clears both columns.
ALTER TABLE agent_feedback ADD COLUMN resolution TEXT;
-- Who resolved it: `<participant slug> · <session id>`.
ALTER TABLE agent_feedback ADD COLUMN resolved_by TEXT;
