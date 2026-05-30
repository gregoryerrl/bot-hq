"""Unit tests for the pure logic. Run: python -m unittest (no deps, no bot-hq)."""
import json
import unittest

from bothq_client import BotHqError, unwrap
from completion import evaluate, max_id, pick_choice_option


class TestUnwrap(unittest.TestCase):
    def test_double_encoded(self):
        env = {"result": {"content": [{"type": "text", "text": json.dumps({"session_id": "abc"})}], "isError": False}}
        self.assertEqual(unwrap(env), {"session_id": "abc"})

    def test_empty_text(self):
        env = {"result": {"content": [{"type": "text", "text": ""}], "isError": False}}
        self.assertEqual(unwrap(env), {})

    def test_plain_text_payload(self):
        env = {"result": {"content": [{"type": "text", "text": "hello"}], "isError": False}}
        self.assertEqual(unwrap(env), "hello")

    def test_malformed_raises(self):
        with self.assertRaises(BotHqError):
            unwrap({"result": {}})


class TestPickChoice(unittest.TestCase):
    def test_prefers_approve(self):
        self.assertEqual(pick_choice_option(["Deny", "Approve the push"]), "Approve the push")

    def test_prefers_continue(self):
        self.assertEqual(pick_choice_option(["Stop", "Continue working"]), "Continue working")

    def test_fallback_first(self):
        self.assertEqual(pick_choice_option(["Option A", "Option B"]), "Option A")

    def test_empty(self):
        self.assertEqual(pick_choice_option([]), "proceed")


class TestMaxId(unittest.TestCase):
    def test_out_of_order(self):
        self.assertEqual(max_id([{"id": 1}, {"id": 5}, {"id": 3}], 0), 5)

    def test_keeps_current(self):
        self.assertEqual(max_id([{"id": 2}], 9), 9)

    def test_ignores_non_int(self):
        self.assertEqual(max_id([{"id": None}, {"foo": 1}], 4), 4)


class TestEvaluate(unittest.TestCase):
    BASE = dict(silence_timeout=120, wall_clock_cap=600)

    def test_done_on_sentinel(self):
        d = evaluate(awaiting=True, pending_choice_count=0, seconds_since_last_msg=5, elapsed=30, **self.BASE)
        self.assertTrue(d.done)

    def test_gate_is_not_completion(self):
        # awaiting True but a pending choice exists => it's a gate, not done.
        d = evaluate(awaiting=True, pending_choice_count=1, seconds_since_last_msg=5, elapsed=30, **self.BASE)
        self.assertFalse(d.done)

    def test_done_on_silence(self):
        d = evaluate(awaiting=False, pending_choice_count=0, seconds_since_last_msg=200, elapsed=300, **self.BASE)
        self.assertTrue(d.done)

    def test_done_on_wall_clock(self):
        d = evaluate(awaiting=False, pending_choice_count=0, seconds_since_last_msg=5, elapsed=700, **self.BASE)
        self.assertTrue(d.done)

    def test_in_progress(self):
        d = evaluate(awaiting=False, pending_choice_count=0, seconds_since_last_msg=5, elapsed=30, **self.BASE)
        self.assertFalse(d.done)


if __name__ == "__main__":
    unittest.main()
