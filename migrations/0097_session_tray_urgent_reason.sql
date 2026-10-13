-- 0097: why a gated command is URGENT (feedback #52, the user's desktop
-- tier): the reason the parking agent gave. NULL = an ordinary gate. An
-- urgent gate notifies the user at once even while bot-hq is focused, again
-- every ten minutes until answered (at most six times), and turns the
-- session's tile red; the card shows the reason. No phone push.
ALTER TABLE session_tray ADD COLUMN urgent_reason TEXT;
